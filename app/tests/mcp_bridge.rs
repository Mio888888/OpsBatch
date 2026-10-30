//! The external MCP bridge end to end: a real HTTP client talks to the bridge, which drives a real local shell tab,
//! records everything in the activity window and asks for approval according to the permission mode.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use opsbatch_app::ai::prefs_store;
use opsbatch_app::mcp_bridge::activity::{Decision, McpActivity, Status};
use opsbatch_app::mcp_bridge::{activity, bridge, BridgeStatus};
use opsbatch_app::terminal::workspace::{workspace, ConnState};
use ops_ai::prefs::PermissionMode;
use serde_json::{json, Value};

use common::{serial, wait_until, Fixture};

type Reply = Arc<Mutex<Option<(u16, Value)>>>;

fn win<R>(cx: &mut TestAppContext, fx: &Fixture, f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx)).unwrap()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The UI thread has to keep running while the request is open, so it goes out from another thread.
fn send(port: u16, token: Option<&str>, body: Value) -> Reply {
    let reply: Reply = Arc::new(Mutex::new(None));
    let out = reply.clone();
    let token = token.map(str::to_string);
    std::thread::spawn(move || {
        let body = body.to_string();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut req = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len());
        if let Some(t) = token {
            req.push_str(&format!("Authorization: Bearer {t}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(&body);
        stream.write_all(req.as_bytes()).unwrap();
        let mut text = String::new();
        let _ = stream.read_to_string(&mut text);
        let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let json = text.split_once("\r\n\r\n").and_then(|(_, b)| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
        *out.lock().unwrap() = Some((status, json));
    });
    reply
}

fn call(tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})
}

fn wait_reply(cx: &mut TestAppContext, reply: &Reply) -> (u16, Value) {
    wait_until(cx, "bridge answered", |_| reply.lock().unwrap().is_some());
    reply.lock().unwrap().clone().unwrap()
}

fn text_of(v: &Value) -> String {
    v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
}

/// A connected local shell, the bridge switched on over HTTP with the given permission mode; returns (fixture, port).
fn running_bridge(cx: &mut TestAppContext, mode: PermissionMode) -> (Fixture, u16) {
    let fx = common::setup(cx, |_| {});
    win(cx, &fx, |w, cx| w.click("terminal-reopen-local", cx));
    let wsp = cx.update(|cx| workspace(cx));
    wait_until(cx, "local tab connected", |cx| cx.update(|cx| wsp.read(cx).active_tab().is_some_and(|t| t.state == ConnState::Connected)));
    std::thread::sleep(Duration::from_millis(600));
    cx.run_until_parked();
    let store = cx.update(|cx| prefs_store::shared(cx));
    wait_until(cx, "preferences loaded", |cx| store.read_with(cx, |s, _| s.loaded()));
    let port = free_port();
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.update(cx, |p| {
                p.global_permission_mode = mode;
                p.external_mcp.enabled = true;
                p.external_mcp.http.enabled = true;
                p.external_mcp.http.port = port;
                p.external_mcp.http.token = "test-token".into();
            })
        })
    });
    let b = cx.update(|cx| bridge(cx));
    wait_until(cx, "bridge listening", |cx| b.read_with(cx, |b, _| matches!(b.status(), BridgeStatus::Running(_))));
    (fx, port)
}

fn act(cx: &mut TestAppContext) -> Entity<McpActivity> {
    cx.update(|cx| activity(cx))
}

/// The activity window, if it is open: runs `f` against it.
fn in_activity_window<R>(cx: &mut TestAppContext, f: impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> R) -> Option<R> {
    let windows = cx.update(|cx| cx.windows());
    windows.into_iter().find_map(|h| cx.update_window(h, |_, w, cx| { w.render_frame(cx); w.try_find("mcp-window").is_some().then(|| f(w, cx)) }).ok().flatten())
}

#[gpui_kit::test]
fn the_bridge_authenticates_lists_tools_and_describes_the_environment(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, port) = running_bridge(cx, PermissionMode::Confirm);
    let denied = send(port, None, json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
    assert_eq!(wait_reply(cx, &denied).0, 401);

    let init = send(port, Some("test-token"), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
    assert_eq!(wait_reply(cx, &init).1["result"]["serverInfo"]["name"], "opsbatch-external");
    let list = send(port, Some("test-token"), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(wait_reply(cx, &list).1["result"]["tools"].as_array().unwrap().len(), 4);

    // no window until a tool is called
    assert!(in_activity_window(cx, |_, _| ()).is_none());
    let env = send(port, Some("test-token"), call("get_environment", json!({})));
    let env = wait_reply(cx, &env).1;
    let env_json: Value = serde_json::from_str(&text_of(&env)).unwrap();
    assert_eq!(env_json["permission_mode"], "confirm");
    assert_eq!(env_json["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(env_json["sessions"][0]["type"], "local");
    let sessions = send(port, Some("test-token"), call("list_sessions", json!({})));
    let listed: Value = serde_json::from_str(&text_of(&wait_reply(cx, &sessions).1)).unwrap();
    assert_eq!(listed[0]["connected"], true);

    // the activity window opened by itself and shows the records
    cx.run_until_parked();
    let shown = in_activity_window(cx, |w, _| (w.try_find("mcp-record-0").is_some(), w.try_find("mcp-record-1").is_some(), w.try_find("mcp-bridge-status").is_some()));
    assert_eq!(shown, Some((true, true, true)));
    let records = cx.update(|cx| activity(cx).read(cx).records().len());
    assert_eq!(records, 2);
}

#[gpui_kit::test]
fn auto_mode_runs_commands_in_the_terminal_and_the_window_shows_the_server(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, port) = running_bridge(cx, PermissionMode::Auto);
    let reply = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo mcp-mark-7"})));
    let (status, v) = wait_reply(cx, &reply);
    assert_eq!(status, 200);
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert!(text_of(&v).contains("mcp-mark-7"), "{}", text_of(&v));
    assert!(!text_of(&v).contains('\x1b'), "answers carry no terminal escape codes");

    let activity = act(cx);
    cx.update(|cx| {
        let a = activity.read(cx);
        let r = &a.records()[0];
        assert_eq!((r.tool.as_str(), r.summary.as_str(), r.status), ("terminal_execute", "echo mcp-mark-7", Status::Done));
        assert_eq!(r.detail, "mcp-mark-7", "the record shows the first output line, not the echoed command or escape codes");
        assert_eq!(a.target().map(|t| t.local), Some(true));
    });
    // the header shows the server and a preview of its terminal
    let shown = in_activity_window(cx, |w, _| (w.try_find("mcp-target").is_some(), w.try_find("mcp-preview").is_some(), w.try_find("mcp-record-0").is_some()));
    assert_eq!(shown, Some((true, true, true)));

    // reading the terminal back
    let read = send(port, Some("test-token"), call("terminal_read", json!({"lines": 20})));
    assert!(text_of(&wait_reply(cx, &read).1).contains("mcp-mark-7"));
    // a failing command is reported as an error with its exit code
    let fail = send(port, Some("test-token"), call("terminal_execute", json!({"command": "false"})));
    let fail = wait_reply(cx, &fail).1;
    assert_eq!(fail["result"]["isError"], true);
    assert!(text_of(&fail).contains("退出码 1"), "{}", text_of(&fail));
    cx.update(|cx| assert_eq!(activity.read(cx).records().last().unwrap().status, Status::Failed));
}

#[gpui_kit::test]
fn confirm_mode_waits_for_the_window_and_honours_the_decision(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, port) = running_bridge(cx, PermissionMode::Confirm);
    let activity = act(cx);

    // approve
    let reply = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo approved-mark"})));
    wait_until(cx, "approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    assert!(reply.lock().unwrap().is_none(), "the agent is still waiting");
    let card = in_activity_window(cx, |w, _| (w.try_find("mcp-approval-0").is_some(), w.try_find("mcp-approve-0").is_some(), w.try_find("mcp-reject-0").is_some()));
    assert_eq!(card, Some((true, true, true)), "the approval card is in the window");
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    assert_eq!(cx.update(|cx| activity.read(cx).pending()[0].summary.clone()), "echo approved-mark");
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::Approve, cx)));
    let (_, v) = wait_reply(cx, &reply);
    assert!(text_of(&v).contains("approved-mark") && v["result"]["isError"] == false, "{v}");
    cx.update(|cx| assert_eq!(activity.read(cx).record(&id).unwrap().status, Status::Done));

    // reject
    let reply = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo rejected-mark"})));
    wait_until(cx, "second approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::Reject, cx)));
    let (_, v) = wait_reply(cx, &reply);
    assert_eq!(v["result"]["isError"], true);
    assert!(text_of(&v).starts_with("Rejected"), "{}", text_of(&v));
    cx.update(|cx| assert_eq!(activity.read(cx).record(&id).unwrap().status, Status::Denied));
    let buffer = cx.update(|cx| workspace(cx).read(cx).active_tab().and_then(|t| t.pane.clone()).unwrap().read(cx).buffer_text());
    assert!(!buffer.contains("rejected-mark\n"), "a rejected command never reaches the shell");

    // "always allow" runs it and remembers a grant, so the next one needs no approval
    let reply = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo grant-mark"})));
    wait_until(cx, "third approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::AlwaysAllow, cx)));
    assert!(text_of(&wait_reply(cx, &reply).1).contains("grant-mark"));
    let store = cx.update(|cx| prefs_store::shared(cx));
    let grants = store.read_with(cx, |s, _| s.prefs().permission_grants.clone());
    assert_eq!((grants.len(), grants[0].capability_id.as_str(), grants[0].command_pattern.as_deref()), (1, "terminal.execute", Some("echo *")));
    let again = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo grant-again"})));
    assert!(text_of(&wait_reply(cx, &again).1).contains("grant-again"), "the grant lets it through without asking");
    let _ = fx;
}

#[gpui_kit::test]
fn observer_mode_and_the_blocklist_refuse_without_asking(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, port) = running_bridge(cx, PermissionMode::Observer);
    let activity = act(cx);
    let reply = send(port, Some("test-token"), call("terminal_execute", json!({"command": "echo nope"})));
    let v = wait_reply(cx, &reply).1;
    assert_eq!(v["result"]["isError"], true);
    assert!(text_of(&v).starts_with("Refused by OpsBatch") && text_of(&v).contains("观察者"), "{}", text_of(&v));
    cx.update(|cx| {
        assert!(activity.read(cx).pending().is_empty());
        assert_eq!(activity.read(cx).records()[0].status, Status::Denied);
    });
    // reading stays allowed in observer mode
    let read = send(port, Some("test-token"), call("terminal_read", json!({})));
    assert_eq!(wait_reply(cx, &read).1["result"]["isError"], false);

    // auto mode still refuses what the blocklist names
    let store = cx.update(|cx| prefs_store::shared(cx));
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.global_permission_mode = PermissionMode::Auto)));
    let blocked = send(port, Some("test-token"), call("terminal_execute", json!({"command": "rm -rf /"})));
    let v = wait_reply(cx, &blocked).1;
    assert!(text_of(&v).starts_with("Refused"), "{}", text_of(&v));
    let unknown = send(port, Some("test-token"), call("terminal_execute", json!({"command": "ls", "session_id": "nope"})));
    assert!(text_of(&wait_reply(cx, &unknown).1).contains("Unknown session_id"));
    let empty = send(port, Some("test-token"), call("terminal_execute", json!({})));
    assert!(text_of(&wait_reply(cx, &empty).1).contains("`command` is required"));
}

#[gpui_kit::test]
fn the_server_follows_the_settings_and_keeps_a_discovery_file(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, port) = running_bridge(cx, PermissionMode::Auto);
    let store = cx.update(|cx| prefs_store::shared(cx));
    let b = cx.update(|cx| bridge(cx));
    let discovery = ops_ai::mcp_bridge::default_discovery_path(&fx.ctx.core.data_dir);
    assert_eq!(ops_ai::mcp_bridge::read_discovery(&discovery).unwrap().url, format!("http://127.0.0.1:{port}/mcp"));

    // HTTP off while external MCP stays on: the server moves to a free loopback port for the stdio launcher
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.external_mcp.http.enabled = false)));
    wait_until(cx, "moved to a loopback port", |cx| b.read_with(cx, |b, _| matches!(b.status(), BridgeStatus::Running(u) if !u.contains(&format!(":{port}/")))));
    std::thread::sleep(Duration::from_millis(200));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "the configured port is released");
    let found = ops_ai::mcp_bridge::read_discovery(&discovery).unwrap();
    assert!(found.url.starts_with("http://127.0.0.1:") && !found.url.contains(&format!(":{port}/")), "{}", found.url);
    let launcher = b.read_with(cx, |b, _| b.launcher_path()).expect("a launcher script is written");
    assert!(std::path::Path::new(&launcher).exists());

    // and back on
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.external_mcp.http.enabled = true)));
    wait_until(cx, "listening on the configured port again", |cx| b.read_with(cx, |b, _| matches!(b.status(), BridgeStatus::Running(u) if u.contains(&format!(":{port}/")))));
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.external_mcp.enabled = false)));
    cx.run_until_parked();
    assert_eq!(b.read_with(cx, |b, _| b.status().clone()), BridgeStatus::Disabled);
    assert!(!discovery.exists(), "the discovery file goes with the server");
    assert!(b.read_with(cx, |b, _| b.discovery_path()).is_none());
}

#[gpui_kit::test]
fn the_real_stdio_launcher_talks_to_the_running_app(cx: &mut TestAppContext) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let _s = serial();
    let (fx, _port) = running_bridge(cx, PermissionMode::Auto);
    let discovery = ops_ai::mcp_bridge::default_discovery_path(&fx.ctx.core.data_dir);
    // the launcher is this very executable in --mcp-stdio mode, started the way an MCP client starts it
    let mut child = Command::new(env!("CARGO_BIN_EXE_opsbatch"))
        .arg("--mcp-stdio")
        .env(ops_ai::mcp_bridge::DISCOVERY_ENV, &discovery)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let lines: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    std::thread::spawn(move || {
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(serde_json::from_str(&l).unwrap());
        }
    });
    let mut say = |v: Value| writeln!(stdin, "{v}").unwrap();
    say(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
    say(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    say(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    say(call("terminal_execute", json!({"command": "echo stdio-mark-3"})));
    wait_until(cx, "three answers through stdio", |_| lines.lock().unwrap().len() >= 3);
    let got = lines.lock().unwrap().clone();
    assert_eq!(got[0]["result"]["serverInfo"]["name"], "opsbatch-external");
    assert_eq!(got[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert!(text_of(&got[2]).contains("stdio-mark-3"), "{}", text_of(&got[2]));
    // the call shows up in the activity records like any other
    assert!(cx.update(|cx| activity(cx).read(cx).records().iter().any(|r| r.summary == "echo stdio-mark-3")));

    // with External MCP switched off the launcher explains instead of hanging
    let store = cx.update(|cx| prefs_store::shared(cx));
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.external_mcp.enabled = false)));
    cx.run_until_parked();
    say(json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}));
    wait_until(cx, "an explanation", |_| lines.lock().unwrap().len() >= 4);
    assert!(lines.lock().unwrap()[3]["error"]["message"].as_str().unwrap().contains("not running"));
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "the launcher exits cleanly when its input closes");
}
