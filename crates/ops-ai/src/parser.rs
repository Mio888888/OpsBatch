//! Port of the SSH side of `src/utils/aiActionParser.ts`: turns an assistant reply into
//! pending command actions (ACTION blocks, shell fences, COMMAND_PLAN JSON).
//! RDP operation plans are handled by the RDP milestone.

use crate::client::{ai_validate_command_plan, AiActionAssessment};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

const DEFAULT_DESCRIPTION: &str = "执行终端命令";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Command,
    FileInspect,
    Diagnose,
    RdpAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionSource {
    ActionBlock,
    Fence,
    CommandPlan,
    RdpPlan,
}

/// One remote-desktop operation proposed by the AI (coordinates are desktop pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpOperation {
    Click {
        x: u16,
        y: u16,
        button: u8,
        double_click: bool,
    },
    Move {
        x: u16,
        y: u16,
    },
    Drag {
        from_x: u16,
        from_y: u16,
        to_x: u16,
        to_y: u16,
        button: u8,
    },
    Scroll {
        x: u16,
        y: u16,
        delta: i32,
        vertical: bool,
    },
    Type {
        text: String,
    },
    Key {
        keys: Vec<String>,
    },
    Wait {
        ms: u32,
    },
}

#[derive(Debug, Clone)]
pub struct ParsedPendingAction {
    pub id: String,
    pub kind: ActionKind,
    pub description: String,
    pub command: Option<String>,
    pub rdp_operations: Option<Vec<RdpOperation>>,
    pub source: Option<ActionSource>,
    pub intent: Option<String>,
    pub expected_outcome: Option<String>,
    pub assessment: Option<AiActionAssessment>,
    pub assessment_loading: bool,
    pub assessment_error: Option<String>,
    pub approved: bool,
    pub executed: bool,
    pub rejected: bool,
}

impl ParsedPendingAction {
    fn command(id: String, description: String, command: String, source: ActionSource) -> Self {
        Self {
            id,
            kind: ActionKind::Command,
            description,
            command: Some(command),
            rdp_operations: None,
            source: Some(source),
            intent: None,
            expected_outcome: None,
            assessment: None,
            assessment_loading: false,
            assessment_error: None,
            approved: false,
            executed: false,
            rejected: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlanNotice {
    pub version: u32,
    pub summary: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedAiActionsResult {
    pub display_content: String,
    pub actions: Vec<ParsedPendingAction>,
    pub command_plan_notice: Option<CommandPlanNotice>,
    pub rdp_parse_error: Option<String>,
    pub rdp_parse_diagnostic: Option<String>,
}

const SHELL_SCRIPT_FENCE_LANGUAGES: &[&str] = &["bash", "sh", "shell", "zsh"];
const TERMINAL_FENCE_LANGUAGES: &[&str] = &["console", "terminal"];

const COMMON_SHELL_COMMANDS: &[&str] = &[
    "apt",
    "apt-get",
    "awk",
    "bash",
    "cat",
    "cd",
    "chmod",
    "chown",
    "cp",
    "curl",
    "df",
    "dnf",
    "docker",
    "du",
    "echo",
    "export",
    "find",
    "free",
    "git",
    "grep",
    "head",
    "helm",
    "id",
    "journalctl",
    "kubectl",
    "less",
    "ls",
    "mkdir",
    "mv",
    "netstat",
    "nginx",
    "node",
    "npm",
    "ping",
    "pip",
    "pip3",
    "pnpm",
    "printf",
    "ps",
    "psql",
    "pwd",
    "python",
    "python3",
    "redis-cli",
    "rm",
    "rsync",
    "scp",
    "sed",
    "sh",
    "ss",
    "sudo",
    "systemctl",
    "tail",
    "tar",
    "top",
    "touch",
    "uname",
    "unzip",
    "vim",
    "vi",
    "wget",
    "whoami",
    "yum",
    "yarn",
    "zip",
    "zsh",
];

pub(crate) struct FenceBlock {
    pub language: String,
    pub code: String,
    pub start_index: usize,
}

struct CommandPlanCandidate {
    raw_plan: String,
    start_index: usize,
    end_index: usize,
}

static ACTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\[ACTION(?::([^\]]+))?\](.*?)\[/ACTION\]").unwrap());
static PLAN_TAGGED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<COMMAND_PLAN\s*>?\s*(.*?)\s*</COMMAND_PLAN?>?").unwrap());
static PLAN_OPEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<COMMAND_PLAN\s*>?").unwrap());
static PLAN_CLOSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</COMMAND_PLAN?>?").unwrap());
static FENCE_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\r?\n)?(?:```|~~~)[ \t]*(\r?\n|$)").unwrap());
static TRAILING_PUNCT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*["'}\])]*[。.!！]*\s*$"#).unwrap());
static STRIP_PLAN_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<COMMAND_PLAN\s*>?.*?</COMMAND_PLAN?>?").unwrap());
static STRIP_PLAN_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<COMMAND_PLAN\s*>?.*$").unwrap());
static STRIP_PLAN_CLOSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</COMMAND(?:_PLAN?)?>?\s*$").unwrap());
static SIMPLE_PROMPT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:\$|>)\s+(.+)$").unwrap());
static USER_HOST_PROMPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:\[[^\]]+\]\s*)?[A-Za-z0-9._-]+@[A-Za-z0-9._-]+:[^#$\n]*[#$]\s+(.+)$")
        .unwrap()
});
static STRUCTURED_DATA_START_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:[{\[}\]]|---\s*$|[A-Za-z0-9_]+:\s*(?:$|[A-Za-z0-9_'"\[{]))"#).unwrap()
});
static ENV_ASSIGN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)*[A-Za-z_][A-Za-z0-9_]*=").unwrap()
});
static PATH_LIKE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\.\.?/|~/|/)[^\s]+").unwrap());
static OPERATOR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[|&]{2}|\||>>?|<\s*\S+").unwrap());
static WHITESPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+").unwrap());
static BULLET_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-*+]\s+").unwrap());
static ORDERED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+[.)]\s+").unwrap());
static COLON_TAIL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[：:]\s*$").unwrap());
static REPAIR_TRAILING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[,\s]+$").unwrap());
static REPAIR_COLON_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":\s*$").unwrap());
static REPAIR_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#",\s*"[A-Za-z_]+"\s*:\s*$"#).unwrap());

/// Parse with COMMAND_PLAN validation (the Tauri `parseAiPendingActionsAsync`; the validator is
/// synchronous here). `single_command` keeps only the first action (SSH / terminal scopes).
pub fn parse_ai_pending_actions_with_plan(
    content: &str,
    single_command: bool,
) -> ParsedAiActionsResult {
    let Some(candidate) = extract_command_plan_candidate(content) else {
        return constrain(parse_ai_pending_actions(content), single_command);
    };

    if let Some(notice) = parse_empty_command_plan_notice(&candidate.raw_plan) {
        return ParsedAiActionsResult {
            display_content: remove_command_plan_from_display(content, &candidate)
                .trim()
                .to_string(),
            actions: Vec::new(),
            command_plan_notice: Some(notice),
            ..Default::default()
        };
    }

    match ai_validate_command_plan(candidate.raw_plan.clone()) {
        Ok(plan) => constrain(
            ParsedAiActionsResult {
                display_content: remove_command_plan_from_display(content, &candidate)
                    .trim()
                    .to_string(),
                actions: plan
                    .steps
                    .into_iter()
                    .map(|step| {
                        let mut action = ParsedPendingAction::command(
                            step.id,
                            step.description,
                            step.command,
                            ActionSource::CommandPlan,
                        );
                        action.intent = Some(step.intent);
                        action.expected_outcome = Some(step.expected_outcome);
                        action
                    })
                    .collect(),
                command_plan_notice: None,
                ..Default::default()
            },
            single_command,
        ),
        Err(_) => constrain(parse_ai_pending_actions(content), single_command),
    }
}

/// Entry point used by the chat store: in a remote-desktop session an `<RDP_PLAN>` is tried first.
pub fn parse_ai_pending_actions_for(
    content: &str,
    rdp_desktop: Option<(u32, u32)>,
    single_command: bool,
) -> ParsedAiActionsResult {
    if let Some(size) = rdp_desktop {
        if let Some(r) = crate::rdp_parser::parse_rdp_actions(content, size) {
            return ParsedAiActionsResult {
                display_content: r.display_content,
                actions: r.actions,
                command_plan_notice: None,
                rdp_parse_error: r.parse_error,
                rdp_parse_diagnostic: r.parse_diagnostic,
            };
        }
    }
    parse_ai_pending_actions_with_plan(content, single_command)
}

pub fn parse_ssh_pending_actions(content: &str) -> ParsedAiActionsResult {
    constrain(parse_ai_pending_actions(content), true)
}

pub fn parse_ai_pending_actions(content: &str) -> ParsedAiActionsResult {
    let mut actions = Vec::new();
    let mut seen = HashSet::new();

    for caps in ACTION_RE.captures_iter(content) {
        let tag = caps.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or("").trim();
        let colon = tag.find(':').filter(|i| *i > 0);
        let description = match colon {
            Some(i) => tag[..i].trim(),
            None => tag,
        };
        let raw_command = match colon {
            Some(i) if body.is_empty() => tag[i + 1..].trim(),
            _ => body,
        };
        add_command_action(
            &mut actions,
            &mut seen,
            description,
            raw_command,
            ActionSource::ActionBlock,
        );
    }

    let display_content = ACTION_RE.replace_all(content, "").trim().to_string();

    for block in extract_fence_blocks(&display_content) {
        let command = shell_command_from_fence(&block.language, &block.code);
        if command.is_empty() {
            continue;
        }
        let description = extract_description_before_fence(&display_content, block.start_index);
        add_command_action(
            &mut actions,
            &mut seen,
            &description,
            &command,
            ActionSource::Fence,
        );
    }

    ParsedAiActionsResult {
        display_content,
        actions,
        command_plan_notice: None,
        rdp_parse_error: None,
        rdp_parse_diagnostic: None,
    }
}

fn constrain(mut result: ParsedAiActionsResult, single_command: bool) -> ParsedAiActionsResult {
    if single_command && result.actions.len() > 1 {
        result.actions.truncate(1);
    }
    result
}

pub fn extract_empty_command_plan_notice(content: &str) -> Option<ParsedAiActionsResult> {
    let candidate = extract_command_plan_candidate(content)?;
    let notice = parse_empty_command_plan_notice(&candidate.raw_plan)?;
    Some(ParsedAiActionsResult {
        display_content: remove_command_plan_from_display(content, &candidate)
            .trim()
            .to_string(),
        actions: Vec::new(),
        command_plan_notice: Some(notice),
        ..Default::default()
    })
}

pub fn strip_command_plan_block(content: &str) -> String {
    let result = STRIP_PLAN_FULL.replace_all(content, "");
    let result = STRIP_PLAN_OPEN.replace(&result, "");
    let result = STRIP_PLAN_CLOSE.replace(&result, "");
    result.trim().to_string()
}

fn add_command_action(
    actions: &mut Vec<ParsedPendingAction>,
    seen: &mut HashSet<String>,
    description: &str,
    raw_command: &str,
    source: ActionSource,
) {
    let command = sanitize_shell_command(raw_command);
    let key = normalize_command_for_dedupe(&command);
    if command.is_empty() || key.is_empty() || !seen.insert(key) {
        return;
    }
    let description = if description.is_empty() {
        DEFAULT_DESCRIPTION
    } else {
        description
    };
    actions.push(ParsedPendingAction::command(
        uuid::Uuid::new_v4().to_string(),
        description.to_string(),
        command,
        source,
    ));
}

// ---------------------------------------------------------------------------
// COMMAND_PLAN extraction
// ---------------------------------------------------------------------------

fn extract_command_plan_candidate(content: &str) -> Option<CommandPlanCandidate> {
    if let Some(caps) = PLAN_TAGGED_RE.captures(content) {
        let whole = caps.get(0).unwrap();
        return Some(CommandPlanCandidate {
            raw_plan: caps.get(1).unwrap().as_str().trim().to_string(),
            start_index: whole.start(),
            end_index: whole.end(),
        });
    }

    if let Some(open) = PLAN_OPEN_RE.find(content) {
        if let Some((text, json_end)) = slice_balanced_json(content, open.end()) {
            let end_index = match PLAN_CLOSE_RE.find(&content[json_end..]) {
                Some(close) => json_end + close.end(),
                None => json_end,
            };
            return Some(CommandPlanCandidate {
                raw_plan: text.trim().to_string(),
                start_index: open.start(),
                end_index,
            });
        }
    }

    for block in extract_fence_blocks(content) {
        if block.language != "json" || !looks_like_command_plan_json(&block.code) {
            continue;
        }
        return Some(CommandPlanCandidate {
            raw_plan: block.code.trim().to_string(),
            start_index: block.start_index,
            end_index: find_fence_end(content, block.start_index),
        });
    }
    None
}

fn looks_like_command_plan_json(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .map(|v| v.get("steps").is_some_and(|s| s.is_array()))
        .unwrap_or(false)
}

pub(crate) fn find_fence_end(content: &str, fence_start: usize) -> usize {
    let rest = &content[fence_start..];
    match FENCE_END_RE.captures(rest) {
        Some(caps) => fence_start + caps.get(1).unwrap().start(),
        None => content.len(),
    }
}

fn remove_command_plan_from_display(content: &str, candidate: &CommandPlanCandidate) -> String {
    let after = &content[candidate.end_index..];
    let end = if TRAILING_PUNCT_RE.is_match(after) {
        content.len()
    } else {
        candidate.end_index
    };
    format!("{}{}", &content[..candidate.start_index], &content[end..])
}

fn parse_empty_command_plan_notice(raw: &str) -> Option<CommandPlanNotice> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    if parsed.get("version")?.as_f64()? != 1.0 {
        return None;
    }
    let steps = parsed.get("steps")?.as_array()?;
    if !steps.is_empty() {
        return None;
    }
    let summary = parsed.get("summary")?.as_str()?.trim();
    if summary.is_empty() || summary.encode_utf16().count() > 200 {
        return None;
    }
    Some(CommandPlanNotice {
        version: 1,
        summary: summary.to_string(),
    })
}

/// First balanced JSON object at/after `start` (tolerates streaming truncation by repairing it).
pub(crate) fn slice_balanced_json(content: &str, start: usize) -> Option<(String, usize)> {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut first_brace: Option<usize> = None;
    let mut open_stack: Vec<char> = Vec::new();

    for (idx, ch) in content[start..].char_indices() {
        let idx = start + idx;
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => {
                if depth == 0 && ch == '{' {
                    first_brace = Some(idx);
                }
                depth += 1;
                open_stack.push(ch);
            }
            '}' | ']' => {
                depth -= 1;
                open_stack.pop();
                if depth == 0 {
                    if let Some(fb) = first_brace {
                        return Some((content[fb..idx + 1].to_string(), idx + 1));
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(fb) = first_brace {
        if !open_stack.is_empty() {
            return Some((
                repair_truncated_json(&content[fb..], &open_stack),
                content.len(),
            ));
        }
    }
    None
}

fn repair_truncated_json(partial: &str, open_stack: &[char]) -> String {
    let mut repaired = partial.to_string();
    let mut in_string = false;
    let mut escape = false;
    for ch in repaired.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        }
    }
    if in_string {
        repaired.push('"');
    }
    let mut repaired = REPAIR_TRAILING_RE.replace(&repaired, "").into_owned();
    repaired = REPAIR_COLON_RE.replace(&repaired, "").into_owned();
    repaired = REPAIR_KEY_RE.replace(&repaired, "").into_owned();
    for open in open_stack.iter().rev() {
        repaired.push(if *open == '{' { '}' } else { ']' });
    }
    repaired
}

// ---------------------------------------------------------------------------
// Fenced code blocks
// ---------------------------------------------------------------------------

/// Equivalent of the original
/// `/(^|\n)(```|~~~)([^\r\n]*)\r?\n([\s\S]*?)(?:\r?\n)?\2[ \t]*(?=\r?\n|$)/g`
/// (the regex crate has no back-references or look-ahead).
pub(crate) fn extract_fence_blocks(content: &str) -> Vec<FenceBlock> {
    let mut blocks = Vec::new();
    // Earliest index of the '\n' that may precede the next opening marker.
    let mut from = 0usize;
    let mut first = true;
    loop {
        let m = if std::mem::take(&mut first) && starts_with_marker(content, 0).is_some() {
            Some(0)
        } else {
            next_marker_after_newline(content, from)
        };
        let Some(m) = m else { break };
        match try_fence_at(content, m) {
            Some((block, end)) => {
                blocks.push(block);
                from = end;
            }
            None => from = m,
        }
    }
    blocks
}

fn starts_with_marker(content: &str, at: usize) -> Option<&'static str> {
    let rest = content.get(at..)?;
    if rest.starts_with("```") {
        Some("```")
    } else if rest.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// Find the next index `p > from-1` such that `content[p-1] == '\n'` (with `p-1 >= from`) and a
/// marker starts at `p`.
fn next_marker_after_newline(content: &str, from: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'\n' && starts_with_marker(content, i + 1).is_some() {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn try_fence_at(content: &str, m: usize) -> Option<(FenceBlock, usize)> {
    let marker = starts_with_marker(content, m)?;
    let bytes = content.as_bytes();
    let info_start = m + 3;
    let mut line_end = info_start;
    while line_end < bytes.len() && bytes[line_end] != b'\r' && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    let body_start = match bytes.get(line_end) {
        Some(b'\n') => line_end + 1,
        Some(b'\r') if bytes.get(line_end + 1) == Some(&b'\n') => line_end + 2,
        _ => return None,
    };
    let info = &content[info_start..line_end];

    let mut i = body_start;
    loop {
        // (?:\r?\n)? marker
        let nl_len = if bytes[i..].starts_with(b"\r\n") {
            2
        } else if bytes[i..].starts_with(b"\n") {
            1
        } else {
            0
        };
        for skip in [nl_len, 0] {
            let j = i + skip;
            if content[j..].starts_with(marker) {
                let mut k = j + 3;
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                let ok = k == bytes.len()
                    || bytes[k..].starts_with(b"\n")
                    || bytes[k..].starts_with(b"\r\n");
                if ok {
                    return Some((
                        FenceBlock {
                            language: normalize_fence_language(info),
                            code: content[body_start..i].to_string(),
                            start_index: m,
                        },
                        k,
                    ));
                }
            }
            if nl_len == 0 {
                break;
            }
        }
        if i >= bytes.len() {
            return None;
        }
        i += 1;
        while i < bytes.len() && !content.is_char_boundary(i) {
            i += 1;
        }
    }
}

fn normalize_fence_language(info: &str) -> String {
    let token = info.split_whitespace().next().unwrap_or("");
    let token = token.strip_prefix('{').unwrap_or(token);
    let token = token.strip_prefix('.').unwrap_or(token);
    let token = match token.find(['}', ',']) {
        Some(i) => &token[..i],
        None => token,
    };
    token.to_lowercase()
}

fn shell_command_from_fence(language: &str, code: &str) -> String {
    if SHELL_SCRIPT_FENCE_LANGUAGES.contains(&language) {
        return code.to_string();
    }
    if TERMINAL_FENCE_LANGUAGES.contains(&language) {
        let prompted = extract_terminal_transcript_commands(code);
        if !prompted.is_empty() {
            return prompted.join("\n");
        }
        return if looks_like_shell_commands(code) {
            code.to_string()
        } else {
            String::new()
        };
    }
    if !language.is_empty() {
        return String::new();
    }
    if looks_like_shell_commands(code) {
        code.to_string()
    } else {
        String::new()
    }
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn extract_terminal_transcript_commands(command: &str) -> Vec<String> {
    normalize_newlines(command)
        .split('\n')
        .filter_map(extract_shell_prompt_command)
        .collect()
}

pub fn sanitize_shell_command(command: &str) -> String {
    let normalized = normalize_newlines(command);
    let raw_lines: Vec<&str> = normalized
        .split('\n')
        .map(|l| l.trim_end_matches([' ', '\t']))
        .collect();
    let prompted: Vec<String> = raw_lines
        .iter()
        .filter_map(|l| extract_shell_prompt_command(l))
        .collect();
    let lines: Vec<String> = if prompted.is_empty() {
        raw_lines
            .iter()
            .map(|l| extract_shell_prompt_command(l).unwrap_or_else(|| l.to_string()))
            .collect()
    } else {
        prompted
    };
    lines
        .into_iter()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn extract_shell_prompt_command(line: &str) -> Option<String> {
    if let Some(c) = SIMPLE_PROMPT_RE.captures(line) {
        return Some(c[1].to_string());
    }
    if let Some(c) = USER_HOST_PROMPT_RE.captures(line) {
        return Some(c[1].to_string());
    }
    None
}

fn looks_like_shell_commands(code: &str) -> bool {
    let sanitized = sanitize_shell_command(code);
    let lines: Vec<&str> = sanitized
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return false;
    }
    let shell_like = lines.iter().filter(|l| is_likely_shell_line(l)).count();
    if shell_like == 0 {
        return false;
    }
    if lines.len() == 1 {
        return shell_like == 1 && !STRUCTURED_DATA_START_RE.is_match(lines[0]);
    }
    shell_like >= lines.len().div_ceil(2)
}

fn is_likely_shell_line(line: &str) -> bool {
    if ENV_ASSIGN_RE.is_match(line) || PATH_LIKE_RE.is_match(line) || OPERATOR_RE.is_match(line) {
        return true;
    }
    let first = line.split_whitespace().next().unwrap_or("");
    COMMON_SHELL_COMMANDS.contains(&first)
}

fn normalize_command_for_dedupe(command: &str) -> String {
    WHITESPACE_RE
        .replace_all(&sanitize_shell_command(command), " ")
        .trim()
        .to_string()
}

fn extract_description_before_fence(content: &str, fence_start: usize) -> String {
    for line in content[..fence_start].split('\n').rev() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("```") || line.starts_with("~~~") {
            continue;
        }
        let c = HEADING_RE.replace(line, "");
        let c = BULLET_RE.replace(&c, "");
        let c = ORDERED_RE.replace(&c, "");
        let c = COLON_TAIL_RE.replace(&c, "");
        let candidate = c.trim();
        if !candidate.is_empty() {
            return if candidate.chars().count() > 80 {
                format!("{}...", candidate.chars().take(77).collect::<String>())
            } else {
                candidate.to_string()
            };
        }
    }
    DEFAULT_DESCRIPTION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_empty_plan_with_broken_closing_tag() {
        let result = extract_empty_command_plan_notice(
            "<COMMAND_PLAN>\n{\n  \"version\": 1,\n  \"summary\": \"当前只需要观察目录，不需要后续操作\",\n  \"steps\": []\n}\n</COMMAND_PLAN",
        )
        .unwrap();
        assert_eq!(result.display_content, "");
        assert_eq!(
            result.command_plan_notice,
            Some(CommandPlanNotice {
                version: 1,
                summary: "当前只需要观察目录，不需要后续操作".into()
            })
        );
    }

    #[test]
    fn strips_nonempty_plan_with_malformed_closing_tag() {
        let result = strip_command_plan_block(
            "<COMMAND_PLAN>\n{\n  \"version\": 1,\n  \"summary\": \"查看\",\n  \"steps\": [\n    {\"description\": \"列出\", \"command\": \"ls -la\", \"intent\": \"observe\"}\n  ]\n}\n</COMMAND_PLAN",
        );
        assert_eq!(result, "");
    }

    #[test]
    fn strip_handles_streaming_partial_plan() {
        assert_eq!(
            strip_command_plan_block("好的\n<COMMAND_PLAN>\n{\"ver"),
            "好的"
        );
        assert_eq!(strip_command_plan_block("结果\n</COMMAND"), "结果");
    }

    #[test]
    fn ssh_keeps_only_first_action_block() {
        let result = parse_ssh_pending_actions(
            "\n[ACTION:检查服务]\nsystemctl status nginx\n[/ACTION]\n[ACTION:读取日志]\njournalctl -u nginx -n 50\n[/ACTION]\n",
        );
        assert_eq!(result.actions.len(), 1);
        assert_eq!(
            result.actions[0].command.as_deref(),
            Some("systemctl status nginx")
        );
        assert_eq!(result.actions[0].description, "检查服务");
        assert_eq!(result.display_content, "");
    }

    #[test]
    fn ssh_preserves_one_compound_shell_action() {
        let result = parse_ssh_pending_actions(
            "\n```bash\nsystemctl restart nginx && systemctl status nginx\njournalctl -u nginx -n 20 --no-pager\n```\n",
        );
        assert_eq!(result.actions.len(), 1);
        let cmd = result.actions[0].command.as_deref().unwrap();
        assert!(cmd.contains("&&"));
        assert!(cmd.contains("\njournalctl"));
        assert_eq!(result.actions[0].source, Some(ActionSource::Fence));
    }

    #[test]
    fn validated_plan_keeps_first_step_only() {
        let content = "说明\n<COMMAND_PLAN>\n{\"version\":1,\"summary\":\"依次检查\",\"steps\":[{\"description\":\"检查服务\",\"command\":\"systemctl status nginx\",\"intent\":\"observe\",\"expectedOutcome\":\"状态\"},{\"description\":\"读日志\",\"command\":\"journalctl -u nginx -n 50\",\"intent\":\"observe\",\"expectedOutcome\":\"日志\"}]}\n</COMMAND_PLAN>";
        let all = parse_ai_pending_actions_with_plan(content, false);
        assert_eq!(all.actions.len(), 2);
        assert_eq!(all.display_content, "说明");
        let one = parse_ai_pending_actions_with_plan(content, true);
        assert_eq!(one.actions.len(), 1);
        assert_eq!(
            one.actions[0].command.as_deref(),
            Some("systemctl status nginx")
        );
        assert_eq!(one.actions[0].source, Some(ActionSource::CommandPlan));
        assert_eq!(one.actions[0].expected_outcome.as_deref(), Some("状态"));
    }

    #[test]
    fn invalid_plan_falls_back_to_fences() {
        let content = "<COMMAND_PLAN>{\"version\":1,\"summary\":\"x\",\"steps\":[{\"command\":\"first\"}]}</COMMAND_PLAN>\n```bash\nls -la\n```";
        let result = parse_ai_pending_actions_with_plan(content, true);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].command.as_deref(), Some("ls -la"));
    }

    #[test]
    fn json_fence_plan_is_recognised_and_removed() {
        let content = "先看看\n```json\n{\"version\":1,\"summary\":\"s\",\"steps\":[{\"description\":\"d\",\"command\":\"uptime\",\"intent\":\"observe\",\"expectedOutcome\":\"e\"}]}\n```\n";
        let result = parse_ai_pending_actions_with_plan(content, true);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].command.as_deref(), Some("uptime"));
        assert_eq!(result.display_content, "先看看");
    }

    #[test]
    fn truncated_plan_json_is_repaired_to_empty_notice() {
        let content = "<COMMAND_PLAN>{\"version\":1,\"summary\":\"无需操作\",\"steps\":[]";
        let result = extract_empty_command_plan_notice(content).unwrap();
        assert_eq!(result.command_plan_notice.unwrap().summary, "无需操作");
    }

    #[test]
    fn empty_notice_requires_valid_shape() {
        assert!(extract_empty_command_plan_notice(
            "<COMMAND_PLAN>{\"version\":2,\"summary\":\"x\",\"steps\":[]}</COMMAND_PLAN>"
        )
        .is_none());
        assert!(extract_empty_command_plan_notice(
            "<COMMAND_PLAN>{\"version\":1,\"summary\":\"\",\"steps\":[]}</COMMAND_PLAN>"
        )
        .is_none());
    }

    #[test]
    fn dedupes_and_describes_fence_commands() {
        let content = "### 查看磁盘：\n```bash\ndf -h\n```\n再来一次\n```sh\ndf   -h\n```";
        let result = parse_ai_pending_actions(content);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].description, "查看磁盘");
    }

    #[test]
    fn terminal_transcripts_use_prompted_lines_and_comments_drop() {
        let content = "```console\nroot@host:~# ls /etc\nfile1\n$ uname -a\n```\n```bash\n# just a comment\n```";
        let result = parse_ai_pending_actions(content);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(
            result.actions[0].command.as_deref(),
            Some("ls /etc\nuname -a")
        );
    }

    #[test]
    fn untagged_fence_requires_shell_like_content() {
        let yaml = "```\nname: demo\nversion: 1\n```";
        assert!(parse_ai_pending_actions(yaml).actions.is_empty());
        let json = "```json\n{\"a\": 1}\n```";
        assert!(parse_ai_pending_actions(json).actions.is_empty());
        let shell = "```\nls -la | grep foo\n```";
        assert_eq!(parse_ai_pending_actions(shell).actions.len(), 1);
        let python = "```python\nprint('x')\n```";
        assert!(parse_ai_pending_actions(python).actions.is_empty());
    }

    #[test]
    fn action_tag_with_inline_command() {
        let result = parse_ai_pending_actions("[ACTION:查看时间:date][/ACTION]");
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].description, "查看时间");
        assert_eq!(result.actions[0].command.as_deref(), Some("date"));
    }

    #[test]
    fn long_descriptions_are_truncated() {
        let long = "x".repeat(120);
        let content = format!("{long}\n```bash\nls\n```");
        let result = parse_ai_pending_actions(&content);
        let d = &result.actions[0].description;
        assert_eq!(d.chars().count(), 80);
        assert!(d.ends_with("..."));
    }

    #[test]
    fn tilde_and_crlf_fences() {
        let content = "说明\r\n~~~bash\r\nwhoami\r\n~~~\r\n";
        let result = parse_ai_pending_actions(content);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].command.as_deref(), Some("whoami"));
    }

    #[test]
    fn unterminated_fence_is_ignored() {
        let result = parse_ai_pending_actions("```bash\nls -la");
        assert!(result.actions.is_empty());
    }
}
