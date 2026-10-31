//! Port of the RDP half of `aiActionParser.ts`: turns an `<RDP_PLAN>` reply into pending remote-desktop actions.
//! Models often emit sloppy JSON (JS object literals, smart quotes, unescaped quotes, truncated output), so the
//! plan goes through a lenient parser before validation.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::parser::{
    extract_fence_blocks, find_fence_end, slice_balanced_json, ActionKind, ActionSource,
    ParsedPendingAction, RdpOperation,
};

const RDP_PLAN_MAX_STEPS: usize = 12;
const RDP_PLAN_MAX_OPS_PER_STEP: usize = 30;
const DIAGNOSTIC_PREVIEW_CHARS: usize = 800;
const DEFAULT_WAIT_MS: f64 = 1_500.0;
const MIN_WAIT_MS: f64 = 250.0;
const MAX_WAIT_MS: f64 = 30_000.0;

const PLAN_FENCE_LANGUAGES: &[&str] = &["rdp", "rdp-plan", "rdp_actions", "rdp-ops"];
const GENERIC_FENCE_LANGUAGES: &[&str] = &["", "json", "jsonc", "json5", "js", "javascript"];
const ALLOWED_INTENTS: &[&str] = &[
    "click", "type", "key", "scroll", "navigate", "launch", "operate", "interact", "wait",
];
const OPERATION_WRAPPER_KEYS: &[&str] = &[
    "operations",
    "operation",
    "actions",
    "ops",
    "items",
    "events",
];
const SUPPORTED_KEY_NAMES: &[&str] = &[
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Space",
    "Shift",
    "Control",
    "Alt",
    "Meta",
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
    "CapsLock",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "Win",
    "Windows",
    "Ctrl",
    "AltGr",
];

static TAGGED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<RDP_PLAN>\s*(.*?)\s*</RDP_PLAN>").unwrap());
static OPEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<RDP_PLAN\s*>?").unwrap());
static CLOSE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</RDP_PLAN?>?").unwrap());
static TRAILING_COMMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s),\s*([}\]])").unwrap());
static STRIP_FULL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<RDP_PLAN\s*>?.*?</RDP_PLAN?>?").unwrap());
static STRIP_OPEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<RDP_PLAN\s*>?.*$").unwrap());
static STRIP_CLOSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</RDP(?:_PLAN?)?>?\s*$").unwrap());
static STRIP_OPEN_FENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:```|~~~)[ \t]*(?:json[c5]?|javascript|js)?[ \t]*\r?\n\s*\{[^`]*"steps".*$"#,
    )
    .unwrap()
});
static KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z_$][A-Za-z0-9_$-]*)(\s*:)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Tag,
    OpenTagBalancedJson,
    Fence,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Tag => "tag",
            Source::OpenTagBalancedJson => "open-tag-balanced-json",
            Source::Fence => "fence",
        }
    }
}

struct Candidate {
    raw_plan: String,
    start: usize,
    end: usize,
    source: Source,
    extract_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatedRdpPlanStep {
    pub id: String,
    pub description: String,
    pub operations: Vec<RdpOperation>,
    pub intent: String,
    pub expected_outcome: String,
}

#[derive(Debug, Clone)]
pub struct ValidatedRdpPlan {
    pub summary: String,
    pub steps: Vec<ValidatedRdpPlanStep>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedRdpActions {
    pub display_content: String,
    pub actions: Vec<ParsedPendingAction>,
    pub parse_error: Option<String>,
    pub parse_diagnostic: Option<String>,
}

// ---------------------------------------------------------------------------
// JS-like coercions (the original used `Number(x)`, `String(x)`, `Boolean(x)`)
// ---------------------------------------------------------------------------

fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(_) => f64::NAN,
    }
}

fn js_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `a ?? b ?? ...` over object keys (null and missing both fall through).
fn first_present<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|k| obj.get(*k))
        .find(|v| !v.is_null())
}

// ---------------------------------------------------------------------------
// Candidate extraction
// ---------------------------------------------------------------------------

fn extract_candidate(content: &str) -> Option<Candidate> {
    if let Some(caps) = TAGGED_RE.captures(content) {
        let whole = caps.get(0).unwrap();
        return Some(Candidate {
            raw_plan: caps.get(1).unwrap().as_str().trim().to_string(),
            start: whole.start(),
            end: whole.end(),
            source: Source::Tag,
            extract_error: None,
        });
    }

    if let Some(open) = OPEN_RE.find(content) {
        if let Some((text, json_end)) = slice_balanced_json(content, open.end()) {
            let end = match CLOSE_RE.find(&content[json_end..]) {
                Some(close) => json_end + close.end(),
                None => json_end,
            };
            return Some(Candidate {
                raw_plan: text.trim().to_string(),
                start: open.start(),
                end,
                source: Source::OpenTagBalancedJson,
                extract_error: None,
            });
        }
        return Some(Candidate {
            raw_plan: String::new(),
            start: open.start(),
            end: content.len(),
            source: Source::OpenTagBalancedJson,
            extract_error: Some(
                "检测到 <RDP_PLAN> 标签但无法提取有效 JSON（可能格式错误或被截断）".into(),
            ),
        });
    }

    // Models frequently skip the tag and answer with a plain ```json block (or an untagged one), so every fence whose
    // body is a plan-shaped object is accepted; the explicit rdp* languages stay accepted as before.
    for block in extract_fence_blocks(content) {
        let lang = block.language.trim().to_ascii_lowercase();
        if !PLAN_FENCE_LANGUAGES.contains(&lang.as_str())
            && !GENERIC_FENCE_LANGUAGES.contains(&lang.as_str())
        {
            continue;
        }
        if !looks_like_plan(&block.code) {
            continue;
        }
        return Some(Candidate {
            raw_plan: block.code.trim().to_string(),
            start: block.start_index,
            end: find_fence_end(content, block.start_index),
            source: Source::Fence,
            extract_error: None,
        });
    }

    // Last resort: an unfenced JSON object that carries a `steps` array.
    for (at, _) in content.match_indices('{') {
        let Some((text, end)) = slice_balanced_json(content, at) else {
            continue;
        };
        if looks_like_plan(&text) {
            return Some(Candidate {
                raw_plan: text.trim().to_string(),
                start: at,
                end,
                source: Source::Fence,
                extract_error: None,
            });
        }
        // a failed object may still contain the plan further in; keep scanning only past this object
        if end > at {
            break;
        }
    }
    None
}

/// A plan-shaped body: an object with a `steps` array, tolerating the same sloppy JSON as the tagged form.
fn looks_like_plan(code: &str) -> bool {
    if !code.contains("steps") {
        return false;
    }
    let has_steps = |v: &Value| v.get("steps").is_some_and(Value::is_array);
    serde_json::from_str::<Value>(code.trim())
        .map(|v| has_steps(&v))
        .unwrap_or(false)
        || plan_json_candidates(code)
            .iter()
            .any(|c| parse_plan_json_loose(c).is_some_and(|v| has_steps(&v)))
}

// ---------------------------------------------------------------------------
// Lenient JSON
// ---------------------------------------------------------------------------

fn plan_json_candidates(raw: &str) -> Vec<String> {
    let mut list = vec![raw.trim().to_string()];
    if let Some((text, _)) = slice_balanced_json(raw, 0) {
        let t = text.trim().to_string();
        if !t.is_empty() {
            list.insert(0, t);
        }
    }
    dedupe(list.into_iter().filter(|c| !c.is_empty()).collect())
}

fn dedupe(list: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    list.into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

fn strip_json_like_comments(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();
        if let Some(q) = in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == q {
                in_string = None;
            }
            i += 1;
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_string = Some(ch);
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\r' && chars[i] != '\n' {
                i += 1;
            }
            if let Some(c) = chars.get(i) {
                out.push(*c);
            }
            i += 1;
            continue;
        }
        if ch == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn normalize_single_quoted_strings(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_double = false;
    let mut escape = false;
    while i < chars.len() {
        let ch = chars[i];
        if in_double {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch != '\'' {
            out.push(ch);
            i += 1;
            continue;
        }
        i += 1;
        let mut value = String::new();
        let mut closed = false;
        while i < chars.len() {
            let inner = chars[i];
            if inner == '\\' && i + 1 < chars.len() {
                value.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if inner == '\'' {
                closed = true;
                i += 1;
                break;
            }
            value.push(inner);
            i += 1;
        }
        out.push_str(&serde_json::to_string(&value).unwrap_or_default());
        if !closed {
            break;
        }
    }
    out
}

fn quote_unquoted_object_keys(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut expecting_key = false;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == q {
                in_string = None;
            }
            i += 1;
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_string = Some(ch);
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '{' || ch == ',' {
            expecting_key = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '}' || ch == '[' || ch == ']' {
            expecting_key = false;
            out.push(ch);
            i += 1;
            continue;
        }
        if expecting_key && ch.is_whitespace() {
            out.push(ch);
            i += 1;
            continue;
        }
        if expecting_key && (ch.is_ascii_alphabetic() || ch == '_' || ch == '$') {
            let rest: String = chars[i..].iter().collect();
            if let Some(caps) = KEY_RE.captures(&rest) {
                let whole = caps.get(0).unwrap().as_str();
                out.push_str(&serde_json::to_string(&caps[1]).unwrap_or_default());
                out.push_str(&caps[2]);
                i += whole.chars().count();
                expecting_key = false;
                continue;
            }
        }
        expecting_key = false;
        out.push(ch);
        i += 1;
    }
    out
}

fn normalize_loose_json(raw: &str) -> String {
    let mut result: String = raw
        .chars()
        .map(|c| match c {
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2018}' | '\u{2019}' => '\'',
            other => other,
        })
        .collect();
    result = strip_json_like_comments(&result);
    result = quote_unquoted_object_keys(&result);
    result = normalize_single_quoted_strings(&result);
    loop {
        let next = TRAILING_COMMA_RE.replace_all(&result, "$1").into_owned();
        if next == result {
            break;
        }
        result = next;
    }
    result
}

/// Escape quotes that sit inside a string value: a quote only closes a string when what follows looks structural.
fn rebuild_json_structure(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(raw.len() + 8);
    let mut i = 0;
    while i < len {
        let ch = chars[i];
        if ch != '"' {
            out.push(ch);
            i += 1;
            continue;
        }
        out.push('"');
        i += 1;
        let mut closed = false;
        while i < len {
            let inner = chars[i];
            if inner == '\\' && i + 1 < len {
                out.push(inner);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if inner != '"' {
                out.push(inner);
                i += 1;
                continue;
            }
            let structural = i + 1 >= len
                || matches!(chars[i + 1], ',' | '}' | ']' | ':')
                || chars[i + 1].is_whitespace();
            if structural {
                out.push('"');
                i += 1;
                closed = true;
                break;
            }
            out.push_str("\\\"");
            i += 1;
        }
        if !closed {
            out.push('"');
        }
    }
    out
}

fn parse_plan_json_loose(raw: &str) -> Option<Value> {
    for candidate in plan_json_candidates(raw) {
        for parse_candidate in dedupe(vec![candidate.clone(), normalize_loose_json(&candidate)]) {
            if let Ok(v) = serde_json::from_str(&parse_candidate) {
                return Some(v);
            }
            let repaired = rebuild_json_structure(&parse_candidate);
            for r in dedupe(vec![repaired.clone(), normalize_loose_json(&repaired)]) {
                if let Ok(v) = serde_json::from_str(&r) {
                    return Some(v);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn parse_json_like_operations(value: &str) -> Option<Value> {
    let t = value.trim();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    serde_json::from_str(t).ok()
}

fn is_record_with_type(v: &Value) -> bool {
    v.is_object() && v.get("type").is_some_and(Value::is_string)
}

fn unwrap_operations(raw: &Value, depth: u32) -> Option<Vec<Value>> {
    if let Value::Array(a) = raw {
        return Some(a.clone());
    }
    if depth > 4 {
        return None;
    }
    if let Value::String(s) = raw {
        return parse_json_like_operations(s).and_then(|p| unwrap_operations(&p, depth + 1));
    }
    let obj = raw.as_object()?;
    if raw.get("type").is_some_and(Value::is_string) {
        return Some(vec![raw.clone()]);
    }
    for key in OPERATION_WRAPPER_KEYS {
        if let Some(inner) = obj.get(*key) {
            if let Some(list) = unwrap_operations(inner, depth + 1) {
                return Some(list);
            }
        }
    }
    let mut indexed: Vec<(u64, &Value)> = obj
        .iter()
        .filter(|(k, _)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|(k, v)| k.parse().ok().map(|n| (n, v)))
        .collect();
    if !indexed.is_empty() {
        indexed.sort_by_key(|(n, _)| *n);
        return Some(indexed.into_iter().map(|(_, v)| v.clone()).collect());
    }
    let values: Vec<&Value> = obj.values().collect();
    if !values.is_empty() && values.iter().all(|v| is_record_with_type(v)) {
        return Some(values.into_iter().cloned().collect());
    }
    None
}

fn normalize_wait_ms(v: Option<&Value>) -> Result<u32, String> {
    let n = if v.is_none_or(Value::is_null) {
        DEFAULT_WAIT_MS
    } else {
        js_number(v).floor()
    };
    if !n.is_finite() {
        return Err("RDP wait ms 必须是数字".into());
    }
    Ok(n.clamp(MIN_WAIT_MS, MAX_WAIT_MS) as u32)
}

fn wait_ms_of(op: &Value) -> Result<u32, String> {
    normalize_wait_ms(first_present(op, &["ms", "durationMs", "duration"]))
}

fn clamp_coord(v: Option<&Value>, max: u32) -> Result<u16, String> {
    let n = js_number(v).floor();
    if !n.is_finite() {
        return Err("RDP 坐标必须是数字".into());
    }
    Ok(n.clamp(0.0, f64::from(max.saturating_sub(1))) as u16)
}

fn button_of(op: &Value) -> Result<f64, String> {
    Ok(if op.get("button").is_none() {
        0.0
    } else {
        js_number(op.get("button")).floor()
    })
}

fn validate_operations(raw: Option<&Value>, max: (u32, u32)) -> Result<Vec<RdpOperation>, String> {
    let raw = raw.ok_or_else(|| "RDP operations 必须是数组或单个操作对象".to_string())?;
    let ops = unwrap_operations(raw, 0)
        .ok_or_else(|| "RDP operations 必须是数组或单个操作对象".to_string())?;
    if ops.is_empty() {
        return Err("RDP operations 不能为空".into());
    }
    if ops.len() > RDP_PLAN_MAX_OPS_PER_STEP {
        return Err(format!(
            "RDP operations 单步不能超过 {RDP_PLAN_MAX_OPS_PER_STEP} 个操作"
        ));
    }

    let mut result = Vec::new();
    for raw in &ops {
        if !raw.is_object() {
            return Err("RDP operation 必须是对象".into());
        }
        let kind = raw.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "click" | "move" => {
                let x = clamp_coord(raw.get("x"), max.0)?;
                let y = clamp_coord(raw.get("y"), max.1)?;
                if kind == "click" {
                    let b = button_of(raw)?;
                    if ![0.0, 1.0, 2.0].contains(&b) {
                        return Err(format!("RDP click button 不支持: {b}"));
                    }
                    result.push(RdpOperation::Click {
                        x,
                        y,
                        button: b as u8,
                        double_click: js_truthy(raw.get("doubleClick")),
                    });
                } else {
                    result.push(RdpOperation::Move { x, y });
                }
            }
            "drag" => {
                let from_x = clamp_coord(first_present(raw, &["fromX", "from_x"]), max.0)?;
                let from_y = clamp_coord(first_present(raw, &["fromY", "from_y"]), max.1)?;
                let to_x = clamp_coord(first_present(raw, &["toX", "to_x"]), max.0)?;
                let to_y = clamp_coord(first_present(raw, &["toY", "to_y"]), max.1)?;
                let b = button_of(raw)?;
                if ![0.0, 2.0].contains(&b) {
                    return Err(format!("RDP drag button 不支持: {b}"));
                }
                result.push(RdpOperation::Drag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    button: b as u8,
                });
            }
            "scroll" => {
                let x = clamp_coord(raw.get("x"), max.0)?;
                let y = clamp_coord(raw.get("y"), max.1)?;
                let delta = js_number(raw.get("delta")).floor();
                if !delta.is_finite() || delta == 0.0 {
                    return Err("RDP scroll delta 必须是非零整数".into());
                }
                if delta.abs() > 100.0 {
                    return Err("RDP scroll delta 不能超过 ±100".into());
                }
                let vertical = if raw.get("vertical").is_none() {
                    true
                } else {
                    js_truthy(raw.get("vertical"))
                };
                result.push(RdpOperation::Scroll {
                    x,
                    y,
                    delta: delta as i32,
                    vertical,
                });
            }
            "type" => {
                let text = js_string(raw.get("text"));
                if text.is_empty() {
                    return Err("RDP type text 不能为空".into());
                }
                if utf16_len(&text) > 500 {
                    return Err("RDP type text 不能超过 500 字符".into());
                }
                if text.contains('\0') {
                    return Err("RDP type text 包含非法字符".into());
                }
                result.push(RdpOperation::Type { text });
            }
            "wait" | "pause" | "sleep" => result.push(RdpOperation::Wait {
                ms: wait_ms_of(raw)?,
            }),
            "key" => {
                let keys: Vec<Value> = match raw.get("keys") {
                    Some(Value::Array(a)) => a.clone(),
                    Some(Value::String(s)) => vec![Value::String(s.clone())],
                    _ => Vec::new(),
                };
                if keys.is_empty() {
                    return Err("RDP key keys 不能为空".into());
                }
                if keys.len() > 6 {
                    return Err("RDP key 组合键不能超过 6 个".into());
                }
                if keys.len() == 1 && js_string(keys.first()).trim().eq_ignore_ascii_case("wait") {
                    result.push(RdpOperation::Wait {
                        ms: wait_ms_of(raw)?,
                    });
                    continue;
                }
                let mut names = Vec::new();
                for k in &keys {
                    let name = js_string(Some(k));
                    if utf16_len(&name) == 1 || SUPPORTED_KEY_NAMES.contains(&name.as_str()) {
                        names.push(name);
                    } else {
                        return Err(format!("RDP 不支持的按键: {name}"));
                    }
                }
                result.push(RdpOperation::Key { keys: names });
            }
            other => {
                let shown = if raw.get("type").is_none() {
                    "undefined".to_string()
                } else {
                    other.to_string()
                };
                return Err(format!("RDP operation type 不支持: {shown}"));
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Plan validation
// ---------------------------------------------------------------------------

fn raw_steps(plan: &Value, summary: &str) -> Vec<Value> {
    if let Some(Value::Array(steps)) = plan.get("steps") {
        return steps.clone();
    }
    let Some(ops) = first_present(plan, &["operations", "operation", "actions", "ops"]) else {
        return Vec::new();
    };
    let mut step = serde_json::Map::new();
    let description = first_present(plan, &["description"])
        .cloned()
        .unwrap_or_else(|| {
            Value::String(if summary.is_empty() {
                "RDP 操作".into()
            } else {
                summary.into()
            })
        });
    step.insert("description".into(), description);
    step.insert("operations".into(), ops.clone());
    step.insert(
        "intent".into(),
        first_present(plan, &["intent"])
            .cloned()
            .unwrap_or_else(|| Value::String("operate".into())),
    );
    if let Some(e) = first_present(plan, &["expected_outcome", "expectedOutcome"]) {
        step.insert("expected_outcome".into(), e.clone());
    }
    vec![Value::Object(step)]
}

pub fn validate_rdp_plan(raw: &str, max: (u32, u32)) -> Result<ValidatedRdpPlan, String> {
    let plan = parse_plan_json_loose(raw).ok_or_else(|| "RdpPlan JSON 解析失败".to_string())?;
    let version = if plan.get("version").is_none_or(Value::is_null) {
        1.0
    } else {
        js_number(plan.get("version"))
    };
    if version != 1.0 {
        return Err(format!(
            "RdpPlan version 必须为 1，实际为 {}",
            js_string(plan.get("version"))
        ));
    }

    let mut summary = js_string(plan.get("summary")).trim().to_string();
    let steps_raw = raw_steps(&plan, &summary);
    if summary.is_empty() && !steps_raw.is_empty() {
        let first = steps_raw
            .first()
            .and_then(|s| s.get("description"))
            .filter(|v| !v.is_null())
            .or_else(|| plan.get("description"));
        summary = js_string(first).trim().chars().take(120).collect();
        if summary.is_empty() {
            summary = "RDP 操作".into();
        }
    }
    if summary.is_empty() {
        return Err("RdpPlan summary 和 steps 均为空，无法解析操作计划".into());
    }
    if utf16_len(&summary) > 200 {
        return Err("RdpPlan summary 不能超过 200 字符".into());
    }
    if steps_raw.is_empty() {
        return Err("RdpPlan steps 必须包含 1 到 12 个步骤".into());
    }
    if steps_raw.len() > RDP_PLAN_MAX_STEPS {
        return Err(format!(
            "RdpPlan steps 不能超过 {RDP_PLAN_MAX_STEPS} 个步骤"
        ));
    }

    let mut steps = Vec::new();
    for step in &steps_raw {
        let description = js_string(step.get("description")).trim().to_string();
        if description.is_empty() {
            return Err("RdpPlan step description 不能为空".into());
        }
        if utf16_len(&description) > 120 {
            return Err("RdpPlan step description 不能超过 120 字符".into());
        }
        let operations = validate_operations(
            first_present(step, &["operations", "operation", "actions", "ops"]),
            max,
        )?;
        let intent = if step.get("intent").is_none_or(Value::is_null) {
            "operate".to_string()
        } else {
            js_string(step.get("intent")).trim().to_lowercase()
        };
        if !ALLOWED_INTENTS.contains(&intent.as_str()) {
            return Err(format!("RdpPlan step intent 不支持: {intent}"));
        }
        let expected_outcome: String = js_string(step.get("expected_outcome"))
            .trim()
            .chars()
            .take(240)
            .collect();
        steps.push(ValidatedRdpPlanStep {
            id: uuid::Uuid::new_v4().to_string(),
            description,
            operations,
            intent,
            expected_outcome,
        });
    }
    Ok(ValidatedRdpPlan { summary, steps })
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

fn diagnostic_preview(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\t', "  ");
    let normalized = normalized.trim();
    if normalized.chars().count() <= DIAGNOSTIC_PREVIEW_CHARS {
        return normalized.to_string();
    }
    format!(
        "{}...",
        normalized
            .chars()
            .take(DIAGNOSTIC_PREVIEW_CHARS)
            .collect::<String>()
    )
}

fn json_state(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(_) => "ok".into(),
        Err(e) => e.to_string(),
    }
}

fn plan_json_diagnostic(raw: &str) -> String {
    let mut lines = vec![
        format!("rawLength={}", utf16_len(raw)),
        format!("rawPreview={}", diagnostic_preview(raw)),
    ];
    let candidates = plan_json_candidates(raw);
    lines.push(format!("candidateCount={}", candidates.len()));
    for (idx, candidate) in candidates.iter().enumerate() {
        lines.push(format!("candidate[{idx}].length={}", utf16_len(candidate)));
        lines.push(format!(
            "candidate[{idx}].preview={}",
            diagnostic_preview(candidate)
        ));
        lines.push(format!(
            "candidate[{idx}].JSON.parse={}",
            json_state(candidate)
        ));
        let repaired = rebuild_json_structure(candidate);
        let state = json_state(&repaired);
        lines.push(format!("candidate[{idx}].repair.JSON.parse={state}"));
        if state != "ok" {
            lines.push(format!(
                "candidate[{idx}].repair.preview={}",
                diagnostic_preview(&repaired)
            ));
        }
        let normalized = normalize_loose_json(candidate);
        if &normalized != candidate {
            let state = json_state(&normalized);
            lines.push(format!("candidate[{idx}].normalized.JSON.parse={state}"));
            if state != "ok" {
                lines.push(format!(
                    "candidate[{idx}].normalized.preview={}",
                    diagnostic_preview(&normalized)
                ));
            }
        }
    }
    lines.join("\n")
}

fn context_diagnostic(content: &str, c: &Candidate, max: (u32, u32)) -> String {
    [
        format!("contentLength={}", utf16_len(content)),
        format!("candidateSource={}", c.source.name()),
        format!("candidateRange={}..{}", c.start, c.end),
        format!("maxCoords={}x{}", max.0, max.1),
        format!(
            "candidatePreview={}",
            diagnostic_preview(&content[c.start..c.end.min(content.len())])
        ),
    ]
    .join("\n")
}

fn remove_range(content: &str, start: usize, end: usize) -> String {
    format!(
        "{}{}",
        &content[..start],
        &content[end.min(content.len())..]
    )
    .trim()
    .to_string()
}

/// Hide `<RDP_PLAN>` blocks (complete, or still streaming in) from the chat text.
pub fn strip_rdp_plan_block(content: &str) -> String {
    let r = STRIP_FULL_RE.replace_all(content, "");
    let r = STRIP_OPEN_RE.replace(&r, "");
    let r = STRIP_CLOSE_RE.replace(&r, "").trim().to_string();
    // an untagged plan (```json block or bare object) is hidden the same way once it is recognisable
    if let Some(c) = extract_candidate(&r).filter(|c| c.extract_error.is_none()) {
        return remove_range(&r, c.start, c.end);
    }
    // ... and while it is still streaming in: an unclosed plan-looking fence
    STRIP_OPEN_FENCE_RE.replace(&r, "").trim().to_string()
}

/// `None` when the reply contains no RDP plan at all.
pub fn parse_rdp_actions(content: &str, max: (u32, u32)) -> Option<ParsedRdpActions> {
    let candidate = extract_candidate(content)?;
    let display_content = remove_range(content, candidate.start, candidate.end);

    if let Some(err) = &candidate.extract_error {
        return Some(ParsedRdpActions {
            display_content,
            actions: Vec::new(),
            parse_error: Some(err.clone()),
            parse_diagnostic: Some(format!(
                "{}\nextractError={err}",
                context_diagnostic(content, &candidate, max)
            )),
        });
    }

    match validate_rdp_plan(&candidate.raw_plan, max) {
        Err(error) => {
            let detail = if error == "RdpPlan JSON 解析失败" {
                plan_json_diagnostic(&candidate.raw_plan)
            } else {
                format!(
                    "validationError={error}\nrawPlanLength={}\nrawPlanPreview={}",
                    utf16_len(&candidate.raw_plan),
                    diagnostic_preview(&candidate.raw_plan)
                )
            };
            Some(ParsedRdpActions {
                display_content,
                actions: Vec::new(),
                parse_error: Some(error),
                parse_diagnostic: Some(format!(
                    "{}\n{detail}",
                    context_diagnostic(content, &candidate, max)
                )),
            })
        }
        Ok(plan) => {
            let actions = plan
                .steps
                .into_iter()
                .map(|step| ParsedPendingAction {
                    id: step.id,
                    kind: ActionKind::RdpAction,
                    description: step.description,
                    command: None,
                    rdp_operations: Some(step.operations),
                    source: Some(ActionSource::RdpPlan),
                    intent: Some(step.intent),
                    expected_outcome: Some(step.expected_outcome),
                    assessment: None,
                    assessment_loading: false,
                    assessment_error: None,
                    approved: false,
                    executed: false,
                    rejected: false,
                })
                .collect();
            Some(ParsedRdpActions {
                display_content,
                actions,
                parse_error: None,
                parse_diagnostic: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RdpOperation::*;

    const MAX: (u32, u32) = (1280, 720);

    fn ops(content: &str) -> Vec<RdpOperation> {
        let r = parse_rdp_actions(content, MAX).expect("plan found");
        assert_eq!(r.parse_error, None, "{:?}", r.parse_diagnostic);
        r.actions[0].rdp_operations.clone().unwrap()
    }

    fn key(names: &[&str]) -> RdpOperation {
        Key {
            keys: names.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn typed(t: &str) -> RdpOperation {
        Type { text: t.into() }
    }

    #[test]
    fn single_operation_object() {
        let c = "<RDP_PLAN>\n{\"version\":1,\"summary\":\"打开开始菜单\",\"steps\":[{\"description\":\"按下 Windows 键\",\"intent\":\"key\",\"operations\":{\"type\":\"key\",\"keys\":[\"Meta\"]},\"expected_outcome\":\"开始菜单打开\"}]}\n</RDP_PLAN>";
        let r = parse_rdp_actions(c, MAX).unwrap();
        assert_eq!(r.actions.len(), 1);
        assert_eq!(
            r.actions[0].rdp_operations.as_ref().unwrap(),
            &vec![key(&["Meta"])]
        );
        assert_eq!(
            r.actions[0].expected_outcome.as_deref(),
            Some("开始菜单打开")
        );
    }

    #[test]
    fn wrapped_and_stringified_operations() {
        let c = r#"<RDP_PLAN>
{"version":1,"summary":"搜索记事本","steps":[{"description":"输入搜索内容","intent":"type","operations":{"operations":"[{\"type\":\"type\",\"text\":\"notepad\"},{\"type\":\"key\",\"keys\":[\"Enter\"]}]"},"expected_outcome":"记事本启动"}]}
</RDP_PLAN>"#;
        assert_eq!(ops(c), vec![typed("notepad"), key(&["Enter"])]);
    }

    #[test]
    fn wait_key_becomes_wait() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"等待","steps":[{"description":"等待下载","intent":"wait","operations":[{"type":"key","keys":["Wait"]}]}]}</RDP_PLAN>"#;
        assert_eq!(ops(c), vec![Wait { ms: 1500 }]);
    }

    #[test]
    fn numbered_operation_objects() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"打开浏览器","steps":[{"description":"打开 Edge","intent":"launch","operations":{"1":{"type":"key","keys":["Meta"]},"2":{"type":"type","text":"edge"},"10":{"type":"key","keys":["Enter"]}}}]}</RDP_PLAN>"#;
        assert_eq!(ops(c), vec![key(&["Meta"]), typed("edge"), key(&["Enter"])]);
    }

    #[test]
    fn fenced_json_inside_the_tag() {
        let c = "<RDP_PLAN>\n下面是下一步操作：\n```json\n{\"version\":1,\"summary\":\"打开浏览器\",\"steps\":[{\"description\":\"启动 Edge\",\"intent\":\"launch\",\"operations\":[{\"type\":\"key\",\"keys\":[\"Meta\"]},{\"type\":\"type\",\"text\":\"edge\"},{\"type\":\"key\",\"keys\":[\"Enter\"]}]}]}\n```\n</RDP_PLAN>";
        assert_eq!(ops(c), vec![key(&["Meta"]), typed("edge"), key(&["Enter"])]);
    }

    const JSON_FENCED_REPLY: &str = "好的，我来打开资源管理器。\n\n**第1步：** 按 Win+R\n**第2步：** 输入 explorer 并回车\n\n```json\n{\n  \"version\": 1,\n  \"summary\": \"打开资源管理器\",\n  \"steps\": [\n    {\"description\": \"按 Win+R 打开运行\", \"intent\": \"key\", \"operations\": [{\"type\": \"key\", \"keys\": [\"Meta\", \"r\"]}], \"expected_outcome\": \"运行对话框出现\"},\n    {\"description\": \"输入 explorer 并回车\", \"intent\": \"type\", \"operations\": [{\"type\": \"type\", \"text\": \"explorer\"}, {\"type\": \"key\", \"keys\": [\"Enter\"]}], \"expected_outcome\": \"资源管理器打开\"}\n  ]\n}\n```\n";

    #[test]
    fn plain_json_fence_is_a_plan() {
        let r = parse_rdp_actions(JSON_FENCED_REPLY, MAX).expect("plan found");
        assert_eq!(r.parse_error, None, "{:?}", r.parse_diagnostic);
        assert_eq!(r.actions.len(), 2);
        assert_eq!(
            r.actions[0].rdp_operations.as_ref().unwrap(),
            &vec![key(&["Meta", "r"])]
        );
        assert_eq!(
            r.actions[1].rdp_operations.as_ref().unwrap(),
            &vec![typed("explorer"), key(&["Enter"])]
        );
        assert!(
            !r.display_content.contains("steps"),
            "{}",
            r.display_content
        );
        assert!(r.display_content.contains("第1步"));
        assert!(strip_rdp_plan_block(JSON_FENCED_REPLY).contains("第2步"));
        assert!(!strip_rdp_plan_block(JSON_FENCED_REPLY).contains("\"steps\""));
    }

    #[test]
    fn untagged_fence_and_bare_json_are_plans_too() {
        let body = r#"{"version":1,"summary":"s","steps":[{"description":"d","intent":"key","operations":[{"type":"key","keys":["Enter"]}]}]}"#;
        assert_eq!(
            ops(&format!("先说明\n```\n{body}\n```")),
            vec![key(&["Enter"])]
        );
        assert_eq!(ops(&format!("先说明\n{body}\n完毕")), vec![key(&["Enter"])]);
        // sloppy JSON in a fence (trailing comma, single quotes, unquoted keys)
        let sloppy = "```json\n{version:1,summary:'s',steps:[{description:'d',intent:'key',operations:[{type:'key',keys:['Enter'],},],},],}\n```";
        assert_eq!(ops(sloppy), vec![key(&["Enter"])]);
        // unrelated json is left alone
        assert!(parse_rdp_actions("```json\n{\"a\":1}\n```", MAX).is_none());
        assert!(parse_rdp_actions("没有计划 {\"steps\": 3}", MAX).is_none());
    }

    #[test]
    fn streaming_plain_fence_is_hidden_while_incomplete() {
        let s = strip_rdp_plan_block(
            "好的\n```json\n{\"version\":1,\"summary\":\"x\",\"steps\":[{\"descr",
        );
        assert_eq!(s, "好的");
    }

    #[test]
    fn unparsable_json_reports_diagnostics() {
        let c = "<RDP_PLAN>\n{\n  \"version\": 1,\n  \"summary\": \"坏 JSON\",\n  \"steps\": [\n    {\n      \"description\": \"缺少逗号\"\n      \"intent\": \"launch\",\n      \"operations\": [{ \"type\": \"key\", \"keys\": [\"Meta\"] }]\n    }\n  ]\n}\n</RDP_PLAN>";
        let r = parse_rdp_actions(c, MAX).unwrap();
        assert_eq!(r.parse_error.as_deref(), Some("RdpPlan JSON 解析失败"));
        assert!(r.actions.is_empty());
        let d = r.parse_diagnostic.unwrap();
        for needle in [
            "contentLength=",
            "candidateSource=",
            "candidateRange=",
            "maxCoords=1280x720",
            "rawLength=",
            "candidate[0]",
            "JSON.parse",
            "缺少逗号",
        ] {
            assert!(d.contains(needle), "{needle} missing in {d}");
        }
        assert_eq!(r.display_content, "");
    }

    #[test]
    fn javascript_like_output() {
        let c = "<RDP_PLAN>\n{\n  // 模型有时会输出 JS 对象而不是严格 JSON\n  version: 1,\n  summary: '打开 Edge 搜索 Telegram',\n  steps: [\n    {\n      description: '打开 Edge 浏览器',\n      intent: 'launch',\n      operations: [\n        { type: 'key', keys: ['Meta'] },\n        { type: 'type', text: 'edge' },\n        { type: 'key', keys: ['Enter'] },\n      ],\n      expected_outcome: 'Edge 浏览器打开',\n    },\n  ],\n}\n</RDP_PLAN>";
        assert_eq!(ops(c), vec![key(&["Meta"]), typed("edge"), key(&["Enter"])]);
    }

    #[test]
    fn malformed_closing_tag_and_streaming_tail() {
        let c = "<RDP_PLAN>\n{\"version\":1,\"summary\":\"启动 Edge\",\"steps\":[{\"description\":\"双击图标\",\"intent\":\"click\",\"operations\":[{\"type\":\"click\",\"x\":26,\"y\":218,\"doubleClick\":true},{\"type\":\"wait\",\"ms\":3000}],\"expected_outcome\":\"打开\"}]}\n</RDP_PLAN\"}]}]}";
        assert_eq!(
            ops(c),
            vec![
                Click {
                    x: 26,
                    y: 218,
                    button: 0,
                    double_click: true
                },
                Wait { ms: 3000 }
            ]
        );
    }

    #[test]
    fn top_level_operations_become_one_step() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"打开浏览器","description":"通过开始菜单启动 Edge","intent":"launch","operations":[{"type":"key","keys":["Meta"]},{"type":"type","text":"edge"},{"type":"key","keys":["Enter"]}],"expected_outcome":"Edge 浏览器打开"}</RDP_PLAN>"#;
        let r = parse_rdp_actions(c, MAX).unwrap();
        assert_eq!(r.parse_error, None);
        assert_eq!(r.actions.len(), 1);
        assert_eq!(r.actions[0].description, "通过开始菜单启动 Edge");
        assert_eq!(r.actions[0].intent.as_deref(), Some("launch"));
        assert_eq!(r.actions[0].rdp_operations.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn trailing_commas_and_smart_quotes_are_repaired() {
        let commas = "<RDP_PLAN>{\"version\":1,\"summary\":\"s\",\"steps\":[{\"description\":\"d\",\"intent\":\"launch\",\"operations\":[{\"type\":\"key\",\"keys\":[\"Meta\"]},],\"expected_outcome\":\"e\",},],}</RDP_PLAN>";
        assert_eq!(ops(commas), vec![key(&["Meta"])]);
        let smart = "<RDP_PLAN>\n{\n  “version”: 1,\n  “summary”: “搜索下载页”,\n  “steps”: [\n    {\n      “description”: “输入地址”,\n      “intent”: “navigate”,\n      “operations”: [\n        { “type”: “type”, “text”: “https://desktop.telegram.org/” },\n        { “type”: “key”, “keys”: [“Enter”] }\n      ]\n    }\n  ]\n}\n</RDP_PLAN>";
        assert_eq!(
            ops(smart),
            vec![typed("https://desktop.telegram.org/"), key(&["Enter"])]
        );
    }

    #[test]
    fn coordinates_are_clamped_to_the_desktop() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"点击","steps":[{"description":"点击估算位置","intent":"click","operations":[{"type":"click","x":1320,"y":960},{"type":"drag","fromX":-12,"fromY":900,"toX":1400,"toY":-30}]}]}</RDP_PLAN>"#;
        assert_eq!(
            ops(c),
            vec![
                Click {
                    x: 1279,
                    y: 719,
                    button: 0,
                    double_click: false
                },
                Drag {
                    from_x: 0,
                    from_y: 719,
                    to_x: 1279,
                    to_y: 0,
                    button: 0
                }
            ]
        );
    }

    #[test]
    fn validation_errors_are_reported_not_panicked() {
        let bad = |ops: &str| {
            let c = format!(
                r#"<RDP_PLAN>{{"version":1,"summary":"s","steps":[{{"description":"d","intent":"click","operations":{ops}}}]}}</RDP_PLAN>"#
            );
            parse_rdp_actions(&c, MAX).unwrap().parse_error.unwrap()
        };
        assert!(bad(r#"[{"type":"key","keys":["Banana"]}]"#).contains("不支持的按键: Banana"));
        assert!(bad(r#"[{"type":"scroll","x":1,"y":1,"delta":0}]"#).contains("非零整数"));
        assert!(bad(r#"[{"type":"scroll","x":1,"y":1,"delta":500}]"#).contains("±100"));
        assert!(bad(r#"[{"type":"click","x":1,"y":1,"button":7}]"#).contains("button 不支持"));
        assert!(bad(r#"[{"type":"teleport"}]"#).contains("type 不支持: teleport"));
        assert!(bad(r#"[]"#).contains("不能为空"));
        assert!(bad(r#"[{"type":"type","text":""}]"#).contains("text 不能为空"));
        let many = format!("[{}]", vec![r#"{"type":"move","x":1,"y":1}"#; 31].join(","));
        assert!(bad(&many).contains("不能超过 30"));
    }

    #[test]
    fn bad_version_intent_and_wait_clamp() {
        let r = parse_rdp_actions(
            r#"<RDP_PLAN>{"version":2,"summary":"s","steps":[]}</RDP_PLAN>"#,
            MAX,
        )
        .unwrap();
        assert!(r
            .parse_error
            .unwrap()
            .contains("version 必须为 1，实际为 2"));
        let r = parse_rdp_actions(r#"<RDP_PLAN>{"summary":"s","steps":[{"description":"d","intent":"dance","operations":[{"type":"wait","ms":10}]}]}</RDP_PLAN>"#, MAX).unwrap();
        assert!(r.parse_error.unwrap().contains("intent 不支持: dance"));
        assert_eq!(
            ops(
                r#"<RDP_PLAN>{"summary":"s","steps":[{"description":"d","operations":[{"type":"wait","ms":10},{"type":"pause","ms":99999},{"type":"sleep"}]}]}</RDP_PLAN>"#
            ),
            vec![Wait { ms: 250 }, Wait { ms: 30000 }, Wait { ms: 1500 }]
        );
    }

    #[test]
    fn unterminated_tag_without_json_is_an_extract_error() {
        let r = parse_rdp_actions("好的 <RDP_PLAN> 我先想想", MAX).unwrap();
        assert!(r.parse_error.unwrap().contains("无法提取有效 JSON"));
        assert_eq!(r.display_content, "好的");
        assert!(parse_rdp_actions("没有计划", MAX).is_none());
    }

    #[test]
    fn rdp_fence_is_recognised_and_removed_from_display() {
        let c = "先看看\n```rdp-plan\n{\"summary\":\"s\",\"steps\":[{\"description\":\"d\",\"operations\":[{\"type\":\"move\",\"x\":5,\"y\":6}]}]}\n```\n";
        let r = parse_rdp_actions(c, MAX).unwrap();
        assert_eq!(r.parse_error, None);
        assert_eq!(r.display_content, "先看看");
        assert_eq!(
            r.actions[0].rdp_operations.as_ref().unwrap(),
            &vec![Move { x: 5, y: 6 }]
        );
    }

    #[test]
    fn plan_blocks_are_hidden_from_chat_text() {
        assert_eq!(
            strip_rdp_plan_block("好的\n<RDP_PLAN>{\"a\":1}</RDP_PLAN>"),
            "好的"
        );
        assert_eq!(strip_rdp_plan_block("先说明\n<RDP_PLAN>\n{\"ver"), "先说明");
        assert_eq!(strip_rdp_plan_block("结果\n</RDP"), "结果");
        assert_eq!(strip_rdp_plan_block("没有计划"), "没有计划");
    }

    #[test]
    fn truncated_plan_is_repaired() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"s","steps":[{"description":"d","operations":[{"type":"key","keys":["Enter"]}"#;
        assert_eq!(ops(c), vec![key(&["Enter"])]);
    }

    #[test]
    fn unescaped_quotes_inside_strings() {
        let c = r#"<RDP_PLAN>{"version":1,"summary":"点击"开始"按钮","steps":[{"description":"d","operations":[{"type":"key","keys":["Enter"]}]}]}</RDP_PLAN>"#;
        let r = parse_rdp_actions(c, MAX).unwrap();
        assert_eq!(r.parse_error, None);
    }
}
