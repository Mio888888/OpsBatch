//! Command-completion tracking for commands the app injects into a live shell (AI "approve & run", Docker panel).
//!
//! The injected text is `<command>` followed by a hidden tracker line that prints
//! `__OPSBATCH_AI_DONE_<id>__:<exit code>__`. The output stream is scanned for that marker (see `done_pattern`)
//! and the tracker artifacts are stripped from what the user sees (`TrackedCommandOutputCleaner`).

use std::sync::OnceLock;

use regex::Regex;

const TRACKER_STATUS_TOKEN: &str = "__opsbatch_ai_status=$?";
const TRACKER_DONE_PREFIX: &str = "__OPSBATCH_AI_DONE_";
const PARTIAL_TRACKER_PREFIXES: [&str; 2] = [TRACKER_STATUS_TOKEN, TRACKER_DONE_PREFIX];

/// What the tracker prints before its completion marker (an OSC sequence no terminal shows): "if the output so far
/// does not end with a line break, start a new line". The shell cannot know the cursor column, the cleaner knows
/// whether the text it passed on ended a line, and a command whose output has no final newline keeps its last line.
pub const FRESH_LINE_MARKER: &str = "\x1b]7777;opsbatch-fresh-line\x07";
/// The same sequence as the shell's `printf` writes it.
const FRESH_LINE_PRINTF: &str = "\\033]7777;opsbatch-fresh-line\\007";

fn done_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"__OPSBATCH_AI_DONE_[a-f0-9]+__:-?\d+__").unwrap())
}

fn artifact_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?m)^.*(?:__opsbatch_ai_status=\$\?|printf ['"][^\r\n]*__OPSBATCH_AI_DONE_[a-f0-9]+__|__OPSBATCH_AI_DONE_[a-f0-9]+__:-?\d+__).*(?:\r?\n)?"#,
        )
        .unwrap()
    })
}

fn partial_done_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"__OPSBATCH_AI_DONE_[a-f0-9]{4,}").unwrap())
}

/// `__OPSBATCH_AI_DONE_` plus 0-3 hex digits at the very end of the text: a marker cut off inside its first hex
/// digits. (The TS original only held back an exact prefix or >=4 hex digits, leaking `__OPSBATCH_AI_DONE_a` when a
/// chunk boundary fell in that window.)
fn trailing_done_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"__OPSBATCH_AI_DONE_[a-f0-9]{0,3}\z").unwrap())
}

fn ansi_control_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:[\r\s]|\x1b\[[0-?]*[ -/]*[@-~])*$").unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedCommand {
    pub visible_command: String,
    pub hidden_tracker: String,
}

/// `done_prefix` is `__OPSBATCH_AI_DONE_<execution id>__:`; the shell prints `<prefix><exit code>__`.
pub fn create_tracked_command(command: &str, done_prefix: &str) -> TrackedCommand {
    TrackedCommand {
        visible_command: command.trim_end().to_string(),
        hidden_tracker: format!(
            "\n__opsbatch_ai_status=$?; printf '{FRESH_LINE_PRINTF}'; printf '{done_prefix}%s__\\n' \"$__opsbatch_ai_status\""
        ),
    }
}

/// Regex source matching the completion marker of one execution; capture group 1 is the exit code.
pub fn done_pattern(execution_id: &str) -> Regex {
    Regex::new(&format!(
        r"{}(-?\d+)__",
        regex::escape(&format!("{TRACKER_DONE_PREFIX}{execution_id}__:"))
    ))
    .unwrap()
}

pub fn strip_tracked_command_output_artifacts(output: &str) -> String {
    let without_lines = artifact_line_re().replace_all(output, "");
    done_marker_re()
        .replace_all(&without_lines, "")
        .into_owned()
}

fn find_tracker_token_index(text: &str, from: usize) -> Option<usize> {
    let status = text
        .get(from..)
        .and_then(|t| t.find(TRACKER_STATUS_TOKEN))
        .map(|i| i + from);
    let done = partial_done_re().find_at(text, from).map(|m| m.start());
    match (status, done) {
        (None, d) => d,
        (s, None) => s,
        (Some(s), Some(d)) => Some(s.min(d)),
    }
}

fn has_potential_partial_tracker(text: &str, index: usize) -> bool {
    let rest = &text[index..];
    PARTIAL_TRACKER_PREFIXES.iter().any(|p| p.starts_with(rest))
}

/// Longest suffix of `text` that is a proper prefix of a tracker token (the token may continue in the next chunk).
fn tracker_prefix_suffix_length(text: &str) -> usize {
    let mut longest = 0;
    for prefix in PARTIAL_TRACKER_PREFIXES {
        let max = (prefix.len() - 1).min(text.len());
        let mut len = max;
        while len > longest {
            let start = text.len() - len;
            if text.is_char_boundary(start) && prefix.starts_with(&text[start..]) {
                longest = len;
                break;
            }
            len -= 1;
        }
    }
    longest
}

/// Same semantics as the JS `lastIndexOf('\n', max(0, index-1)) + 1`.
fn line_start(text: &str, index: usize) -> usize {
    let from = index.saturating_sub(1);
    let end = (from + 1).min(text.len());
    let end = (0..=end)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    text[..end].rfind('\n').map(|p| p + 1).unwrap_or(0)
}

fn pending_start_for_partial_tracker(text: &str, suffix_length: usize) -> usize {
    let suffix_start = text.len() - suffix_length;
    let ls = line_start(text, suffix_start);
    let before = &text[ls..suffix_start];
    if ansi_control_prefix_re().is_match(before) {
        ls
    } else {
        suffix_start
    }
}

/// Streaming cleaner: feed output chunks as they arrive; tracker lines (even when split across chunks) are removed.
#[derive(Default)]
pub struct TrackedCommandOutputCleaner {
    pending: String,
    discarding_artifact_line: bool,
    /// The tail of a fresh-line marker that was cut off at the end of the previous chunk.
    carried_marker: String,
    /// The text passed on so far did not end with a line break (the cursor is in the middle of a line).
    mid_line: bool,
}

impl TrackedCommandOutputCleaner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clean(&mut self, data: &str) -> String {
        if data.is_empty() {
            return String::new();
        }
        let resolved = self.resolve_fresh_lines(data);
        let out = self.clean_lines(&resolved);
        if !out.is_empty() {
            self.mid_line = !out.ends_with('\n');
        }
        out
    }

    /// Replaces every fresh-line marker by a line break, or by nothing when the text before it already ended one.
    fn resolve_fresh_lines(&mut self, data: &str) -> String {
        let mut text = format!("{}{}", std::mem::take(&mut self.carried_marker), data);
        // a marker cut off by the end of the chunk waits for the next one
        for len in (1..FRESH_LINE_MARKER.len()).rev() {
            if text.is_char_boundary(text.len().saturating_sub(len))
                && text.ends_with(&FRESH_LINE_MARKER[..len])
            {
                self.carried_marker = FRESH_LINE_MARKER[..len].to_string();
                text.truncate(text.len() - len);
                break;
            }
        }
        if !text.contains(FRESH_LINE_MARKER) {
            return text;
        }
        let mut out = String::with_capacity(text.len());
        let mut mid_line = self.mid_line;
        for (i, piece) in text.split(FRESH_LINE_MARKER).enumerate() {
            if i > 0 && mid_line {
                out.push_str("\r\n");
            }
            out.push_str(piece);
            if !piece.is_empty() {
                mid_line = !piece.ends_with('\n');
            } else if i > 0 {
                mid_line = false;
            }
        }
        out
    }

    fn clean_lines(&mut self, data: &str) -> String {
        let source = format!("{}{}", std::mem::take(&mut self.pending), data);
        let mut cursor = 0usize;
        let mut output = String::new();

        while cursor < source.len() {
            if self.discarding_artifact_line {
                match source[cursor..].find('\n') {
                    None => return output,
                    Some(rel) => {
                        cursor += rel + 1;
                        self.discarding_artifact_line = false;
                        continue;
                    }
                }
            }

            let Some(tracker_index) = find_tracker_token_index(&source, cursor) else {
                let trailing = trailing_done_prefix_re()
                    .find(&source[cursor..])
                    .map(|m| cursor + m.start());
                if let Some(done_prefix_index) = trailing.or_else(|| {
                    source[cursor..]
                        .find(TRACKER_DONE_PREFIX)
                        .map(|rel| cursor + rel)
                }) {
                    if trailing.is_some()
                        || has_potential_partial_tracker(&source, done_prefix_index)
                    {
                        let pending_start = line_start(&source, done_prefix_index);
                        output.push_str(&source[cursor..pending_start.max(cursor)]);
                        self.pending = source[pending_start.max(cursor)..].to_string();
                        break;
                    }
                }

                let remainder = &source[cursor..];
                let suffix_len = tracker_prefix_suffix_length(remainder);
                if suffix_len == 0 {
                    output.push_str(remainder);
                    break;
                }
                let pending_start = pending_start_for_partial_tracker(remainder, suffix_len);
                output.push_str(&remainder[..pending_start]);
                self.pending = remainder[pending_start..].to_string();
                break;
            };

            let ls = line_start(&source, tracker_index);
            output.push_str(&source[cursor..ls.max(cursor)]);

            match source[tracker_index..].find('\n') {
                None => {
                    self.discarding_artifact_line = true;
                    break;
                }
                Some(rel) => cursor = tracker_index + rel + 1,
            }
        }

        strip_tracked_command_output_artifacts(&output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker_tail(id: &str, code: i32) -> String {
        format!("{FRESH_LINE_MARKER}__OPSBATCH_AI_DONE_{id}__:{code}__\r\n")
    }

    #[test]
    fn output_without_a_final_newline_keeps_its_last_line() {
        let mut c = TrackedCommandOutputCleaner::new();
        let out = c.clean(&format!("noeol{}", tracker_tail("abc123", 0)));
        assert_eq!(out, "noeol\r\n");
        // the marker line is gone, what came before it is shown, and the cursor ends on a fresh line
        assert!(!out.contains("DONE") && !out.contains('\u{7}'));
    }

    #[test]
    fn output_that_ended_a_line_gets_no_extra_blank_line() {
        let mut c = TrackedCommandOutputCleaner::new();
        let out = c.clean(&format!("hi\r\n{}", tracker_tail("abc123", 0)));
        assert_eq!(out, "hi\r\n");
        // nothing printed at all before the marker: still no blank line
        let mut c = TrackedCommandOutputCleaner::new();
        assert_eq!(c.clean(&tracker_tail("abc123", 0)), "");
    }

    #[test]
    fn the_decision_survives_chunk_boundaries() {
        // text and marker in different chunks
        let mut c = TrackedCommandOutputCleaner::new();
        assert_eq!(c.clean("partial"), "partial");
        let out = c.clean(&tracker_tail("abc123", 0));
        assert_eq!(out, "\r\n");
        let mut c = TrackedCommandOutputCleaner::new();
        assert_eq!(c.clean("done\r\n"), "done\r\n");
        assert_eq!(c.clean(&tracker_tail("abc123", 0)), "");
        // the marker cut at every possible byte
        let tail = tracker_tail("abc123", 0);
        for cut in 0..tail.len() {
            if !tail.is_char_boundary(cut) {
                continue;
            }
            let mut c = TrackedCommandOutputCleaner::new();
            let mut out = c.clean("noeol");
            out.push_str(&c.clean(&tail[..cut]));
            out.push_str(&c.clean(&tail[cut..]));
            assert_eq!(out, "noeol\r\n", "cut at {cut}");
        }
    }

    #[test]
    fn ordinary_output_is_untouched() {
        let mut c = TrackedCommandOutputCleaner::new();
        assert_eq!(
            c.clean("\x1b[31mred\x1b[0m text\r\n"),
            "\x1b[31mred\x1b[0m text\r\n"
        );
        assert_eq!(c.clean("a\x1b"), "a");
        assert_eq!(
            c.clean("[1mb"),
            "\x1b[1mb",
            "an escape split across chunks is put back together"
        );
    }

    #[test]
    fn the_tracked_command_asks_for_the_fresh_line() {
        let t = create_tracked_command("echo hi", "__OPSBATCH_AI_DONE_abc__:");
        assert!(
            t.hidden_tracker
                .contains("printf '\\033]7777;opsbatch-fresh-line\\007'"),
            "{}",
            t.hidden_tracker
        );
        assert!(t.hidden_tracker.contains("__opsbatch_ai_status=$?"));
    }

    const PREFIX: &str = "__OPSBATCH_AI_DONE_abc123__:";

    fn run_cleaner(chunks: &[&str]) -> String {
        let mut c = TrackedCommandOutputCleaner::new();
        chunks.iter().map(|s| c.clean(s)).collect()
    }

    #[test]
    fn tracked_command_wraps_the_visible_command_with_a_hidden_tracker() {
        let t = create_tracked_command("ls -la  \n", PREFIX);
        assert_eq!(t.visible_command, "ls -la");
        assert!(t.hidden_tracker.contains("__opsbatch_ai_status=$?"));
        assert!(t.hidden_tracker.contains(&format!("printf '{PREFIX}%s__")));
    }

    #[test]
    fn done_pattern_extracts_the_exit_code_of_that_execution_only() {
        let re = done_pattern("abc123");
        let caps = re
            .captures("noise __OPSBATCH_AI_DONE_abc123__:-7__ tail")
            .unwrap();
        assert_eq!(&caps[1], "-7");
        assert!(re.captures("__OPSBATCH_AI_DONE_other__:0__").is_none());
    }

    #[test]
    fn strips_echoed_tracker_line_and_done_marker() {
        let raw = "ls\r\nfile1\r\n__opsbatch_ai_status=$?; printf '\\r\\033[K'; printf '__OPSBATCH_AI_DONE_abc123__:%s__\\n' \"$__opsbatch_ai_status\"\r\n__OPSBATCH_AI_DONE_abc123__:0__\r\nuser@host:~$ ";
        let out = strip_tracked_command_output_artifacts(raw);
        assert_eq!(out, "ls\r\nfile1\r\nuser@host:~$ ");
    }

    #[test]
    fn cleaner_passes_ordinary_output_through_unchanged() {
        assert_eq!(
            run_cleaner(&["hello ", "world\r\n", "total 0"]),
            "hello world\r\ntotal 0"
        );
    }

    #[test]
    fn cleaner_removes_tracker_lines_in_a_single_chunk() {
        let chunk = "out\r\n__opsbatch_ai_status=$?; printf 'x'\r\n__OPSBATCH_AI_DONE_abc123__:0__\r\nprompt$ ";
        assert_eq!(run_cleaner(&[chunk]), "out\r\nprompt$ ");
    }

    #[test]
    fn cleaner_handles_tokens_split_at_every_possible_position() {
        let full = "before\r\n__opsbatch_ai_status=$?; printf '\\r'\r\n__OPSBATCH_AI_DONE_abc123__:0__\r\nafter\r\n";
        let expected = "before\r\nafter\r\n";
        for split in 1..full.len() {
            if !full.is_char_boundary(split) {
                continue;
            }
            let (a, b) = full.split_at(split);
            assert_eq!(run_cleaner(&[a, b]), expected, "split at {split}");
        }
    }

    #[test]
    fn cleaner_handles_a_chunk_per_character() {
        let full = "x\r\n__OPSBATCH_AI_DONE_abc123__:130__\r\ny";
        let chars: Vec<String> = full.chars().map(|c| c.to_string()).collect();
        let refs: Vec<&str> = chars.iter().map(String::as_str).collect();
        assert_eq!(run_cleaner(&refs), "x\r\ny");
    }

    #[test]
    fn partial_prefix_at_end_is_held_back_then_released_when_it_is_not_a_tracker() {
        let mut c = TrackedCommandOutputCleaner::new();
        assert_eq!(c.clean("value __opsbatch_ai_"), "value ");
        assert_eq!(c.clean("other text"), "__opsbatch_ai_other text");
    }

    #[test]
    fn utf8_text_around_trackers_is_preserved() {
        let full = "部署完成 ✓\r\n__OPSBATCH_AI_DONE_abc123__:0__\r\n结束";
        for split in 1..full.len() {
            if !full.is_char_boundary(split) {
                continue;
            }
            let (a, b) = full.split_at(split);
            assert_eq!(
                run_cleaner(&[a, b]),
                "部署完成 ✓\r\n结束",
                "split at {split}"
            );
        }
    }
}
