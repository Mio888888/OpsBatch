//! Port of `src/utils/terminalFollowUp.ts`: decides whether a command's terminal output is worth
//! sending back to the model and formats results for the follow-up message.

use regex::{Regex, RegexBuilder};
use std::sync::LazyLock;

/// Compile a pattern that is case-insensitive and uses ASCII word boundaries like JS `\b`.
fn ci(pattern: &str) -> Regex {
    let pattern = pattern.replace(r"\b", r"(?-u:\b)");
    RegexBuilder::new(&pattern).case_insensitive(true).build().expect("valid regex")
}

fn cs(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

static ANSI_SEQUENCE_RE: LazyLock<Regex> =
    LazyLock::new(|| cs(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\].*?(?:\x07|\x1b\\)"));
static CONTROL_SEQUENCE_RE: LazyLock<Regex> =
    LazyLock::new(|| cs(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]"));
static SHELL_PROMPT_RE: LazyLock<Regex> =
    LazyLock::new(|| cs(r"^[A-Za-z0-9_.-]+@[A-Za-z0-9_.-]+:[^\n]*[#$]\s*$"));
static SIMPLE_PROMPT_RE: LazyLock<Regex> = LazyLock::new(|| cs(r"^(?:[$#]|❯|➜)\s*$"));

static ERROR_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"\b(?:error|failed|failure|fatal|exception|traceback|panic|permission denied|access denied|not found|no such file|command not found|segmentation fault|timed out|timeout|connection refused|unauthorized|forbidden|exit code|curl:\s*\(\d+\)|npm ERR!|ERR!|E:\s|Err:)\b")
});
static CJK_ERROR_RE: LazyLock<Regex> =
    LazyLock::new(|| cs(r"(?:错误|失败|拒绝|无权限|不存在|未找到|超时|异常)"));
static INTERACTIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"(?:password|passphrase|enter\s+.+|input\s+.+|select\s+.+|choose\s+.+|confirm\s+.+|continue\?|proceed\?|are you sure|\[[YyNn]/[^\]]+\]|\([^)]*(?:yes/no|y/n)[^)]*\)|press any key|请输入|请选择|确认|是否|密码|口令)")
});
static INTERACTIVE_TAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| ci(r"(?:password|passphrase|请输入|请选择|确认|是否|密码|口令)[^\n]*[:?]\s*$"));

static SHELL_TRACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^\+\s+(?:sh\s+-c|bash\s+-c|sudo\b|env\b|apt(?:-get)?\b|yum\b|dnf\b|brew\b|docker\b|curl\b|wget\b|chmod\b|install\b|echo\b|cat\b|tee\b|mkdir\b|rm\b|mv\b|cp\b|sed\b|awk\b|grep\b|gpg\b|systemctl\b|snap\b|pip\b|npm\b|pnpm\b|yarn\b|apk\b)")
});
static LONG_COMMAND_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^(?:\+?\s*)?(?:(?:sudo|env|sh\s+-c|bash\s+-c|apt-get|apt|yum|dnf|brew|docker|curl|wget|chmod|install|echo|cat|tee|mkdir|rm|mv|cp|sed|awk|grep|gpg|pip|npm|pnpm|yarn|apk|DEBIAN_FRONTEND=|[A-Z_]+=[^\s]+)\b)")
});
static INSTALL_NOISE_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^(?:\+?\s*)?(?:Get:\d+|Hit:\d+|Ign:\d+|Fetched\b|Reading package lists|Building dependency tree|Reading state information|Selecting previously unselected|Preparing to unpack|Unpacking|Setting up|Processing triggers|update-alternatives|debconf: delaying package configuration|Collecting\s+|Downloading\s+|Installing collected packages|Successfully (?:built|installed)|Pulling fs layer|Waiting|Extracting|Verifying Checksum|Download complete|Pull complete|Status: Downloaded|Removing intermediate container|Step \d+/\d+|--->|==>|Already up-to-date|Package .* is already installed|Installed:|Updated:|Complete!$|Loaded plugins:)")
});
static INSTALL_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^(?:Get:\d+|Hit:\d+|Ign:\d+|Fetched\b|Reading package lists|Building dependency tree|Reading state information|The following .*packages|Suggested packages|Recommended packages|Need to get\b|After this operation\b|Selecting previously unselected|Preparing to unpack|Unpacking|Setting up|Processing triggers|Collecting\s+|Downloading\s+|Installing collected packages|Pulling fs layer|Step \d+/\d+|Sending build context to Docker daemon|#\d+\s|Successfully tagged\b|exporting to image\b)")
});
static INSTALL_DETAIL_NOISE_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^(?:The following .*packages|Suggested packages|Recommended packages|Need to get\b|After this operation\b|Additional disk space|Sending build context to Docker daemon|#\d+\s|=>\s|CACHED\s|DONE\s|naming to docker\.io|exporting layers|exporting manifest|exporting config|exporting attestation manifest|exporting manifest list|exporting to image|Successfully tagged\b|[a-z0-9][a-z0-9+.:~_-]+(?:\s+[a-z0-9][a-z0-9+.:~_-]+){1,})$")
});
static PROGRESS_NOISE_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"(?:\b\d{1,3}%\b|\[[#=>.\-\s]{8,}\]|[#=]{12,}|^\s*(?:\||/|-|\\)\s*$|^\s*\d+(?:\.\d+)?\s*(?:B|KB|MB|GB|KiB|MiB|GiB)/?s?\b)")
});
static PACKAGE_SUMMARY_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"^(?:\d+\s+)?(?:upgraded|newly installed|to remove|not upgraded|packages? can be upgraded|files? and directories currently installed)\b")
});
static VALUEFUL_COMPLETION_RE: LazyLock<Regex> = LazyLock::new(|| {
    ci(r"\b(?:created|started|stopped|restarted|enabled|disabled|listening|active \(running\)|version|successfully|completed|done|ok|passed)\b")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpReason {
    Empty,
    Noise,
    Error,
    Interactive,
    Meaningful,
}

#[derive(Debug, Clone)]
pub struct TerminalFollowUpDecision {
    pub should_follow_up: bool,
    pub output: String,
    pub reason: FollowUpReason,
}

impl TerminalFollowUpDecision {
    fn skip(reason: FollowUpReason) -> Self {
        Self { should_follow_up: false, output: String::new(), reason }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalCommandResult {
    pub status: String,
    pub output: String,
    pub exit_code: Option<i32>,
}

pub fn format_terminal_command_result(result: &TerminalCommandResult) -> String {
    let exit_text = match result.exit_code {
        Some(code) => format!("，退出码 {code}"),
        None => String::new(),
    };
    let output = result.output.trim();
    let with_note = |note: &str| format!("{output}\n\n{note}").trim().to_string();
    let detail = if result.status == "timeout" {
        with_note("[OpsBatch] 命令仍未收到完成信号，已等待到超时，可能仍在运行或等待交互。")
    } else if result.status == "closed" {
        with_note("[OpsBatch] 终端会话已关闭，命令执行被中断或会话结束。")
    } else if result.status == "write_failed" {
        with_note("[OpsBatch] 命令写入终端失败。")
    } else if matches!(result.exit_code, Some(code) if code != 0) {
        with_note(&format!("[OpsBatch] 命令退出码：{}", result.exit_code.unwrap()))
    } else if output.is_empty() {
        "[OpsBatch] 命令执行完成，但没有产生输出。".to_string()
    } else {
        output.to_string()
    };
    format!("[当前命令：{}{}]\n{}", result.status, exit_text, detail)
}

/// Strip ANSI sequences and control characters; lone `\r` becomes a newline.
pub fn strip_terminal_control(text: &str) -> String {
    let no_ansi = ANSI_SEQUENCE_RE.replace_all(text, "");
    let mut normalized = String::with_capacity(no_ansi.len());
    let mut chars = no_ansi.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() != Some(&'\n') {
            normalized.push('\n');
        } else {
            normalized.push(c);
        }
    }
    CONTROL_SEQUENCE_RE.replace_all(&normalized, "").into_owned()
}

fn is_prompt_line(line: &str) -> bool {
    SHELL_PROMPT_RE.is_match(line) || SIMPLE_PROMPT_RE.is_match(line)
}

fn has_error_signal(line: &str) -> bool {
    ERROR_RE.is_match(line) || CJK_ERROR_RE.is_match(line)
}

fn has_interactive_signal(line: &str) -> bool {
    INTERACTIVE_RE.is_match(line) || INTERACTIVE_TAIL_RE.is_match(line)
}

fn is_long_command_echo(line: &str) -> bool {
    line.encode_utf16().count() >= 140 && LONG_COMMAND_RE.is_match(line)
}

fn has_install_context(lines: &[&str]) -> bool {
    lines.iter().any(|l| INSTALL_CONTEXT_RE.is_match(l))
}

fn is_noise_line(line: &str, install_context: bool) -> bool {
    if line.is_empty() || is_prompt_line(line) {
        return true;
    }
    if has_error_signal(line) || has_interactive_signal(line) {
        return false;
    }
    SHELL_TRACE_RE.is_match(line)
        || is_long_command_echo(line)
        || INSTALL_NOISE_RE.is_match(line)
        || (install_context && INSTALL_DETAIL_NOISE_RE.is_match(line))
        || PROGRESS_NOISE_RE.is_match(line)
        || PACKAGE_SUMMARY_RE.is_match(line)
}

/// Keep the tail of `text` (at most `max_chars` characters).
fn limit_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

pub fn classify_terminal_follow_up(raw_buffer: &str) -> TerminalFollowUpDecision {
    let cleaned = strip_terminal_control(raw_buffer);
    let all: Vec<&str> = cleaned.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
    let lines = &all[all.len().saturating_sub(100)..];

    if lines.is_empty() {
        return TerminalFollowUpDecision::skip(FollowUpReason::Empty);
    }

    let relevant: Vec<&str> = lines.iter().copied().filter(|l| !is_prompt_line(l)).collect();
    let base: &[&str] = if relevant.is_empty() { lines } else { &relevant };
    let output_lines: &[&str] = &base[base.len().saturating_sub(80)..];
    let has_error = output_lines.iter().any(|l| has_error_signal(l));
    let has_interactive = output_lines.iter().any(|l| has_interactive_signal(l));
    let output = limit_text(&output_lines.join("\n"), 12_000).trim().to_string();

    if output.is_empty() {
        return TerminalFollowUpDecision::skip(FollowUpReason::Empty);
    }
    if has_error {
        return TerminalFollowUpDecision {
            should_follow_up: true,
            output,
            reason: FollowUpReason::Error,
        };
    }
    if has_interactive {
        return TerminalFollowUpDecision {
            should_follow_up: true,
            output,
            reason: FollowUpReason::Interactive,
        };
    }

    let install_context = has_install_context(output_lines);
    let noise_count = output_lines.iter().filter(|l| is_noise_line(l, install_context)).count();
    let meaningful: Vec<&str> =
        output_lines.iter().copied().filter(|l| !is_noise_line(l, install_context)).collect();
    let trace_count = output_lines.iter().filter(|l| SHELL_TRACE_RE.is_match(l)).count();
    let meaningful_text = meaningful.join("\n").trim().to_string();

    if meaningful.is_empty() {
        return TerminalFollowUpDecision::skip(FollowUpReason::Noise);
    }

    let total = output_lines.len() as f64;
    let noise_ratio = noise_count as f64 / total;
    let trace_ratio = trace_count as f64 / total;
    let meaningful_chars = meaningful_text.encode_utf16().count();

    if trace_ratio >= 0.6 && meaningful.len() <= 2 && meaningful_chars < 240 {
        return TerminalFollowUpDecision::skip(FollowUpReason::Noise);
    }
    if noise_ratio >= 0.8 && meaningful_chars < 240 {
        return TerminalFollowUpDecision::skip(FollowUpReason::Noise);
    }

    if meaningful.len() >= 2
        || meaningful_chars >= 120
        || meaningful.iter().any(|l| VALUEFUL_COMPLETION_RE.is_match(l))
    {
        let chosen = if meaningful_text.is_empty() { &output } else { &meaningful_text };
        return TerminalFollowUpDecision {
            should_follow_up: true,
            output: limit_text(chosen, 8_000).trim().to_string(),
            reason: FollowUpReason::Meaningful,
        };
    }

    TerminalFollowUpDecision::skip(FollowUpReason::Noise)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_without_output_is_reported() {
        let out = format_terminal_command_result(&TerminalCommandResult {
            status: "completed".into(),
            output: String::new(),
            exit_code: Some(0),
        });
        assert!(out.starts_with("[当前命令：completed，退出码 0]"));
        assert!(out.contains("没有产生输出"));
    }

    #[test]
    fn nonzero_exit_timeout_and_closed_add_notes() {
        let r = |status: &str, output: &str, code: Option<i32>| {
            format_terminal_command_result(&TerminalCommandResult {
                status: status.into(),
                output: output.into(),
                exit_code: code,
            })
        };
        assert!(r("completed", "boom", Some(2)).contains("命令退出码：2"));
        assert!(r("timeout", "", None).contains("等待到超时"));
        assert!(r("closed", "x", None).contains("会话已关闭"));
        assert!(r("write_failed", "", None).contains("写入终端失败"));
        assert_eq!(r("completed", "ok", Some(0)), "[当前命令：completed，退出码 0]\nok");
    }

    #[test]
    fn strips_ansi_and_controls() {
        assert_eq!(strip_terminal_control("\x1b[31mred\x1b[0m\r\nline\rnext\x07"), "red\r\nline\nnext");
    }

    #[test]
    fn errors_and_prompts_trigger_follow_up() {
        let d = classify_terminal_follow_up("cat x\ncat: x: No such file or directory\nuser@h:~$ ");
        assert!(d.should_follow_up);
        assert_eq!(d.reason, FollowUpReason::Error);
        assert!(d.output.contains("No such file"));
        assert!(!d.output.contains("user@h"));

        let d = classify_terminal_follow_up("[sudo] password for root:");
        assert_eq!(d.reason, FollowUpReason::Interactive);
        assert!(d.should_follow_up);

        let d = classify_terminal_follow_up("Do you want to continue? [Y/n]");
        assert_eq!(d.reason, FollowUpReason::Interactive);
    }

    #[test]
    fn empty_and_prompt_only_output_is_skipped() {
        assert_eq!(classify_terminal_follow_up("").reason, FollowUpReason::Empty);
        let d = classify_terminal_follow_up("\n  \n");
        assert!(!d.should_follow_up);
    }

    #[test]
    fn install_noise_is_skipped_but_real_output_follows() {
        let noise = "Get:1 http://deb.example stable InRelease\nHit:2 http://deb.example updates\nReading package lists...\nBuilding dependency tree...\n";
        let d = classify_terminal_follow_up(noise);
        assert!(!d.should_follow_up);
        assert_eq!(d.reason, FollowUpReason::Noise);

        let d = classify_terminal_follow_up("total 8\ndrwxr-xr-x 2 root root 4096 a\n-rw-r--r-- 1 root root 12 b");
        assert!(d.should_follow_up);
        assert_eq!(d.reason, FollowUpReason::Meaningful);
    }

    #[test]
    fn single_short_line_needs_a_valueful_word() {
        assert!(!classify_terminal_follow_up("hello").should_follow_up);
        assert!(classify_terminal_follow_up("nginx started").should_follow_up);
    }

    #[test]
    fn output_is_limited_to_the_tail() {
        let long: String = (0..400).map(|i| format!("line number {i} with some words\n")).collect();
        let d = classify_terminal_follow_up(&long);
        assert!(d.should_follow_up);
        assert!(d.output.contains("line number 399"));
        assert!(!d.output.contains("line number 100 "));
    }
}
