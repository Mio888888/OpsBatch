//! Workflow executor (port of `pages/Workflow/workflowExecutor.ts` and `workflowExecutionLogs.ts`).
//!
//! Nodes run one after another in dependency order. Command-like nodes, file transfers, confirmations and sleeping are
//! delegated to a [`WorkflowRuntime`], so the logic here is deterministic and testable without hosts.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use regex::Regex;

use crate::model::{CanvasNode, Connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeResult {
    pub success: bool,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    Log(String),
    NodeStart { node_id: String, node_name: String },
    NodeComplete { node_id: String, node_name: String, success: bool, output: String, exit_code: i32 },
    LevelStart { level: usize, total: usize, count: usize },
    Done,
}

/// How a shell-ish node talks to the hosts: SSH timeout and how long to wait for the result events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub ssh_timeout_secs: u64,
    pub wait_cap_secs: u64,
}

pub const COMMAND_SPEC: CommandSpec = CommandSpec { ssh_timeout_secs: 30, wait_cap_secs: 120 };
pub const SCRIPT_SPEC: CommandSpec = CommandSpec { ssh_timeout_secs: 60, wait_cap_secs: 180 };
pub const ROLLBACK_SPEC: CommandSpec = CommandSpec { ssh_timeout_secs: 30, wait_cap_secs: 120 };
pub const TRANSFER_TIMEOUT_SECS: u64 = 120;
const CANCELLED: &str = "用户取消";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// `[host] output` lines of every host that printed something.
    pub output: String,
    pub success: bool,
    pub exit_code: i32,
}

#[allow(async_fn_in_trait)]
pub trait WorkflowRuntime {
    async fn run_command(&self, host_ids: &[String], command: &str, spec: CommandSpec) -> Result<CommandOutput, String>;
    async fn transfer(&self, host_ids: &[String], direction: &str, local_path: &str, remote_path: &str, timeout_secs: u64) -> Result<(), String>;
    /// Ask the user; `false` terminates the workflow.
    async fn confirm(&self, node_name: &str, description: &str) -> bool;
    async fn sleep(&self, duration: Duration);
    /// Set by a "stop" button; checked between nodes.
    fn cancelled(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Templates and conditions
// ---------------------------------------------------------------------------

fn js_bool(v: Option<bool>) -> String {
    v.map(|b| b.to_string()).unwrap_or_default()
}

/// `{{lastResult[.output|.exitCode|.success]}}` and `{{result.<id>[.output|.exitCode|.success]}}`.
pub fn resolve_templates(template: &str, last: Option<&NodeResult>, results: &HashMap<String, NodeResult>) -> String {
    let mut s = template.to_string();
    s = s.replace("{{lastResult}}", last.map(|r| r.output.as_str()).unwrap_or(""));
    s = s.replace("{{lastResult.output}}", last.map(|r| r.output.as_str()).unwrap_or(""));
    s = s.replace("{{lastResult.exitCode}}", &last.map(|r| r.exit_code.to_string()).unwrap_or_default());
    s = s.replace("{{lastResult.success}}", &js_bool(last.map(|r| r.success)));
    let re = |suffix: &str| Regex::new(&format!(r"\{{\{{result\.([a-zA-Z0-9_-]+){suffix}\}}\}}")).unwrap();
    // Each pattern is applied to the text left by the previous one, like the chained `replace` calls.
    for (suffix, f) in [
        ("", Box::new(|r: &NodeResult| r.output.clone()) as Box<dyn Fn(&NodeResult) -> String>),
        (r"\.output", Box::new(|r: &NodeResult| r.output.clone())),
        (r"\.exitCode", Box::new(|r: &NodeResult| r.exit_code.to_string())),
        (r"\.success", Box::new(|r: &NodeResult| r.success.to_string())),
    ] {
        s = re(suffix).replace_all(&s, |caps: &regex::Captures<'_>| results.get(&caps[1]).map(&f).unwrap_or_default()).into_owned();
    }
    s
}

/// JavaScript `Number(text)` for the strings conditions compare.
fn js_number(text: &str) -> f64 {
    let t = text.trim();
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return i64::from_str_radix(rest, radix).map(|n| n as f64).unwrap_or(f64::NAN);
        }
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also parses "inf"/"nan"; JavaScript does not.
    if t.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix(['\'', '"']).unwrap_or(s);
    s.strip_suffix(['\'', '"']).unwrap_or(s).to_string()
}

pub fn evaluate_condition(expression: &str, last: Option<&NodeResult>, results: &HashMap<String, NodeResult>) -> bool {
    let resolved = resolve_templates(expression, last, results);
    let resolved = resolved.trim();
    if resolved == "true" {
        return true;
    }
    if resolved == "false" {
        return false;
    }
    let re = Regex::new(r#"^(!?\s*['"]?.*?['"]?)\s*(===|!==|==|!=|>=|<=|>|<)\s*(['"]?.*?['"]?)$"#).unwrap();
    if let Some(caps) = re.captures(resolved) {
        let (left, op, right) = (strip_quotes(&caps[1]), &caps[2], strip_quotes(&caps[3]));
        let (ln, rn) = (js_number(&left), js_number(&right));
        let use_num = !ln.is_nan() && !rn.is_nan();
        return match op {
            "===" | "==" => {
                if use_num {
                    ln == rn
                } else {
                    left == right
                }
            }
            "!==" | "!=" => {
                if use_num {
                    ln != rn
                } else {
                    left != right
                }
            }
            ">=" => use_num && ln >= rn,
            "<=" => use_num && ln <= rn,
            ">" => use_num && ln > rn,
            "<" => use_num && ln < rn,
            _ => unreachable!(),
        };
    }
    if let Some(inner) = resolved.strip_prefix('!') {
        match inner.trim() {
            "true" => return false,
            "false" => return true,
            _ => {}
        }
    }
    resolved != "0" && !resolved.is_empty() && resolved != "false"
}

/// JavaScript `parseInt(text)`: the leading integer, if any.
fn parse_int_prefix(text: &str) -> Option<i64> {
    let t = text.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    digits[..end].parse::<i64>().ok().map(|n| sign * n)
}

// ---------------------------------------------------------------------------
// Levels
// ---------------------------------------------------------------------------

/// Nodes grouped by dependency depth; nodes caught in a cycle follow one by one at the end.
pub fn build_execution_levels<'a>(nodes: &'a [CanvasNode], connections: &[Connection]) -> Vec<Vec<&'a CanvasNode>> {
    let by_id: HashMap<&str, &CanvasNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut in_degree: HashMap<&str, i64> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut children: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.id.as_str(), Vec::new())).collect();
    for c in connections {
        if by_id.contains_key(c.from_id.as_str()) && by_id.contains_key(c.to_id.as_str()) {
            *in_degree.entry(c.to_id.as_str()).or_default() += 1;
            children.entry(c.from_id.as_str()).or_default().push(c.to_id.as_str());
        }
    }
    let mut queue: Vec<&CanvasNode> = nodes.iter().filter(|n| in_degree[n.id.as_str()] == 0).collect();
    let mut levels = Vec::new();
    let mut processed: HashSet<&str> = HashSet::new();
    while !queue.is_empty() {
        levels.push(queue.clone());
        processed.extend(queue.iter().map(|n| n.id.as_str()));
        let mut next = Vec::new();
        for n in &queue {
            for child in children.get(n.id.as_str()).into_iter().flatten() {
                let d = in_degree.entry(child).or_default();
                *d -= 1;
                if *d == 0 && !processed.contains(child) {
                    if let Some(node) = by_id.get(child) {
                        next.push(*node);
                    }
                }
            }
        }
        queue = next;
    }
    for n in nodes {
        if !processed.contains(n.id.as_str()) {
            levels.push(vec![n]);
        }
    }
    levels
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

struct Outcome {
    ok: bool,
    msg: String,
    result: NodeResult,
}

impl Outcome {
    fn ok(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self { ok: true, result: NodeResult { success: true, output: msg.clone(), exit_code: 0 }, msg }
    }
    fn fail(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self { ok: false, result: NodeResult { success: false, output: msg.clone(), exit_code: 1 }, msg }
    }
    fn from_command(out: CommandOutput, ok_text: &str, fail_text: &str) -> Self {
        let text = if out.output.is_empty() { (if out.success { ok_text } else { fail_text }).to_string() } else { out.output };
        Self { ok: out.success, msg: text.clone(), result: NodeResult { success: out.success, output: text, exit_code: out.exit_code } }
    }
}

pub async fn execute_workflow<R: WorkflowRuntime>(
    nodes: &[CanvasNode],
    connections: &[Connection],
    host_ids: &[String],
    runtime: &R,
    on_progress: &mut dyn FnMut(ProgressEvent),
    default_node_interval_seconds: f64,
) {
    let enabled: Vec<&CanvasNode> = nodes.iter().filter(|n| n.enabled).collect();
    if enabled.is_empty() {
        on_progress(ProgressEvent::Log("工作流没有启用的步骤".into()));
        return;
    }
    on_progress(ProgressEvent::Log(format!("开始执行工作流（{} 个步骤）", enabled.len())));

    let node_map: HashMap<&str, &CanvasNode> = enabled.iter().map(|n| (n.id.as_str(), *n)).collect();
    let mut outgoing: HashMap<&str, Vec<&Connection>> = HashMap::new();
    let mut in_degree: HashMap<&str, i64> = enabled.iter().map(|n| (n.id.as_str(), 0)).collect();
    for c in connections {
        if !node_map.contains_key(c.from_id.as_str()) || !node_map.contains_key(c.to_id.as_str()) {
            continue;
        }
        outgoing.entry(c.from_id.as_str()).or_default().push(c);
        *in_degree.entry(c.to_id.as_str()).or_default() += 1;
    }

    let mut results: HashMap<String, NodeResult> = HashMap::new();
    let mut condition_results: HashMap<&str, bool> = HashMap::new();
    let mut switch_results: HashMap<&str, String> = HashMap::new();
    let mut last: Option<NodeResult> = None;
    let mut ready: Vec<&CanvasNode> = enabled.iter().copied().filter(|n| in_degree[n.id.as_str()] == 0).collect();
    let mut processed: HashSet<&str> = HashSet::new();
    let mut executed = 0usize;
    let mut level = 0usize;
    let total_levels = enabled.len();

    while !ready.is_empty() {
        level += 1;
        on_progress(ProgressEvent::LevelStart { level, total: total_levels, count: ready.len() });
        let mut next_ready: Vec<&CanvasNode> = Vec::new();

        for step in ready {
            if runtime.cancelled() {
                on_progress(ProgressEvent::Log("工作流已被用户终止".into()));
                on_progress(ProgressEvent::Done);
                return;
            }
            if executed > 0 && default_node_interval_seconds > 0.0 {
                on_progress(ProgressEvent::Log(format!("等待 {default_node_interval_seconds}s 后执行下一个节点")));
                runtime.sleep(Duration::from_secs_f64(default_node_interval_seconds)).await;
            }
            processed.insert(step.id.as_str());
            executed += 1;
            on_progress(ProgressEvent::NodeStart { node_id: step.id.clone(), node_name: step.name.clone() });

            let outcome = execute_node(step, host_ids, runtime, last.as_ref(), &results).await;
            results.insert(step.id.clone(), outcome.result.clone());
            last = Some(outcome.result.clone());
            on_progress(ProgressEvent::NodeComplete {
                node_id: step.id.clone(),
                node_name: step.name.clone(),
                success: outcome.ok,
                output: outcome.msg.clone(),
                exit_code: outcome.result.exit_code,
            });

            if !outcome.ok && outcome.msg == CANCELLED {
                on_progress(ProgressEvent::Log("工作流已被用户终止".into()));
                on_progress(ProgressEvent::Done);
                return;
            }

            if step.kind == "condition" {
                condition_results.insert(step.id.as_str(), outcome.ok);
                on_progress(ProgressEvent::Log(format!("条件节点「{}」评估结果: {}", step.name, if outcome.ok { "是 (true)" } else { "否 (false)" })));
            }
            if step.kind == "switch" && !outcome.result.output.is_empty() {
                switch_results.insert(step.id.as_str(), outcome.result.output.clone());
                on_progress(ProgressEvent::Log(format!("多分支节点「{}」匹配分支: {}", step.name, outcome.result.output)));
            }

            for edge in outgoing.get(step.id.as_str()).into_iter().flatten() {
                let target = edge.to_id.as_str();
                if processed.contains(target) {
                    continue;
                }
                if let (true, Some(handle)) = (step.kind == "condition", edge.source_handle.as_deref()) {
                    let Some(passed) = condition_results.get(step.id.as_str()).copied() else { continue };
                    if (handle == "true" && !passed) || (handle == "false" && passed) {
                        continue;
                    }
                }
                if let (true, Some(handle)) = (step.kind == "switch", edge.source_handle.as_deref()) {
                    let Some(matched) = switch_results.get(step.id.as_str()) else { continue };
                    if handle != matched {
                        continue;
                    }
                }
                // A node is ready once all of its unfiltered incoming edges are satisfied.
                let degree = in_degree.entry(target).or_default();
                *degree -= 1;
                if *degree <= 0 && !processed.contains(target) {
                    if let Some(node) = node_map.get(target) {
                        if !next_ready.iter().any(|n| n.id == *target) {
                            next_ready.push(node);
                        }
                    }
                }
            }
        }
        ready = next_ready;
    }

    on_progress(ProgressEvent::Log("工作流执行完成".into()));
    on_progress(ProgressEvent::Done);
}

async fn execute_node<R: WorkflowRuntime>(step: &CanvasNode, host_ids: &[String], runtime: &R, last: Option<&NodeResult>, results: &HashMap<String, NodeResult>) -> Outcome {
    let noop = || Outcome::ok(step.kind.clone());
    let skipped = |msg: &str| Outcome::ok(msg);
    let resolve = |text: &str| resolve_templates(text, last, results);

    match step.kind.as_str() {
        "confirm" => {
            let text = resolve(if step.config.is_empty() { "请确认是否继续执行" } else { &step.config });
            if runtime.confirm(&step.name, &text).await {
                Outcome::ok("已确认")
            } else {
                Outcome::fail(CANCELLED)
            }
        }
        "start" | "end" | "selectHost" => noop(),
        "command" | "quickAction" | "script" | "rollback" => {
            let (spec, empty_msg, ok_text, fail_text) = match step.kind.as_str() {
                "script" => (SCRIPT_SPEC, "跳过(无配置)", "成功", "失败"),
                "rollback" => (ROLLBACK_SPEC, "回滚(无配置)", "回滚成功", "回滚失败"),
                _ => (COMMAND_SPEC, "跳过(无配置)", "成功", "失败"),
            };
            if step.config.is_empty() {
                return skipped(empty_msg);
            }
            match runtime.run_command(host_ids, &resolve(&step.config), spec).await {
                Ok(out) => Outcome::from_command(out, ok_text, fail_text),
                Err(e) => Outcome::fail(e),
            }
        }
        "transfer" => {
            if step.config.is_empty() {
                return skipped("跳过(无配置)");
            }
            let resolved = resolve(&step.config);
            let run = async {
                let cfg: serde_json::Value = serde_json::from_str(&resolved).map_err(|e| e.to_string())?;
                let text = |k: &str| cfg.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let direction = text("direction");
                let selected: Vec<String> = match cfg.get("localPaths").and_then(|v| v.as_array()) {
                    Some(list) if !list.is_empty() => list.iter().filter_map(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                    _ => Some(text("localPath")).filter(|s| !s.is_empty()).into_iter().collect(),
                };
                let paths: Vec<String> = if direction == "download" { selected.into_iter().take(1).collect() } else { selected };
                if paths.is_empty() {
                    return Err("本地路径为空".to_string());
                }
                let direction = if direction.is_empty() { "upload".to_string() } else { direction };
                for path in &paths {
                    runtime.transfer(host_ids, &direction, path, &text("remotePath"), TRANSFER_TIMEOUT_SECS).await?;
                }
                Ok(paths.len())
            };
            match run.await {
                Ok(n) if n > 1 => Outcome::ok(format!("传输成功({n}项)")),
                Ok(_) => Outcome::ok("传输成功"),
                Err(e) => Outcome::fail(e),
            }
        }
        "delay" => {
            let seconds = parse_int_prefix(&step.config).filter(|n| *n != 0).unwrap_or(5);
            runtime.sleep(Duration::from_secs(seconds.max(0) as u64)).await;
            Outcome::ok(format!("等待{seconds}s"))
        }
        "condition" => {
            if step.config.is_empty() {
                return Outcome::ok("条件通过(无条件表达式)");
            }
            let expr = resolve(&step.config);
            let passed = evaluate_condition(&step.config, last, results);
            let msg = if passed { format!("条件通过: {expr}") } else { format!("条件不满足: {expr}") };
            Outcome { ok: passed, result: NodeResult { success: passed, output: msg.clone(), exit_code: i32::from(!passed) }, msg }
        }
        "switch" => execute_switch(step, last, results),
        _ => noop(),
    }
}

fn execute_switch(step: &CanvasNode, last: Option<&NodeResult>, results: &HashMap<String, NodeResult>) -> Outcome {
    let first = || Outcome::ok("case-0");
    if step.config.is_empty() {
        return first();
    }
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&step.config) else { return first() };
    let expression = cfg.get("expression").and_then(|v| v.as_str()).unwrap_or("");
    let cases: Vec<(String, String)> = cfg
        .get("cases")
        .and_then(|v| v.as_array())
        .map(|list| list.iter().map(|c| (c.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(), c.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    if cases.is_empty() {
        return first();
    }
    let value = resolve_templates(expression, last, results).trim().to_string();
    let mut matched: Option<usize> = None;
    let mut default: Option<usize> = None;
    for (i, (_, v)) in cases.iter().enumerate() {
        if v == "*" {
            default.get_or_insert(i);
            continue;
        }
        if *v == value {
            matched = Some(i);
            break;
        }
    }
    let index = matched.or(default).unwrap_or(0);
    let key = format!("case-{index}");
    let label = if cases[index].0.is_empty() { key.clone() } else { cases[index].0.clone() };
    // The result's output carries the case key; the executor reads it to pick the branch.
    Outcome { ok: true, msg: format!("匹配分支「{label}」: {expression} → {value}"), result: NodeResult { success: true, output: key, exit_code: 0 } }
}

// ---------------------------------------------------------------------------
// Execution log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStatus {
    Running,
    Success,
    Fail,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp_ms: i64,
    pub message: String,
    pub status: Option<LogStatus>,
    pub node_name: Option<String>,
}

pub struct LogLabels {
    pub start_execute: String,
    pub execute_success: String,
    pub execute_fail: String,
    pub stage_info: Box<dyn Fn(usize, usize, usize) -> String>,
}

/// The log line for a progress event (`done` has none).
pub fn create_log_entry(event: &ProgressEvent, labels: &LogLabels, now_ms: i64) -> Option<LogEntry> {
    Some(match event {
        ProgressEvent::Log(message) => LogEntry { timestamp_ms: now_ms, message: message.clone(), status: Some(LogStatus::Info), node_name: None },
        ProgressEvent::NodeStart { node_name, .. } => LogEntry { timestamp_ms: now_ms, message: labels.start_execute.clone(), status: Some(LogStatus::Running), node_name: Some(node_name.clone()) },
        ProgressEvent::NodeComplete { node_name, success, output, .. } => {
            let short = if output.encode_utf16().count() > 200 { format!("{}...", output.chars().take(200).collect::<String>()) } else { output.clone() };
            let message = if short.is_empty() { (if *success { &labels.execute_success } else { &labels.execute_fail }).clone() } else { short };
            LogEntry { timestamp_ms: now_ms, message, status: Some(if *success { LogStatus::Success } else { LogStatus::Fail }), node_name: Some(node_name.clone()) }
        }
        ProgressEvent::LevelStart { level, total, count } => LogEntry { timestamp_ms: now_ms, message: (labels.stage_info)(*level, *total, *count), status: Some(LogStatus::Info), node_name: None },
        ProgressEvent::Done => return None,
    })
}
