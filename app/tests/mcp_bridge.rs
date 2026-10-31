//! The external MCP bridge end to end: a real HTTP client talks to the bridge, which drives a real local shell tab,
//! records everything in the activity window and asks for approval according to the permission mode.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

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
    running_bridge_seeded(cx, mode, |_| {})
}

/// Same, with `seed` run against the services before the UI exists (hosts the test needs).
fn running_bridge_seeded(cx: &mut TestAppContext, mode: PermissionMode, seed: impl FnOnce(&ops_core::Core)) -> (Fixture, u16) {
    let fx = common::setup(cx, seed);
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
    assert_eq!(wait_reply(cx, &list).1["result"]["tools"].as_array().unwrap().len(), ops_ai::mcp_bridge::tool_definitions().len());

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
    assert_eq!(got[1]["result"]["tools"].as_array().unwrap().len(), ops_ai::mcp_bridge::tool_definitions().len());
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

// ---------------------------------------------------------------------------
// SFTP, port forward and host-list tools against a private sshd
// ---------------------------------------------------------------------------

/// Calls a changing tool and approves it in the activity window.
fn call_approved(cx: &mut TestAppContext, port: u16, tool: &str, args: Value) -> Value {
    let activity = act(cx);
    let reply = send(port, Some("test-token"), call(tool, args));
    wait_until(cx, "approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::Approve, cx)));
    wait_reply(cx, &reply).1
}

fn call_plain(cx: &mut TestAppContext, port: u16, tool: &str, args: Value) -> Value {
    let reply = send(port, Some("test-token"), call(tool, args));
    wait_reply(cx, &reply).1
}

#[gpui_kit::test]
fn sftp_forward_and_host_tools_work_against_a_real_server(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (ssh_port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let (_fx, port) = running_bridge_seeded(cx, PermissionMode::Confirm, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            ops_ssh::hosts::NewHost { name: "mcp-srv".into(), ip: "127.0.0.1".into(), port: Some(ssh_port), auth_type: Some("key".into()), username: Some(user.clone()), password: None, private_key: Some(key.clone()), os: Some("linux".into()), tags: Some(r#"["prod","web"]"#.into()), group_id: None, remark: None, jump_chain: None, rdp_settings: None, proxy_settings: None },
        ))
        .unwrap()
        .id;
    });
    let wsp = cx.update(|cx| workspace(cx));
    let hid = host_id.clone();
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&hid, "mcp-srv", "127.0.0.1", cx)));
    wait_until(cx, "ssh tab connected", |cx| cx.update(|cx| wsp.read(cx).tabs().iter().any(|t| t.host_id.is_some() && t.state == ConnState::Connected)));
    cx.run_until_parked();

    let sessions: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "list_sessions", json!({})))).unwrap();
    let ssh_session = sessions.as_array().unwrap().iter().find(|s| s["type"] == "ssh").expect("an ssh session")["session_id"].as_str().unwrap().to_string();
    let activity = act(cx);

    let work = std::env::temp_dir().join(format!("opsbatch-mcp-tools-{}", uuid::Uuid::new_v4()));
    let dir = work.display().to_string();
    let file = format!("{dir}/notes.txt");
    let moved = format!("{dir}/moved.txt");

    // changing tools ask first: a rejected write leaves nothing behind
    let reply = send(port, Some("test-token"), call("sftp_mkdir", json!({"path": dir})));
    wait_until(cx, "mkdir approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::Reject, cx)));
    let v = wait_reply(cx, &reply).1;
    assert!(v["result"]["isError"] == true && text_of(&v).starts_with("Rejected"), "{}", text_of(&v));
    assert!(!work.exists());

    // approved: directory, file, listing, stat, read
    assert_eq!(call_approved(cx, port, "sftp_mkdir", json!({"path": dir}))["result"]["isError"], false, "mkdir");
    assert!(work.is_dir());
    let v = call_approved(cx, port, "sftp_write", json!({"path": file, "content": "hello\nwörld\n", "session_id": ssh_session}));
    assert_eq!(v["result"]["isError"], false, "{}", text_of(&v));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello\nwörld\n");
    let list: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "sftp_list", json!({"path": dir})))).unwrap();
    assert_eq!((list[0]["name"].as_str(), list[0]["type"].as_str(), list[0]["size"].as_u64()), (Some("notes.txt"), Some("file"), Some(13)));
    let stat: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "sftp_stat", json!({"path": file})))).unwrap();
    assert_eq!(stat["size"], 13);
    assert_eq!(text_of(&call_plain(cx, port, "sftp_read", json!({"path": file}))), "hello\nwörld\n");
    // reads are refused for binary files and for files over the limit
    std::fs::write(work.join("blob.bin"), [0xffu8, 0xfe, 0x00]).unwrap();
    let v = call_plain(cx, port, "sftp_read", json!({"path": format!("{dir}/blob.bin")}));
    assert!(v["result"]["isError"] == true && text_of(&v).contains("UTF-8"), "{}", text_of(&v));
    let v = call_plain(cx, port, "sftp_read", json!({"path": file, "max_bytes": 5}));
    assert_eq!(v["result"]["isError"], true);

    // rename and delete (a file, then the now empty directory after clearing it)
    assert_eq!(call_approved(cx, port, "sftp_rename", json!({"from": file, "to": moved}))["result"]["isError"], false);
    assert!(!std::path::Path::new(&file).exists() && std::path::Path::new(&moved).exists());
    assert_eq!(call_approved(cx, port, "sftp_delete", json!({"path": moved}))["result"]["isError"], false);
    let v = call_approved(cx, port, "sftp_delete", json!({"path": dir}));
    assert!(v["result"]["isError"] == true && text_of(&v).contains("empty"), "a directory with content is not deleted: {}", text_of(&v));
    std::fs::remove_file(work.join("blob.bin")).unwrap();
    assert_eq!(call_approved(cx, port, "sftp_delete", json!({"path": dir}))["result"]["isError"], false);
    assert!(!work.exists());

    // "always allow" remembers a capability grant: the next write needs no approval
    std::fs::create_dir_all(&work).unwrap();
    let reply = send(port, Some("test-token"), call("sftp_write", json!({"path": file, "content": "one"})));
    wait_until(cx, "write approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let id = cx.update(|cx| activity.read(cx).pending()[0].id.clone());
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&id, Decision::AlwaysAllow, cx)));
    assert_eq!(wait_reply(cx, &reply).1["result"]["isError"], false);
    let store = cx.update(|cx| prefs_store::shared(cx));
    assert_eq!(store.read_with(cx, |s, _| s.prefs().permission_grants.iter().map(|g| g.capability_id.clone()).collect::<Vec<_>>()), vec!["sftp.write"]);
    assert_eq!(call_plain(cx, port, "sftp_write", json!({"path": file, "content": "two"}))["result"]["isError"], false, "granted");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");

    // observer mode keeps the reads and refuses the changes
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.global_permission_mode = PermissionMode::Observer)));
    let v = call_plain(cx, port, "sftp_write", json!({"path": file, "content": "three"}));
    assert!(text_of(&v).starts_with("Refused by OpsBatch"), "{}", text_of(&v));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
    assert_eq!(call_plain(cx, port, "sftp_read", json!({"path": file}))["result"]["isError"], false);
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.global_permission_mode = PermissionMode::Auto)));

    // a local forward really carries traffic, shows in the list and is removed again
    let echo = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in echo.incoming().flatten() {
            let mut conn = conn;
            let mut buf = [0u8; 64];
            if let Ok(n) = conn.read(&mut buf) {
                let _ = conn.write_all(&buf[..n]);
            }
        }
    });
    let local_port = free_port();
    let v = call_plain(cx, port, "portforward_start", json!({"type": "local", "local_address": format!("127.0.0.1:{local_port}"), "remote_address": format!("127.0.0.1:{echo_port}"), "label": "mcp-test"}));
    assert_eq!(v["result"]["isError"], false, "{}", text_of(&v));
    let started: Value = serde_json::from_str(&text_of(&v)).unwrap();
    let forward_id = started["forward_id"].as_str().unwrap().to_string();
    let reached = Arc::new(Mutex::new(None::<String>));
    let r2 = reached.clone();
    std::thread::spawn(move || {
        for _ in 0..50 {
            if let Ok(mut c) = TcpStream::connect(("127.0.0.1", local_port)) {
                let _ = c.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = c.write_all(b"ping");
                let mut buf = [0u8; 8];
                if let Ok(n) = c.read(&mut buf) {
                    *r2.lock().unwrap() = Some(String::from_utf8_lossy(&buf[..n]).into_owned());
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    wait_until(cx, "traffic through the forward", |_| reached.lock().unwrap().is_some());
    assert_eq!(reached.lock().unwrap().as_deref(), Some("ping"));
    let forwards: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "portforward_list", json!({})))).unwrap();
    assert_eq!((forwards[0]["forward_id"].as_str(), forwards[0]["type"].as_str(), forwards[0]["label"].as_str()), (Some(forward_id.as_str()), Some("local"), Some("mcp-test")));
    let v = call_plain(cx, port, "portforward_start", json!({"type": "local", "local_address": "nonsense"}));
    assert!(text_of(&v).contains("host:port"), "{}", text_of(&v));
    let stopped = call_plain(cx, port, "portforward_stop", json!({"forward_id": forward_id}));
    assert_eq!(stopped["result"]["isError"], false, "{}", text_of(&stopped));
    let after: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "portforward_list", json!({})))).unwrap();
    assert_eq!((after.as_array().unwrap().len(), after[0]["status"].as_str()), (1, Some("suspended")), "a stopped forward stays listed");
    assert_eq!(call_plain(cx, port, "portforward_delete", json!({"forward_id": forward_id}))["result"]["isError"], false);
    assert_eq!(text_of(&call_plain(cx, port, "portforward_list", json!({}))).trim(), "[]");
    let v = call_plain(cx, port, "portforward_stop", json!({"forward_id": "nope"}));
    assert!(text_of(&v).contains("Unknown forward_id"));

    // the host list names the host and never carries credentials
    let text = text_of(&call_plain(cx, port, "vault_list_hosts", json!({})));
    let hosts: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((hosts[0]["name"].as_str(), hosts[0]["address"].as_str(), hosts[0]["username"].as_str(), hosts[0]["tags"].to_string()), (Some("mcp-srv"), Some("127.0.0.1"), Some(user.as_str()), r#"["prod","web"]"#.to_string()));
    assert!(!text.contains("PRIVATE KEY") && !text.contains("password") && !text.contains("private_key"), "{text}");
    assert_eq!(text_of(&call_plain(cx, port, "vault_list_hosts", json!({"query": "PROD"}))).matches("host_id").count(), 1);
    assert_eq!(text_of(&call_plain(cx, port, "vault_list_hosts", json!({"query": "nothing-like-this"}))).trim(), "[]");
    let scripts = call_plain(cx, port, "vault_list_scripts", json!({}));
    assert_eq!(scripts["result"]["isError"], false);
    assert!(serde_json::from_str::<Value>(&text_of(&scripts)).unwrap().is_array());

    // every call is in the records; nothing is left pending
    cx.update(|cx| {
        let a = activity.read(cx);
        assert!(a.pending().is_empty());
        assert!(a.records().iter().any(|r| r.tool == "sftp_write" && r.status == Status::Done && r.summary.ends_with("(3 bytes)")));
        assert!(a.records().iter().any(|r| r.tool == "sftp_mkdir" && r.status == Status::Denied));
        assert!(a.records().iter().all(|r| !r.summary.contains("wörld") && !r.detail.contains("wörld")), "file content never shows in a record");
    });
    let _ = std::fs::remove_dir_all(&work);
}

#[gpui_kit::test]
fn sftp_and_forward_tools_need_an_ssh_session(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, port) = running_bridge(cx, PermissionMode::Auto);
    for (tool, args) in [("sftp_list", json!({"path": "/"})), ("sftp_write", json!({"path": "/tmp/x", "content": "x"})), ("portforward_start", json!({"type": "dynamic", "local_address": "127.0.0.1:1080"}))] {
        let v = call_plain(cx, port, tool, args);
        assert_eq!(v["result"]["isError"], true, "{tool}");
        assert!(text_of(&v).contains("SSH session"), "{tool}: {}", text_of(&v));
    }
    let v = call_plain(cx, port, "sftp_list", json!({"path": "/", "session_id": "nope"}));
    assert!(text_of(&v).contains("Unknown session_id"));
    // reading the saved hosts needs no session
    assert_eq!(text_of(&call_plain(cx, port, "vault_list_hosts", json!({}))).trim(), "[]");
}

#[gpui_kit::test]
fn transfers_permissions_forward_changes_and_vault_writes_work_against_a_real_server(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (ssh_port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let (fx, port) = running_bridge_seeded(cx, PermissionMode::Confirm, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            ops_ssh::hosts::NewHost { name: "mcp-srv".into(), ip: "127.0.0.1".into(), port: Some(ssh_port), auth_type: Some("key".into()), username: Some(user.clone()), password: None, private_key: Some(key.clone()), os: Some("linux".into()), tags: None, group_id: None, remark: None, jump_chain: None, rdp_settings: None, proxy_settings: None },
        ))
        .unwrap()
        .id;
    });
    let wsp = cx.update(|cx| workspace(cx));
    let hid = host_id.clone();
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&hid, "mcp-srv", "127.0.0.1", cx)));
    wait_until(cx, "ssh tab connected", |cx| cx.update(|cx| wsp.read(cx).tabs().iter().any(|t| t.host_id.is_some() && t.state == ConnState::Connected)));
    cx.run_until_parked();
    let activity = act(cx);
    let store = cx.update(|cx| prefs_store::shared(cx));
    let set_mode = |cx: &mut TestAppContext, mode: PermissionMode| cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.global_permission_mode = mode)));

    let work = std::env::temp_dir().join(format!("opsbatch-mcp-xfer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).unwrap();
    let dir = work.display().to_string();

    // ---- upload: the approval names both paths; bad local paths are refused before anything is asked
    let local_src = format!("{dir}/local-src.txt");
    std::fs::write(&local_src, "uploaded bytes").unwrap();
    let remote = format!("{dir}/remote-dst.txt");
    let reply = send(port, Some("test-token"), call("sftp_upload", json!({"local_path": local_src, "remote_path": remote})));
    wait_until(cx, "upload approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let pending = cx.update(|cx| activity.read(cx).pending()[0].clone());
    assert_eq!(pending.summary, format!("{local_src} → {remote}"));
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&pending.id, Decision::Approve, cx)));
    let v = wait_reply(cx, &reply).1;
    assert_eq!(v["result"]["isError"], false, "{}", text_of(&v));
    assert_eq!(std::fs::read_to_string(&remote).unwrap(), "uploaded bytes");
    set_mode(cx, PermissionMode::Auto);
    for (args, needle) in [(json!({"local_path": "relative.txt", "remote_path": remote}), "absolute"), (json!({"local_path": dir, "remote_path": remote}), "regular file"), (json!({"local_path": format!("{dir}/missing"), "remote_path": remote}), "Cannot read")] {
        let v = call_plain(cx, port, "sftp_upload", args);
        assert!(v["result"]["isError"] == true && text_of(&v).contains(needle), "{needle}: {}", text_of(&v));
    }

    // ---- download: never replaces a local file unless told to
    let target = format!("{dir}/downloaded.txt");
    assert_eq!(call_plain(cx, port, "sftp_download", json!({"remote_path": remote, "local_path": target}))["result"]["isError"], false);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "uploaded bytes");
    std::fs::write(&target, "keep me").unwrap();
    let v = call_plain(cx, port, "sftp_download", json!({"remote_path": remote, "local_path": target}));
    assert!(text_of(&v).contains("overwrite=true"), "{}", text_of(&v));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    assert_eq!(call_plain(cx, port, "sftp_download", json!({"remote_path": remote, "local_path": target, "overwrite": true}))["result"]["isError"], false);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "uploaded bytes");
    let v = call_plain(cx, port, "sftp_download", json!({"remote_path": remote, "local_path": format!("{dir}/no-such-folder/x")}));
    assert!(text_of(&v).contains("existing folder"), "{}", text_of(&v));
    let v = call_plain(cx, port, "sftp_download", json!({"remote_path": format!("{dir}/absent"), "local_path": format!("{dir}/absent.txt")}));
    assert_eq!(v["result"]["isError"], true);

    // ---- permissions
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(call_plain(cx, port, "sftp_chmod", json!({"path": remote, "mode": "0600"}))["result"]["isError"], false);
    assert_eq!(std::fs::metadata(&remote).unwrap().permissions().mode() & 0o7777, 0o600);
    let v = call_plain(cx, port, "sftp_chmod", json!({"path": remote, "mode": "rwx"}));
    assert!(text_of(&v).contains("octal"), "{}", text_of(&v));
    // observer mode refuses all three
    set_mode(cx, PermissionMode::Observer);
    for (tool, args) in [("sftp_upload", json!({"local_path": local_src, "remote_path": remote})), ("sftp_download", json!({"remote_path": remote, "local_path": format!("{dir}/o.txt")})), ("sftp_chmod", json!({"path": remote, "mode": "777"}))] {
        let v = call_plain(cx, port, tool, args);
        assert!(text_of(&v).starts_with("Refused by OpsBatch"), "{tool}: {}", text_of(&v));
    }
    assert_eq!(std::fs::metadata(&remote).unwrap().permissions().mode() & 0o7777, 0o600);
    set_mode(cx, PermissionMode::Auto);

    // ---- forward changes: update replaces, duplicate adds, delete removes; bad input leaves things alone
    let (p1, p2, p3) = (free_port(), free_port(), free_port());
    let started = call_plain(cx, port, "portforward_start", json!({"type": "local", "local_address": format!("127.0.0.1:{p1}"), "remote_address": "127.0.0.1:9", "label": "first"}));
    let first: Value = serde_json::from_str(&text_of(&started)).unwrap();
    let id1 = first["forward_id"].as_str().unwrap().to_string();
    let updated = call_plain(cx, port, "portforward_update", json!({"forward_id": id1, "local_address": format!("127.0.0.1:{p2}"), "label": "renamed"}));
    assert_eq!(updated["result"]["isError"], false, "{}", text_of(&updated));
    let moved: Value = serde_json::from_str(&text_of(&updated)).unwrap();
    let id2 = moved["forward_id"].as_str().unwrap().to_string();
    assert_ne!(id1, id2, "an update starts a new forward");
    assert_eq!((moved["label"].as_str(), moved["remote_address"].as_str()), (Some("renamed"), Some("127.0.0.1:9")), "unchanged fields are kept");
    let v = call_plain(cx, port, "portforward_update", json!({"forward_id": id2, "local_address": "nonsense"}));
    assert!(text_of(&v).contains("host:port"));
    let v = call_plain(cx, port, "portforward_update", json!({"forward_id": id1}));
    assert!(text_of(&v).contains("Unknown forward_id"), "the replaced forward is gone: {}", text_of(&v));
    let copy = call_plain(cx, port, "portforward_duplicate", json!({"forward_id": id2, "local_address": format!("127.0.0.1:{p3}"), "label": "copy"}));
    assert_eq!(copy["result"]["isError"], false, "{}", text_of(&copy));
    let copy: Value = serde_json::from_str(&text_of(&copy)).unwrap();
    let list: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "portforward_list", json!({})))).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    for id in [id2.as_str(), copy["forward_id"].as_str().unwrap()] {
        assert_eq!(call_plain(cx, port, "portforward_delete", json!({"forward_id": id}))["result"]["isError"], false);
    }
    assert_eq!(text_of(&call_plain(cx, port, "portforward_list", json!({}))).trim(), "[]");

    // ---- vault: groups, hosts, scripts. The approval names the object, not its id
    set_mode(cx, PermissionMode::Confirm);
    let group: Value = serde_json::from_str(&text_of(&call_approved(cx, port, "vault_create_group", json!({"name": "Production"})))).unwrap();
    let group_id = group["group_id"].as_str().unwrap().to_string();
    let sub: Value = serde_json::from_str(&text_of(&call_approved(cx, port, "vault_create_group", json!({"name": "Web", "parent_id": group_id})))).unwrap();
    let sub_id = sub["group_id"].as_str().unwrap().to_string();
    assert_eq!(sub["parent_id"], group_id.as_str());
    let v = call_approved(cx, port, "vault_update_group", json!({"group_id": group_id, "parent_id": sub_id}));
    assert!(text_of(&v).contains("below itself"), "{}", text_of(&v));
    assert_eq!(call_approved(cx, port, "vault_update_group", json!({"group_id": sub_id, "name": "Frontend"}))["result"]["isError"], false);
    let groups: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_list_groups", json!({})))).unwrap();
    assert!(groups.as_array().unwrap().iter().any(|g| g["name"] == "Frontend" && g["parent_id"] == group_id.as_str()));

    let created: Value = serde_json::from_str(&text_of(&call_approved(cx, port, "vault_create_host", json!({"name": "db-1", "address": "10.0.0.5", "port": 2222, "username": "deploy", "tags": ["db", "prod"], "group_id": group_id, "remark": "primary"})))).unwrap();
    let new_host = created["host_id"].as_str().unwrap().to_string();
    assert_eq!((created["port"].as_i64(), created["username"].as_str(), created["auth_type"].as_str()), (Some(2222), Some("deploy"), Some("password")));
    // the sidebar knows about it without a manual refresh
    wait_until(cx, "the sidebar shows the new host", |cx| cx.update(|cx| opsbatch_app::stores::assets::shared(cx).is_some_and(|a| a.read(cx).host(&new_host).is_some())));
    set_mode(cx, PermissionMode::Auto);
    for bad in [json!({"name": "x", "address": "y", "port": 0}), json!({"name": "x", "address": "y", "auth_type": "magic"}), json!({"name": "x", "address": "y", "group_id": "nope"}), json!({"name": "x", "address": "y", "tags": "a"}), json!({"name": " ", "address": "y"})] {
        let v = call_plain(cx, port, "vault_create_host", bad.clone());
        assert_eq!(v["result"]["isError"], true, "{bad}");
    }
    let hosts: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_list_hosts", json!({"query": "production"})))).unwrap();
    assert_eq!((hosts.as_array().unwrap().len(), hosts[0]["name"].as_str(), hosts[0]["group"].as_str()), (1, Some("db-1"), Some("Production")));

    // an update changes only what it names and never touches the credentials
    let before = ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db)).unwrap().into_iter().find(|h| h.id == host_id).unwrap();
    let v = call_plain(cx, port, "vault_update_host", json!({"host_id": host_id, "remark": "managed by MCP", "tags": ["x"], "group_id": group_id}));
    assert_eq!(v["result"]["isError"], false, "{}", text_of(&v));
    let notes = call_plain(cx, port, "vault_set_host_notes", json!({"host_id": host_id, "notes": "second note"}));
    assert_eq!(notes["result"]["isError"], false, "{}", text_of(&notes));
    let after = ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db)).unwrap().into_iter().find(|h| h.id == host_id).unwrap();
    assert_eq!((after.remark.as_str(), after.group_id.as_deref(), after.tags.as_str()), ("second note", Some(group_id.as_str()), r#"["x"]"#));
    assert_eq!((after.port, after.username.as_str(), after.auth_type.as_str(), after.ip.as_str()), (before.port, before.username.as_str(), before.auth_type.as_str(), before.ip.as_str()));
    assert!(after.private_key.is_some() && after.private_key == before.private_key, "the stored key is kept");
    assert!(!text_of(&v).contains("PRIVATE KEY"));
    let v = call_plain(cx, port, "vault_update_host", json!({"host_id": host_id, "group_id": ""}));
    assert_eq!(v["result"]["isError"], false);
    assert!(call_plain(cx, port, "vault_update_host", json!({"host_id": "nope", "name": "x"}))["result"]["isError"] == true);

    // deleting asks first, and the approval shows the host's name
    set_mode(cx, PermissionMode::Confirm);
    let reply = send(port, Some("test-token"), call("vault_delete_host", json!({"host_id": new_host})));
    wait_until(cx, "delete approval requested", |cx| cx.update(|cx| activity.read(cx).pending().len() == 1));
    let pending = cx.update(|cx| activity.read(cx).pending()[0].clone());
    assert!(pending.summary.contains("db-1") && pending.summary.contains("deploy@10.0.0.5"), "{}", pending.summary);
    cx.update(|cx| activity.update(cx, |a, cx| a.decide(&pending.id, Decision::Reject, cx)));
    assert!(text_of(&wait_reply(cx, &reply).1).starts_with("Rejected"));
    assert!(ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db)).unwrap().iter().any(|h| h.id == new_host), "a rejected delete keeps the host");
    assert_eq!(call_approved(cx, port, "vault_delete_host", json!({"host_id": new_host}))["result"]["isError"], false);
    assert!(!ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db)).unwrap().iter().any(|h| h.id == new_host));
    wait_until(cx, "the sidebar drops the host", |cx| cx.update(|cx| opsbatch_app::stores::assets::shared(cx).is_some_and(|a| a.read(cx).host(&new_host).is_none())));
    // deleting a group keeps its hosts, now ungrouped
    assert_eq!(call_approved(cx, port, "vault_delete_group", json!({"group_id": group_id}))["result"]["isError"], false);
    let still = ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db)).unwrap().into_iter().find(|h| h.id == host_id).unwrap();
    assert_eq!(still.group_id, None);

    // scripts: the library keeps the text, records and approvals never show it
    set_mode(cx, PermissionMode::Auto);
    let made: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_create_script", json!({"name": "Restart web", "content": "systemctl restart nginx\n", "language": "shell", "risk": "medium", "tags": ["ops"], "description": "restarts nginx"})))).unwrap();
    let sid = made["script_id"].as_str().unwrap().to_string();
    assert_eq!((made["risk"].as_str(), made["language"].as_str(), made["builtin"].as_bool()), (Some("medium"), Some("shell"), Some(false)));
    let got: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_get_script", json!({"script_id": sid})))).unwrap();
    assert_eq!(got["content"], "systemctl restart nginx\n");
    assert!(!text_of(&call_plain(cx, port, "vault_list_scripts", json!({"query": "restart"}))).contains("systemctl"), "the list has no script text");
    let v = call_plain(cx, port, "vault_update_script", json!({"script_id": sid, "content": "systemctl reload nginx\n", "risk": "low"}));
    assert_eq!(v["result"]["isError"], false, "{}", text_of(&v));
    let got: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_get_script", json!({"script_id": sid})))).unwrap();
    assert_eq!((got["content"].as_str(), got["risk"].as_str(), got["name"].as_str()), (Some("systemctl reload nginx\n"), Some("low"), Some("Restart web")));
    assert!(call_plain(cx, port, "vault_update_script", json!({"script_id": sid, "risk": "extreme"}))["result"]["isError"] == true);
    assert_eq!(call_plain(cx, port, "vault_delete_script", json!({"script_id": sid}))["result"]["isError"], false);
    assert!(call_plain(cx, port, "vault_get_script", json!({"script_id": sid}))["result"]["isError"] == true);
    let all: Value = serde_json::from_str(&text_of(&call_plain(cx, port, "vault_list_scripts", json!({})))).unwrap();
    if let Some(builtin) = all.as_array().unwrap().iter().find(|s| s["builtin"] == true) {
        let bid = builtin["script_id"].as_str().unwrap();
        assert!(text_of(&call_plain(cx, port, "vault_update_script", json!({"script_id": bid, "name": "x"}))).contains("Built-in"));
        assert!(text_of(&call_plain(cx, port, "vault_delete_script", json!({"script_id": bid}))).contains("Built-in"));
    }
    cx.update(|cx| {
        let a = activity.read(cx);
        assert!(a.pending().is_empty());
        assert!(a.records().iter().all(|r| !r.summary.contains("systemctl") && !r.detail.contains("systemctl") && !r.summary.contains("second note")), "text and notes never show in a record");
    });
    let _ = std::fs::remove_dir_all(&work);
}
