//! Terminal workspace against real shells: a local PTY and a private sshd (skipped when OpenSSH is missing).

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::{Entity, TestAppContext};
use opsbatch_app::terminal::pane::TerminalPane;
use opsbatch_app::terminal::workspace::{workspace, ConnState, TerminalWorkspace};
use ops_ssh::hosts::NewHost;

use common::{serial, wait_until};

fn ws(cx: &mut TestAppContext) -> Entity<TerminalWorkspace> {
    cx.update(|cx| workspace(cx))
}

fn active_pane(cx: &mut TestAppContext) -> Option<Entity<TerminalPane>> {
    let w = ws(cx);
    cx.update(|cx| w.read(cx).active_tab().and_then(|t| t.pane.clone()))
}

fn pane_has(cx: &mut TestAppContext, needle: &str) -> bool {
    match active_pane(cx) {
        Some(p) => cx.update(|cx| p.read(cx).buffer_text().contains(needle)),
        None => false,
    }
}

fn type_line(cx: &mut TestAppContext, line: &str) {
    let p = active_pane(cx).expect("active pane");
    cx.update(|cx| p.update(cx, |p, cx| p.send_user_text(&format!("{line}\r"), cx)));
}

#[gpui_kit::test]
fn local_shell_tab_connects_and_runs_commands(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let key = cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)));
    cx.update(|cx| assert_eq!(w.read(cx).tab(&key).unwrap().state, ConnState::Connecting));
    wait_until(cx, "local shell connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().state == ConnState::Connected));

    type_line(cx, "echo OPS$((20+22))END");
    wait_until(cx, "command output", |cx| pane_has(cx, "OPS42END"));

    // Close: the session is torn down and the tab is gone.
    cx.update(|cx| w.update(cx, |w, cx| w.close_tab(&key, cx)));
    cx.update(|cx| assert!(w.read(cx).tabs().is_empty() && w.read(cx).active_key().is_none()));
}

#[gpui_kit::test]
fn tracked_execute_command_returns_exit_code_and_clean_output(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let key = cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)));
    wait_until(cx, "connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().state == ConnState::Connected));
    let pane = active_pane(cx).unwrap();
    // Let the shell print its prompt first so the tracked command is not typed into startup noise.
    std::thread::sleep(std::time::Duration::from_millis(800));
    cx.run_until_parked();

    let mut rx = cx.update(|cx| pane.update(cx, |p, cx| p.execute_command("echo tracked-$((6*7)); false", None, cx)));
    let mut result = None;
    wait_until(cx, "tracked command finished", |_| {
        if let Ok(Some(r)) = rx.try_recv() {
            result = Some(r);
        }
        result.is_some()
    });
    let r = result.unwrap();
    assert_eq!(r.status, opsbatch_app::terminal::executions::ExecStatus::Completed);
    assert_eq!(r.exit_code, Some(1));
    assert!(r.output.contains("tracked-42"), "{:?}", r.output);
    assert!(!r.output.contains("__OPSBATCH_AI_DONE_"), "tracker artifacts are stripped: {:?}", r.output);
    // The marker never reaches the visible terminal either.
    cx.update(|cx| assert!(!pane.read(cx).buffer_text().contains("__OPSBATCH_AI_DONE_")));
}

#[gpui_kit::test]
fn dangerous_command_is_blocked_until_confirmed(cx: &mut TestAppContext) {
    let _s = serial();
    let dir = std::env::temp_dir().join(format!("opsbatch-danger-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let key = cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)));
    wait_until(cx, "connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().state == ConnState::Connected));
    let pane = active_pane(cx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    cx.run_until_parked();

    // "rm -rf /<dir>" matches the built-in rule; the first Enter must not reach the shell.
    let victim = dir.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    let cmd = format!("rm -rf {}", victim.display());
    cx.update(|cx| pane.update(cx, |p, cx| p.send_user_text(&cmd, cx)));
    cx.update(|cx| pane.update(cx, |p, cx| p.send_user_text("\r", cx)));
    wait_until(cx, "warning shown", |cx| pane_has(cx, "OpsBatch"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(victim.exists(), "blocked: nothing deleted yet");

    // Second Enter confirms.
    cx.update(|cx| pane.update(cx, |p, cx| p.send_user_text("\r", cx)));
    let start = std::time::Instant::now();
    while victim.exists() && start.elapsed() < std::time::Duration::from_secs(10) {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!victim.exists(), "confirmed command ran");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn ssh_tab_connects_and_failure_shows_error_state(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd available");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let port = sshd.port;
    let key_pem = sshd.private_key.clone();
    let mut host_ids = Vec::new();
    let fx = common::setup(cx, |core| {
        for (name, p) in [("good", port as i32), ("dead", 1)] {
            let h = ops_core::rt::block_on(ops_ssh::hosts::add_host(
                &core.db,
                NewHost {
                    name: name.into(),
                    ip: "127.0.0.1".into(),
                    port: Some(p),
                    auth_type: Some("key".into()),
                    username: Some(user.clone()),
                    password: None,
                    private_key: Some(key_pem.clone()),
                    os: Some("linux".into()),
                    tags: None,
                    group_id: None,
                    remark: None,
                    jump_chain: None,
                    rdp_settings: None,
                    proxy_settings: None,
                },
            ))
            .unwrap();
            host_ids.push(h.id);
        }
    });
    let _ = &fx;
    let w = ws(cx);

    // Opening the same host twice focuses the existing tab.
    let first = cx.update(|cx| w.update(cx, |w, cx| w.open_host(&host_ids[0], "good", "127.0.0.1", cx)));
    let second = cx.update(|cx| w.update(cx, |w, cx| w.open_host(&host_ids[0], "good", "127.0.0.1", cx)));
    assert!(first && !second);
    cx.update(|cx| assert_eq!(w.read(cx).tabs().len(), 1));
    wait_until(cx, "ssh connected", |cx| cx.update(|cx| w.read(cx).tabs()[0].state == ConnState::Connected));
    type_line(cx, "echo SSH$((1+2))OK");
    wait_until(cx, "ssh output", |cx| pane_has(cx, "SSH3OK"));

    // Unreachable host: error state with a message, retry goes back to connecting.
    cx.update(|cx| w.update(cx, |w, cx| w.open_host(&host_ids[1], "dead", "127.0.0.1", cx)));
    wait_until(cx, "error state", |cx| cx.update(|cx| w.read(cx).tabs()[1].state == ConnState::Error));
    cx.update(|cx| {
        let tab = &w.read(cx).tabs()[1];
        assert!(tab.error.as_deref().is_some_and(|e| !e.is_empty()));
        assert!(tab.pane.is_none());
    });
    let dead_key = cx.update(|cx| w.read(cx).tabs()[1].key.clone());
    cx.update(|cx| w.update(cx, |w, cx| w.retry(&dead_key, cx)));
    cx.update(|cx| assert_eq!(w.read(cx).tab(&dead_key).unwrap().state, ConnState::Connecting));
    wait_until(cx, "error again", |cx| cx.update(|cx| w.read(cx).tab(&dead_key).unwrap().state == ConnState::Error));

    // The backend reports an idle disconnect for the live host: its tab turns into an error with the idle message.
    let live = host_ids[0].clone();
    let ctx = fx.ctx.clone();
    ctx.core.emit("connection_status_changed", serde_json::json!({"hostId": live, "status": "idle_disconnected"}));
    wait_until(cx, "idle disconnect applied", |cx| cx.update(|cx| w.read(cx).tabs()[0].state == ConnState::Error));
    cx.update(|cx| assert_eq!(w.read(cx).tabs()[0].error.as_deref(), Some(ops_i18n::t!("terminal.idleDisconnected").as_str())));
}

#[gpui_kit::test]
fn split_opens_a_second_session_and_closes_it(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let key = cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)));
    wait_until(cx, "connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().state == ConnState::Connected));
    cx.update(|cx| w.update(cx, |w, cx| w.open_split(&key, cx)));
    cx.update(|cx| assert_eq!(w.read(cx).tab(&key).unwrap().split.as_ref().unwrap().state, ConnState::Connecting));
    wait_until(cx, "split connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().split.as_ref().unwrap().state == ConnState::Connected));
    let (a, b) = cx.update(|cx| {
        let t = w.read(cx).tab(&key).unwrap();
        (t.pane.as_ref().unwrap().read(cx).session_id.clone(), t.split.as_ref().unwrap().pane.as_ref().unwrap().read(cx).session_id.clone())
    });
    assert_ne!(a, b);
    // A tab cannot be split twice.
    cx.update(|cx| w.update(cx, |w, cx| w.open_split(&key, cx)));
    cx.update(|cx| w.update(cx, |w, cx| w.close_split(&key, cx)));
    cx.update(|cx| assert!(w.read(cx).tab(&key).unwrap().split.is_none()));
}

#[gpui_kit::test]
fn closing_tabs_by_mode_picks_the_right_neighbour(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let keys: Vec<String> = (0..4).map(|_| cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)))).collect();
    wait_until(cx, "all connected", |cx| cx.update(|cx| w.read(cx).tabs().iter().all(|t| t.state == ConnState::Connected)));
    cx.update(|cx| w.update(cx, |w, cx| w.set_active(&keys[1], cx)));

    use ops_terminal::tabs::TabCloseMode;
    cx.update(|cx| w.update(cx, |w, cx| w.close_by_mode(&keys[1], TabCloseMode::Right, cx)));
    cx.update(|cx| {
        let r = w.read(cx);
        assert_eq!(r.tabs().iter().map(|t| t.key.clone()).collect::<Vec<_>>(), keys[..2].to_vec());
        assert_eq!(r.active_key(), Some(keys[1].as_str()));
    });
    cx.update(|cx| w.update(cx, |w, cx| w.close_by_mode(&keys[1], TabCloseMode::Left, cx)));
    cx.update(|cx| assert_eq!(w.read(cx).tabs().len(), 1));
    cx.update(|cx| w.update(cx, |w, cx| w.close_by_mode(&keys[1], TabCloseMode::All, cx)));
    cx.update(|cx| assert!(w.read(cx).tabs().is_empty()));
}

#[gpui_kit::test]
fn opsbatch_bg_runs_detached_and_reports_pid_and_log(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let w = ws(cx);
    let key = cx.update(|cx| w.update(cx, |w, cx| w.open_local(cx)));
    wait_until(cx, "connected", |cx| cx.update(|cx| w.read(cx).tab(&key).unwrap().state == ConnState::Connected));
    std::thread::sleep(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    type_line(cx, "opsbatch bg echo BGJOB$((20+22))");
    wait_until(cx, "pid reported", |cx| pane_has(cx, "PID="));
    // The report names the log file; the detached job wrote to it.
    let pane = active_pane(cx).unwrap();
    let text = cx.update(|cx| pane.read(cx).buffer_text());
    let log = text.lines().find_map(|l| l.split_once(": ").filter(|(k, _)| k.contains("日志") || k.contains("Log")).map(|(_, v)| v.trim().to_string())).expect("log path line");
    let start = std::time::Instant::now();
    loop {
        if std::fs::read_to_string(&log).map(|c| c.contains("BGJOB42")).unwrap_or(false) {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "background job output missing in {log}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // The shell line was cleared, not executed as `opsbatch`.
    assert!(!text.contains("command not found"), "{text}");
}
