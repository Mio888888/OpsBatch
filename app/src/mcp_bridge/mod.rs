//! The external MCP bridge inside the app: starts / stops the HTTP server from the settings, runs the tool calls of
//! outside agents against the terminal sessions (with the permission mode, blocklist and grants of the AI safety
//! settings) and reports everything to the activity window.

pub mod activity;
pub mod window;

use std::time::{Duration, Instant};

use gpui_kit::*;
use ops_ai::mcp_bridge::{BridgeCall, BridgeHandle, ToolOutcome};
use ops_ai::prefs::external_mcp::{self, ApprovalAlert, Mode};
use ops_ai::prefs::PermissionMode;
use ops_i18n::t;
use serde_json::json;

use crate::ai::prefs_store::{self, PrefsStore};
use crate::terminal::executions::ExecStatus;
use crate::terminal::pane::TerminalPane;
use crate::terminal::workspace::{workspace, ConnState, TabKind};
use activity::{Decision, McpActivity, Status, Target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeStatus {
    Disabled,
    /// External MCP is on but the HTTP transport (the only one in this build) is not.
    NeedsHttp,
    Starting,
    Running(String),
    /// `port_in_use`, `invalid_address` or the system's message.
    Error(String),
}

pub struct McpBridge {
    status: BridgeStatus,
    handle: Option<BridgeHandle>,
    /// `(address, port, token)` the running (or starting) server was started with.
    desired: Option<(String, u16, String)>,
    last_call: Instant,
    calls: flume::Sender<BridgeCall>,
    prefs: Entity<PrefsStore>,
}

struct BridgeGlobal {
    bridge: Entity<McpBridge>,
    activity: Entity<McpActivity>,
}
impl Global for BridgeGlobal {}

pub fn bridge(cx: &App) -> Entity<McpBridge> {
    cx.global::<BridgeGlobal>().bridge.clone()
}

pub fn activity(cx: &App) -> Entity<McpActivity> {
    cx.global::<BridgeGlobal>().activity.clone()
}

/// Creates the bridge, follows the shared preferences and serves the tool calls.
pub fn init(cx: &mut App) {
    let (tx, rx) = flume::unbounded::<BridgeCall>();
    let prefs = prefs_store::shared(cx);
    let activity = cx.new(|_| McpActivity::default());
    let bridge = cx.new(|cx| {
        cx.observe(&prefs, |this: &mut McpBridge, prefs, cx| {
            let settings = prefs.read(cx).prefs().external_mcp.clone();
            this.apply(&settings, cx);
        })
        .detach();
        McpBridge { status: BridgeStatus::Disabled, handle: None, desired: None, last_call: Instant::now(), calls: tx, prefs: prefs.clone() }
    });
    cx.set_global(BridgeGlobal { bridge: bridge.clone(), activity });
    cx.spawn(async move |cx| {
        while let Ok(call) = rx.recv_async().await {
            let _ = cx.update(|cx| dispatch(call, cx));
        }
    })
    .detach();
    // temporary mode switches itself off after the idle timeout
    let watched = bridge.clone();
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(30)).await;
        let _ = watched.update(cx, |b, cx| b.check_idle(cx));
    })
    .detach();
}

impl McpBridge {
    pub fn status(&self) -> &BridgeStatus {
        &self.status
    }

    /// Brings the server in line with the settings: running exactly when the switch and the HTTP transport are on.
    pub fn apply(&mut self, s: &external_mcp::ExternalMcpSettings, cx: &mut Context<Self>) {
        if !self.prefs.read(cx).loaded() {
            return;
        }
        let wanted = if !s.enabled {
            None
        } else if !s.http.enabled || s.http.token.is_empty() {
            self.stop(BridgeStatus::NeedsHttp, cx);
            return;
        } else {
            Some((s.http.listen_address.clone(), s.http.port, s.http.token.clone()))
        };
        if wanted == self.desired && wanted.is_some() {
            return;
        }
        let Some((address, port, token)) = wanted else {
            self.stop(BridgeStatus::Disabled, cx);
            return;
        };
        self.handle = None;
        self.desired = Some((address.clone(), port, token.clone()));
        self.status = BridgeStatus::Starting;
        cx.notify();
        let calls = self.calls.clone();
        let want = (address.clone(), port, token.clone());
        let url_host = if external_mcp::is_wildcard_address(&address) { s.http.client_host.clone() } else { address.clone() };
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { ops_ai::mcp_bridge::start(&address, port, token, calls).await }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                // the settings changed again while the listener was starting
                if this.desired.as_ref() != Some(&want) {
                    return;
                }
                match result {
                    Ok(handle) => {
                        let url = external_mcp::http_url(&url_host, handle.addr.port(), "");
                        let url = if url.is_empty() { format!("http://{}{}", handle.addr, external_mcp::HTTP_PATH) } else { url };
                        this.status = BridgeStatus::Running(url);
                        this.handle = Some(handle);
                    }
                    Err(e) => {
                        this.desired = None;
                        this.status = BridgeStatus::Error(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop(&mut self, status: BridgeStatus, cx: &mut Context<Self>) {
        let changed = self.handle.is_some() || self.desired.is_some() || self.status != status;
        self.handle = None;
        self.desired = None;
        self.status = status;
        if changed {
            cx.notify();
        }
    }

    fn check_idle(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.status, BridgeStatus::Running(_)) {
            return;
        }
        let s = self.prefs.read(cx).prefs().external_mcp.clone();
        if s.mode == Mode::Temporary && self.last_call.elapsed() >= Duration::from_secs(s.idle_timeout_minutes as u64 * 60) {
            self.prefs.update(cx, |p, cx| p.update(cx, |prefs| prefs.external_mcp.enabled = false));
        }
    }
}

// ---------------------------------------------------------------------------
// Tool calls
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Session {
    id: String,
    name: String,
    ip: String,
    local: bool,
    state: ConnState,
    active: bool,
    pane: Entity<TerminalPane>,
}

impl Session {
    fn target(&self) -> Target {
        Target { session_id: self.id.clone(), name: self.name.clone(), ip: self.ip.clone(), local: self.local }
    }

    fn json(&self) -> serde_json::Value {
        json!({
            "session_id": self.id,
            "name": self.name,
            "host": self.ip,
            "type": if self.local { "local" } else { "ssh" },
            "connected": self.state == ConnState::Connected,
            "active": self.active,
        })
    }
}

fn sessions(cx: &App) -> Vec<Session> {
    let ws = workspace(cx).read(cx);
    let active = ws.active_key().map(str::to_string);
    let mut out = Vec::new();
    for tab in ws.tabs() {
        let is_active = active.as_deref() == Some(tab.key.as_str());
        let name = if tab.kind == TabKind::Local && tab.host_name.is_empty() { t!("mcpwin.target.local") } else { tab.host_name.clone() };
        if let Some(pane) = &tab.pane {
            out.push(Session { id: pane.read(cx).session_id.clone(), name: name.clone(), ip: tab.host_ip.clone(), local: tab.kind == TabKind::Local, state: tab.state, active: is_active, pane: pane.clone() });
        }
        if let Some(split) = &tab.split {
            if let Some(pane) = &split.pane {
                out.push(Session { id: pane.read(cx).session_id.clone(), name: format!("{name} ({})", t!("mcpwin.target.split")), ip: tab.host_ip.clone(), local: tab.kind == TabKind::Local, state: split.state, active: false, pane: pane.clone() });
            }
        }
    }
    out
}

/// The session a call names, else the active one, else the first connected one.
fn resolve(session_id: Option<&str>, cx: &App) -> Result<Session, String> {
    let all = sessions(cx);
    match session_id.filter(|s| !s.is_empty()) {
        Some(id) => all.into_iter().find(|s| s.id == id).ok_or_else(|| format!("Unknown session_id: {id}. Use list_sessions to see the open sessions.")),
        None => all.iter().find(|s| s.active && s.state == ConnState::Connected).or_else(|| all.iter().find(|s| s.state == ConnState::Connected)).cloned().ok_or_else(|| "No connected terminal session is open in OpsBatch.".to_string()),
    }
}

fn exec_status(s: ExecStatus) -> &'static str {
    match s {
        ExecStatus::Completed => "completed",
        ExecStatus::Closed => "closed",
        ExecStatus::Timeout => "timeout",
        ExecStatus::WriteFailed => "write_failed",
    }
}

fn tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    format!("…{}", text.chars().skip(count - max_chars).collect::<String>())
}

/// Removes terminal control sequences (colours, reverse video from the shell's paste highlight, OSC titles, ...) and
/// carriage returns, so records and answers contain readable text only.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek().copied() {
                // CSI: parameters, intermediates, one final byte
                Some('[') => {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST (ESC \)
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// The line of an answer worth showing in the record: the first output line, else the status note.
fn detail_of(text: &str, command: &str) -> String {
    let clean = strip_ansi(text);
    let lines: Vec<&str> = clean.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // the terminal echoes the command first, and the record already shows it; status notes come last
    let line = lines.iter().find(|l| !l.starts_with('[') && !command.trim().is_empty() && !l.ends_with(command.trim()) && **l != command.trim()).or_else(|| lines.iter().find(|l| !l.starts_with('['))).or_else(|| lines.first()).copied().unwrap_or("");
    line.chars().take(300).collect()
}

pub(crate) fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(0);
    lines[end.saturating_sub(n)..end].join("\n")
}

fn permission_label(mode: PermissionMode) -> &'static str {
    mode.as_str()
}

fn dispatch(call: BridgeCall, cx: &mut App) {
    bridge(cx).update(cx, |b, _| b.last_call = Instant::now());
    let activity = activity(cx);
    window::show(false, cx);
    let BridgeCall { tool, arguments, reply } = call;
    /// Records and answers a call that needs no waiting.
    fn finish_now(cx: &mut App, activity: &Entity<McpActivity>, reply: tokio::sync::oneshot::Sender<ToolOutcome>, tool: &str, summary: String, target: Option<Target>, outcome: ToolOutcome, detail: String) {
        let id = activity.update(cx, |a, cx| a.begin(tool, summary, target, cx));
        let status = if outcome.is_error { Status::Failed } else { Status::Done };
        let detail = if outcome.is_error && detail.is_empty() { tail(&outcome.text, 300) } else { detail };
        activity.update(cx, |a, cx| a.finish(&id, status, detail, cx));
        let _ = reply.send(outcome);
    }
    match tool.as_str() {
        "get_environment" => {
            let limits = crate::ai::runtime::limits(cx);
            let all = sessions(cx);
            let text = serde_json::to_string_pretty(&json!({
                "app": "OpsBatch",
                "version": env!("CARGO_PKG_VERSION"),
                "permission_mode": permission_label(limits.permission_mode),
                "command_timeout_seconds": limits.command_timeout,
                "sessions": all.iter().map(Session::json).collect::<Vec<_>>(),
            }))
            .unwrap_or_default();
            finish_now(cx, &activity, reply, "get_environment", String::new(), None, ToolOutcome::ok(text), t!("mcpwin.detail.environment", count = all.len(), mode = permission_label(limits.permission_mode)));
        }
        "list_sessions" => {
            let all = sessions(cx);
            let text = serde_json::to_string_pretty(&all.iter().map(Session::json).collect::<Vec<_>>()).unwrap_or_default();
            finish_now(cx, &activity, reply, "list_sessions", String::new(), None, ToolOutcome::ok(text), t!("mcpwin.detail.sessions", count = all.len()));
        }
        "terminal_read" => {
            let wanted = arguments.get("session_id").and_then(|v| v.as_str());
            let lines = arguments.get("lines").and_then(|v| v.as_u64()).unwrap_or(50).clamp(1, 500) as usize;
            match resolve(wanted, cx) {
                Ok(s) => {
                    let text = strip_ansi(&last_lines(&s.pane.read(cx).buffer_text(), lines));
                    let detail = text.lines().last().unwrap_or("").to_string();
                    finish_now(cx, &activity, reply, "terminal_read", t!("mcpwin.detail.readLines", count = lines), Some(s.target()), ToolOutcome::ok(text), detail);
                }
                Err(e) => finish_now(cx, &activity, reply, "terminal_read", String::new(), None, ToolOutcome::error(e), String::new()),
            }
        }
        "terminal_execute" => {
            let command = arguments.get("command").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if command.is_empty() {
                return finish_now(cx, &activity, reply, "terminal_execute", String::new(), None, ToolOutcome::error("`command` is required."), String::new());
            }
            let wanted = arguments.get("session_id").and_then(|v| v.as_str()).map(str::to_string);
            let timeout = arguments.get("timeout_seconds").and_then(|v| v.as_u64());
            let session = match resolve(wanted.as_deref(), cx) {
                Ok(s) if s.state == ConnState::Connected => s,
                Ok(_) => return finish_now(cx, &activity, reply, "terminal_execute", command, None, ToolOutcome::error("The session is not connected."), String::new()),
                Err(e) => return finish_now(cx, &activity, reply, "terminal_execute", command, None, ToolOutcome::error(e), String::new()),
            };
            let id = activity.update(cx, |a, cx| a.begin("terminal_execute", command.clone(), Some(session.target()), cx));
            cx.spawn(async move |cx| {
                let summary_for_detail = command.clone();
                let outcome = execute(&id, session, command, timeout, cx).await;
                let status = if outcome.is_error { if outcome.text.starts_with("Refused") || outcome.text.starts_with("Rejected") { Status::Denied } else { Status::Failed } } else { Status::Done };
                cx.update(|cx| activity.update(cx, |a, cx| a.finish(&id, status, detail_of(&outcome.text, &summary_for_detail), cx)));
                let _ = reply.send(outcome);
            })
            .detach();
        }
        other => {
            let _ = reply.send(ToolOutcome::error(format!("Unknown tool: {other}")));
        }
    }
}

/// Policy check, approval and the run of one `terminal_execute`.
async fn execute(id: &str, session: Session, command: String, timeout: Option<u64>, cx: &mut AsyncApp) -> ToolOutcome {
    let (ctx, limits, activity) = cx.update(|cx| (crate::services::ctx(cx), crate::ai::runtime::limits(cx), activity(cx)));
    let assess = command.clone();
    let assessment = ops_core::rt::run(async move { ops_ai::client::ai_assess_action(&ctx.db, assess) }).await.and_then(|r| r);
    let assessment = match assessment {
        Ok(a) => a,
        Err(e) => return ToolOutcome::error(format!("Refused: the safety policy could not be evaluated ({e}).")),
    };
    if assessment.decision == ops_ai::client::AiActionDecision::Block {
        return ToolOutcome::error(format!("Refused by OpsBatch: {}", assessment.reason));
    }
    let needs_approval = assessment.decision != ops_ai::client::AiActionDecision::Safe && limits.permission_mode != PermissionMode::Auto;
    if needs_approval {
        let id_for_request = id.to_string();
        let rx = cx.update(|cx| {
            let rx = activity.update(cx, |a, cx| a.request_approval(&id_for_request, cx));
            let alert = prefs_store::shared(cx).read(cx).prefs().external_mcp.approval_alert;
            let focused = cx.active_window().is_some();
            window::show(alert == ApprovalAlert::Always || (alert == ApprovalAlert::Unfocused && !focused), cx);
            rx
        });
        match rx.await {
            Ok(Decision::Approve) => {}
            Ok(Decision::AlwaysAllow) => {
                let command = command.clone();
                let _ = cx.update(|cx| {
                    let patterns = ops_ai::prefs::safety::always_allow_patterns(&command);
                    prefs_store::shared(cx).update(cx, |s, cx| {
                        s.update(cx, |p| {
                            for pattern in patterns {
                                let mut rule = ops_ai::prefs::safety::new_grant();
                                rule.capability_id = "terminal.execute".into();
                                rule.command_pattern = Some(pattern);
                                rule.note = Some("MCP".into());
                                p.permission_grants.push(rule);
                            }
                        })
                    });
                });
            }
            Ok(Decision::Reject) | Err(_) => return ToolOutcome::error("Rejected: the user did not approve this command in OpsBatch."),
        }
    }
    let seconds = timeout.map(|t| t.clamp(1, 3600) as u32).unwrap_or(limits.command_timeout);
    let pane = session.pane.clone();
    let rx = cx.update(|cx| pane.update(cx, |p, cx| p.execute_command(&command, Some(Duration::from_secs(seconds as u64)), cx)));
    match rx.await {
        Ok(result) => {
            let formatted = ops_ai::follow_up::format_terminal_command_result(&ops_ai::follow_up::TerminalCommandResult { status: exec_status(result.status).into(), output: result.output, exit_code: result.exit_code });
            let failed = result.exit_code.is_some_and(|c| c != 0) || result.status != ExecStatus::Completed;
            ToolOutcome { text: tail(&strip_ansi(&formatted), 20_000), is_error: failed }
        }
        Err(_) => ToolOutcome::error("The terminal session closed before the command finished."),
    }
}

#[cfg(test)]
mod tests {
    use super::{detail_of, strip_ansi};

    #[test]
    fn control_sequences_are_removed() {
        assert_eq!(strip_ansi("\x1b[7muname -a\x1b[27m"), "uname -a");
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m done\r\n"), "ok done\n");
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\text"), "text");
        assert_eq!(strip_ansi("a\tb\x07c"), "a\tbc");
        assert_eq!(strip_ansi("\x1b[?2004hprompt\x1b[?2004l"), "prompt");
        assert_eq!(strip_ansi("plain 中文"), "plain 中文");
        assert_eq!(strip_ansi("trailing \x1b"), "trailing ");
    }

    #[test]
    fn the_record_detail_skips_the_echo_and_status_notes() {
        let out = "\x1b[7muname -a && df -h /\x1b[27m\nDarwin Mac-mini.local 25.6.0\n\n[当前命令：completed，退出码 0]";
        assert_eq!(detail_of(out, "uname -a && df -h /"), "Darwin Mac-mini.local 25.6.0");
        assert_eq!(detail_of("macuser@Mac ~ % echo hi\nhi", "echo hi"), "hi");
        assert_eq!(detail_of("[当前命令：completed，退出码 0]", "true"), "[当前命令：completed，退出码 0]");
        assert_eq!(detail_of("", "true"), "");
    }
}
