//! AI assistant through the real window: chat streaming, command plans, approval + terminal execution + follow-up,
//! policy blocking, history, ⌘I inline panel, selection analysis, settings and the batch-window risk check
//! (headless GPUI, local PTY, mock OpenAI-compatible provider on localhost).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use ops_ai::client::{ai_assess_action, save_ai_config, AiActionDecision, AiConfig};
use opsbatch_app::ai::panel::AiChatPanel;
use opsbatch_app::ai::store::{ai_store, Role};
use opsbatch_app::batch_exec::window::BatchCommandsWindow;
use opsbatch_app::terminal::page::{BottomTab, TerminalPage};
use opsbatch_app::terminal::pane::TerminalPane;
use opsbatch_app::terminal::workspace::{workspace, ConnState, TerminalWorkspace};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use common::{serial, wait_until, Fixture};

// ---------------------------------------------------------------------------
// Mock provider
// ---------------------------------------------------------------------------

struct Mock {
    url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl Mock {
    fn requests(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }

    /// Every user/system message text of every chat request, flattened.
    fn all_text(&self) -> String {
        self.requests()
            .iter()
            .flat_map(|b| b["messages"].as_array().cloned().unwrap_or_default())
            .filter_map(|m| m["content"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n---\n")
    }
}

fn plan(command: &str) -> String {
    format!(
        "我来处理。\n<COMMAND_PLAN>\n{}\n</COMMAND_PLAN>",
        serde_json::json!({"version": 1, "summary": "s", "steps": [{"description": "打印标记", "command": command, "intent": "observe", "expectedOutcome": "42"}]})
    )
}

fn reply_for(last_user: &str) -> String {
    if last_user.contains("[自动跟进]") {
        "已确认输出。".into()
    } else if last_user.contains("磁盘") {
        plan("echo ai-mark-42")
    } else if last_user.contains("危险") {
        plan("rm -rf /")
    } else if last_user.contains("ls 命令") {
        "ls -la".into()
    } else {
        "你好，世界".into()
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut header_end = None;
    loop {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if header_end.is_none() {
            header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
        }
        if let Some(he) = header_end {
            let head = String::from_utf8_lossy(&buf[..he]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= he + len {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (
        head.lines().next().unwrap_or("").to_string(),
        body.to_string(),
    )
}

fn start_mock() -> Mock {
    let listener = ops_core::rt::block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
    let store = bodies.clone();
    ops_core::rt::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let store = store.clone();
            tokio::spawn(async move {
                let (line, body) = read_request(&mut sock).await;
                if line.contains("/models") {
                    let b = r#"{"data":[{"id":"m-b","owned_by":"x"},{"id":"m-a"}]}"#;
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                    return;
                }
                let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                store.lock().unwrap().push(json.clone());
                let last_user = json["messages"]
                    .as_array()
                    .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .to_string();
                let reply = reply_for(&last_user);
                if json["stream"] == true {
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await;
                    let chars: Vec<char> = reply.chars().collect();
                    for piece in chars.chunks(12) {
                        let delta: String = piece.iter().collect();
                        let line = format!(
                            "data: {}\n\n",
                            serde_json::json!({"model": "mock-1", "choices": [{"delta": {"content": delta}}]})
                        );
                        if sock.write_all(line.as_bytes()).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                } else {
                    let b = serde_json::json!({"model": "mock-1", "choices": [{"message": {"content": "连接成功"}}], "usage": {"total_tokens": 3}}).to_string();
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                }
            });
        }
    });
    Mock { url, bodies }
}

fn configure_ai(fx: &Fixture, mock: &Mock) {
    let db = fx.ctx.db.clone();
    let url = mock.url.clone();
    ops_core::rt::block_on(async move {
        save_ai_config(
            &db,
            AiConfig {
                provider: "ollama".into(),
                api_url: url,
                api_key: String::new(),
                model: "mock-1".into(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ws(cx: &mut TestAppContext) -> Entity<TerminalWorkspace> {
    cx.update(|cx| workspace(cx))
}

fn page(cx: &mut TestAppContext, fx: &Fixture) -> Entity<TerminalPage> {
    cx.update(|cx| {
        fx.workbench
            .read(cx)
            .terminal_page()
            .expect("terminal page")
            .clone()
    })
}

fn active_pane(cx: &mut TestAppContext) -> Entity<TerminalPane> {
    let w = ws(cx);
    cx.update(|cx| {
        w.read(cx)
            .active_tab()
            .and_then(|t| t.pane.clone())
            .expect("active pane")
    })
}

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

/// A configured AI, a connected local shell and the AI tab open. Returns (mock, page, panel, session id).
fn ai_fixture(
    cx: &mut TestAppContext,
) -> (
    Fixture,
    Mock,
    Entity<TerminalPage>,
    Entity<AiChatPanel>,
    String,
) {
    let fx = common::setup(cx, |_| {});
    let mock = start_mock();
    configure_ai(&fx, &mock);
    win(cx, &fx, |w, cx| w.click("terminal-reopen-local", cx));
    let wsp = ws(cx);
    wait_until(cx, "local tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    std::thread::sleep(Duration::from_millis(600));
    cx.run_until_parked();
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("bottom-tab-ai", cx);
        w.render_frame(cx);
    });
    let tp = page(cx, &fx);
    cx.update(|cx| assert_eq!(tp.read(cx).bottom_tab(), BottomTab::Ai));
    let panel = cx
        .update(|cx| tp.read(cx).ai_panel().cloned())
        .expect("AI panel exists once the tab is shown");
    let sid = active_pane(cx).read_with(cx, |p, _| p.session_id.clone());
    (fx, mock, tp, panel, sid)
}

fn send_text(cx: &mut TestAppContext, fx: &Fixture, panel: &Entity<AiChatPanel>, text: &str) {
    win(cx, fx, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input()
                .update(cx, |i, cx| i.set_value(text.to_string(), w, cx))
        });
        w.render_frame(cx);
        w.click("ai-send", cx);
    });
}

fn session_idle(cx: &mut TestAppContext, sid: &str) -> bool {
    cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(sid)
            .is_some_and(|s| !s.streaming())
    })
}

fn wait_idle(cx: &mut TestAppContext, sid: &str) {
    wait_until(cx, "reply finished", |cx| session_idle(cx, sid));
}

fn pending_with_assessment(cx: &mut TestAppContext, sid: &str) -> bool {
    cx.update(|cx| {
        ai_store(cx).read(cx).session(sid).is_some_and(|s| {
            s.pending_actions()
                .first()
                .is_some_and(|a| a.assessment.is_some())
        })
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn chat_streams_a_plan_runs_the_approved_command_and_follows_up(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, mock, _tp, panel, sid) = ai_fixture(cx);

    // Type with the keyboard and send with Enter.
    win(cx, &fx, |w, cx| {
        panel.update(cx, |p, cx| p.input().update(cx, |i, cx| i.focus(w, cx)));
        w.input("检查磁盘", cx);
        w.press("enter", cx);
    });
    wait_idle(cx, &sid);
    wait_until(cx, "plan parsed and assessed", |cx| {
        pending_with_assessment(cx, &sid)
    });

    cx.update(|cx| {
        let store = ai_store(cx);
        let s = store.read(cx).session(&sid).unwrap();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].role, Role::User);
        assert_eq!(s.messages[0].content, "检查磁盘");
        assert_eq!(
            s.messages[1].content, "我来处理。",
            "the plan block is removed from the message text"
        );
        let a = s.pending_actions()[0].clone();
        assert_eq!(a.command.as_deref(), Some("echo ai-mark-42"));
        assert_eq!(a.description, "打印标记");
        assert!(!a.assessment_loading);
        assert!(s.input_text.is_empty(), "sending clears the input");
    });
    // The prompt carries the working principles and the single-command rule.
    assert!(mock.all_text().contains("steps 必须恰好包含一个步骤"));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("ai-approve-0").is_some() && w.try_find("ai-reject-0").is_some());
        assert!(w.try_find("ai-cmd-badge").is_some());
        w.click("ai-approve-0", cx);
    });

    // The command runs in the real terminal, is marked executed, and the output goes back to the model.
    wait_until(cx, "command executed", |cx| {
        cx.update(|cx| {
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .is_some_and(|s| s.executed_actions().len() == 1)
        })
    });
    wait_until(cx, "follow-up answered", |cx| {
        cx.update(|cx| {
            ai_store(cx).read(cx).session(&sid).is_some_and(|s| {
                !s.streaming() && s.messages.len() == 4 && s.messages[3].content == "已确认输出。"
            })
        })
    });
    assert!(active_pane(cx).read_with(cx, |p, _| p.buffer_text().contains("ai-mark-42")));
    let text = mock.all_text();
    assert!(text.contains("[自动跟进]"), "{text}");
    assert!(
        text.contains("[当前命令：completed，退出码 0]") && text.contains("ai-mark-42"),
        "{text}"
    );
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("ai-cmd-done-0").is_some(),
            "executed commands move to the done section"
        );
        assert!(w.try_find("ai-approve-0").is_none());
    });
    cx.update(|cx| {
        let store = ai_store(cx);
        let s = store.read(cx).session(&sid).unwrap();
        assert!(s.pending_actions().is_empty());
        assert_eq!(s.messages[2].role, Role::User);
        assert!(s.messages[2].content.starts_with("[自动跟进]"));
    });
    let _ = panel;
}

#[gpui_kit::test]
fn dangerous_commands_are_blocked_and_rejections_are_audited(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, _mock, _tp, panel, sid) = ai_fixture(cx);
    send_text(cx, &fx, &panel, "执行危险操作");
    wait_idle(cx, &sid);
    wait_until(cx, "assessed", |cx| pending_with_assessment(cx, &sid));
    cx.update(|cx| {
        let a = ai_store(cx)
            .read(cx)
            .session(&sid)
            .unwrap()
            .pending_actions()[0]
            .clone();
        assert_eq!(a.command.as_deref(), Some("rm -rf /"));
        assert_eq!(a.assessment.unwrap().decision, AiActionDecision::Block);
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("ai-approve-0", cx); // disabled: nothing happens
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .unwrap()
                .pending_actions()
                .len(),
            1
        )
    });
    panel.read_with(cx, |p, _| assert_eq!(p.executing_count(), 0));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("ai-reject-0", cx);
        w.render_frame(cx);
    });
    cx.update(|cx| {
        assert!(ai_store(cx)
            .read(cx)
            .session(&sid)
            .unwrap()
            .pending_actions()
            .is_empty())
    });

    let db = fx.ctx.db.clone();
    wait_until(cx, "audit event written", |_| {
        let conn = db.pool.get().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM ai_action_audit WHERE event = 'rejected' AND decision = 'BLOCK' AND command = 'rm -rf /'", [], |r| r.get(0)).unwrap();
        n == 1
    });
    // The terminal never saw the command.
    assert!(!active_pane(cx).read_with(cx, |p, _| p.buffer_text().contains("rm -rf")));
}

#[gpui_kit::test]
fn history_lists_conversations_and_reopens_them(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, _mock, _tp, panel, sid) = ai_fixture(cx);
    send_text(cx, &fx, &panel, "你好");
    wait_idle(cx, &sid);
    wait_until(cx, "history loaded", |cx| {
        cx.update(|cx| ai_store(cx).read(cx).conversations_of(&sid).len() == 1)
    });
    cx.update(|cx| {
        let store = ai_store(cx);
        let store = store.read(cx);
        let conv = &store.conversations_of(&sid)[0];
        assert_eq!(conv.title, "你好");
        assert_eq!(conv.scope, "terminal_session");
        assert_eq!(conv.scope_id, sid);
    });

    win(cx, &fx, |w, cx| {
        w.click("ai-new", cx);
        w.render_frame(cx);
    });
    cx.update(|cx| {
        assert!(ai_store(cx)
            .read(cx)
            .session(&sid)
            .unwrap()
            .messages
            .is_empty())
    });

    win(cx, &fx, |w, cx| {
        w.click("ai-history-toggle", cx);
        w.render_frame(cx);
        assert!(w.try_find("ai-history-item-0").is_some());
        w.click("ai-history-item-0", cx);
    });
    wait_until(cx, "conversation reopened", |cx| {
        cx.update(|cx| {
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .is_some_and(|s| s.messages.len() == 2)
        })
    });
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&sid).unwrap().clone();
        assert_eq!(s.messages[0].content, "你好");
        assert_eq!(s.messages[1].content, "你好，世界");
        assert!(s.active_conversation_id.is_some());
    });

    // Delete from the history list.
    win(cx, &fx, |w, cx| {
        w.click("ai-history-toggle", cx);
        w.render_frame(cx);
        w.click("ai-history-del-0", cx);
    });
    wait_until(cx, "conversation deleted", |cx| {
        cx.update(|cx| ai_store(cx).read(cx).conversations_of(&sid).is_empty())
    });
    cx.update(|cx| {
        assert!(
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .unwrap()
                .messages
                .is_empty(),
            "deleting the open chat clears it"
        )
    });
}

#[gpui_kit::test]
fn inline_panel_generates_a_command_and_puts_it_on_the_prompt_without_running_it(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (fx, _mock, tp, _panel, _sid) = ai_fixture(cx);
    win(cx, &fx, |w, cx| {
        w.click("ai-inline-trigger", cx);
        w.render_frame(cx);
    });
    let inline = cx
        .update(|cx| tp.read(cx).ai_inline().cloned())
        .expect("⌘I button opens the inline panel");
    win(cx, &fx, |w, cx| {
        inline.update(cx, |i, cx| {
            i.input()
                .update(cx, |s, cx| s.set_value("给我一个 ls 命令", w, cx))
        });
        w.render_frame(cx);
        w.click("ai-inline-send", cx);
    });
    wait_until(cx, "inline response", |cx| {
        inline.read_with(cx, |i, _| !i.streaming() && i.response() == "ls -la")
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("ai-inline-inject", cx);
        w.render_frame(cx);
    });
    cx.update(|cx| {
        assert!(
            tp.read(cx).ai_inline().is_none(),
            "injecting closes the panel"
        )
    });
    wait_until(cx, "command text on the prompt", |cx| {
        active_pane(cx).read_with(cx, |p, _| p.buffer_text().contains("ls -la"))
    });
    std::thread::sleep(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(
        !active_pane(cx).read_with(cx, |p, _| p.buffer_text().contains("drwx")),
        "no Enter was sent"
    );

    // The ⌘I shortcut and Escape.
    win(cx, &fx, |w, cx| {
        tp.update(cx, |p, cx| p.open_inline(w, cx));
        w.render_frame(cx);
    });
    cx.update(|cx| assert!(tp.read(cx).ai_inline().is_some()));
    win(cx, &fx, |w, cx| {
        w.press("escape", cx);
        w.render_frame(cx);
    });
    cx.update(|cx| assert!(tp.read(cx).ai_inline().is_none(), "Escape closes it"));
}

#[gpui_kit::test]
fn selection_analysis_starts_a_turn_with_the_error_prompt(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, mock, tp, _panel, sid) = ai_fixture(cx);
    let long = format!("make: *** [all] Error 2\n{}", "x".repeat(5000));
    win(cx, &fx, |w, cx| {
        tp.update(cx, |p, cx| p.analyze_selection(long.clone(), w, cx));
    });
    wait_idle(cx, &sid);
    let text = mock.all_text();
    assert!(text.contains("用户框选了终端中的一部分输出"), "{text}");
    assert!(
        text.contains("请分析以下终端输出中的错误原因") && text.contains("make: *** [all] Error 2")
    );
    assert!(text.contains("…(已截断)"));
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&sid).unwrap().clone();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[1].content, "你好，世界");
    });
    cx.update(|cx| assert_eq!(tp.read(cx).bottom_tab(), BottomTab::Ai));
}

#[gpui_kit::test]
fn batch_window_assess_button_shows_the_policy_risk(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let bx: Entity<BatchCommandsWindow> = win(cx, &fx, |w, cx| {
        cx.new(|cx| BatchCommandsWindow::new(None, w, cx))
    });
    win(cx, &fx, |w, cx| {
        bx.update(cx, |b, cx| b.set_command("rm -rf /", w, cx))
    });
    win(cx, &fx, |w, cx| bx.update(cx, |b, cx| b.assess(w, cx)));
    wait_until(cx, "assessment", |cx| {
        bx.read_with(cx, |b, _| b.risk().is_some())
    });
    bx.read_with(cx, |b, _| {
        assert_eq!(b.risk().unwrap().decision, AiActionDecision::Block)
    });
    // Editing the command clears the previous verdict.
    win(cx, &fx, |w, cx| {
        bx.update(cx, |b, cx| b.set_command("ls", w, cx))
    });
    cx.run_until_parked();
    bx.read_with(cx, |b, _| assert!(b.risk().is_none()));
}

// ---------------------------------------------------------------------------
// Safety settings in the chat
// ---------------------------------------------------------------------------

/// Stores AI preferences without disturbing the provider the fixture configured (the legacy row is adopted first).
fn set_prefs(fx: &Fixture, f: impl FnOnce(&mut ops_ai::prefs::AiPrefs)) -> ops_ai::prefs::AiPrefs {
    let mut prefs = ops_ai::prefs::load_prefs(&fx.ctx.db).unwrap();
    f(&mut prefs);
    ops_ai::prefs::save_prefs(&fx.ctx.db, &prefs).unwrap();
    prefs
}

fn executed(cx: &mut TestAppContext, sid: &str) -> bool {
    cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(sid)
            .is_some_and(|s| s.executed_actions().len() == 1)
    })
}

#[gpui_kit::test]
fn auto_mode_runs_commands_without_a_click(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, _mock, _tp, panel, sid) = ai_fixture(cx);
    let prefs = set_prefs(&fx, |p| {
        p.global_permission_mode = ops_ai::prefs::PermissionMode::Auto
    });
    cx.update(|cx| opsbatch_app::ai::runtime::set(cx, ops_ai::prefs::RuntimeLimits::from(prefs)));
    send_text(cx, &fx, &panel, "检查磁盘");
    wait_idle(cx, &sid);
    wait_until(cx, "command ran by itself", |cx| executed(cx, &sid));
    assert!(active_pane(cx).read_with(cx, |p, _| p.buffer_text().contains("ai-mark-42")));
    wait_until(cx, "follow-up answered", |cx| {
        cx.update(|cx| {
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .is_some_and(|s| !s.streaming() && s.messages.len() == 4)
        })
    });
}

#[gpui_kit::test]
fn confirm_mode_waits_for_the_click_and_a_matching_grant_skips_it(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, _mock, _tp, panel, sid) = ai_fixture(cx);
    send_text(cx, &fx, &panel, "检查磁盘");
    wait_idle(cx, &sid);
    wait_until(cx, "plan assessed", |cx| pending_with_assessment(cx, &sid));
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    assert!(!executed(cx, &sid), "nothing runs before the click");

    // a grant for exactly this command lets the next one through on its own
    let prefs = set_prefs(&fx, |p| {
        p.permission_grants
            .push(ops_ai::prefs::PermissionGrantRule {
                id: "g1".into(),
                capability_id: "terminal.execute".into(),
                session_pattern: "*".into(),
                command_pattern: Some("echo ai-mark-42".into()),
                ..Default::default()
            });
    });
    cx.update(|cx| opsbatch_app::ai::runtime::set(cx, ops_ai::prefs::RuntimeLimits::from(prefs)));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("ai-reject-0", cx);
    });
    send_text(cx, &fx, &panel, "检查磁盘");
    wait_until(cx, "granted command ran", |cx| {
        cx.update(|cx| {
            ai_store(cx)
                .read(cx)
                .session(&sid)
                .is_some_and(|s| s.executed_actions().len() == 1)
        })
    });
    let rule = cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(&sid)
            .unwrap()
            .executed_actions()[0]
            .assessment
            .clone()
            .unwrap()
            .matched_rule
    });
    assert_eq!(rule, "PERMISSION_GRANT");
}

#[gpui_kit::test]
fn observer_mode_and_the_blocklist_refuse_commands(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, _mock, _tp, panel, sid) = ai_fixture(cx);
    set_prefs(&fx, |p| {
        p.global_permission_mode = ops_ai::prefs::PermissionMode::Observer
    });
    send_text(cx, &fx, &panel, "检查磁盘");
    wait_idle(cx, &sid);
    wait_until(cx, "plan assessed", |cx| pending_with_assessment(cx, &sid));
    let a = cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(&sid)
            .unwrap()
            .pending_actions()[0]
            .clone()
    });
    assert_eq!(
        a.assessment
            .as_ref()
            .map(|x| (x.decision.clone(), x.matched_rule.as_str())),
        Some((AiActionDecision::Block, "AI_OBSERVER_MODE"))
    );
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("ai-approve-0", cx);
    });
    cx.run_until_parked();
    assert!(!executed(cx, &sid), "a blocked command never runs");

    // back to confirm, with the command on the blocklist
    set_prefs(&fx, |p| {
        p.global_permission_mode = ops_ai::prefs::PermissionMode::Confirm;
        p.command_blocklist = vec!["ai-mark".into()];
    });
    let db = fx.ctx.db.clone();
    let assessed = ai_assess_action(&db, "echo ai-mark-42".into()).unwrap();
    assert_eq!(
        (assessed.decision, assessed.matched_rule.as_str()),
        (AiActionDecision::Block, "AI_COMMAND_BLOCKLIST")
    );
}

#[gpui_kit::test]
fn follow_ups_stop_at_the_iteration_cap(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, _mock, _tp, _panel, sid) = ai_fixture(cx);
    let store = cx.update(|cx| ai_store(cx));
    let sent = cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.send_follow_up(&sid, "next".into(), None, 0, cx)
        })
    });
    assert!(!sent, "a cap of zero refuses the follow-up");
    cx.update(|cx| {
        let s = store.read(cx).session(&sid).unwrap();
        assert_eq!(s.messages.last().map(|m| m.role), Some(Role::System));
        assert!(!s.streaming());
    });
    let sent = cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.send_follow_up(&sid, "next".into(), None, 3, cx)
        })
    });
    assert!(sent);
    cx.update(|cx| assert_eq!(store.read(cx).session(&sid).unwrap().follow_ups, 1));
}

// ---------------------------------------------------------------------------
// Slash commands of the composer
// ---------------------------------------------------------------------------

/// Types into the composer the way the keyboard does (`set_value` alone emits nothing).
fn type_text(cx: &mut TestAppContext, fx: &Fixture, panel: &Entity<AiChatPanel>, text: &str) {
    win(cx, fx, |w, cx| {
        let input = panel.read(cx).input().clone();
        input.update(cx, |i, cx| i.set_value(text.to_string(), w, cx));
        input.update(cx, |_, cx| {
            cx.emit(gpui_kit::component::input::InputEvent::Change)
        });
    });
    cx.run_until_parked();
}

fn text_of(cx: &mut TestAppContext, panel: &Entity<AiChatPanel>) -> String {
    cx.update(|cx| panel.read(cx).input().read(cx).value().to_string())
}

#[gpui_kit::test]
fn the_slash_list_offers_quick_messages_and_fills_the_input(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, mock, _tp, panel, sid) = ai_fixture(cx);
    let prefs = set_prefs(&fx, |p| {
        p.quick_messages =
            ops_ai::prefs::quick::sanitize_quick_messages(vec![ops_ai::prefs::QuickMessage {
                name: "磁盘检查".into(),
                slug: "disk".into(),
                content: "检查磁盘".into(),
                description: Some("看看空间".into()),
                ..Default::default()
            }]);
    });
    cx.update(|cx| opsbatch_app::ai::runtime::set(cx, ops_ai::prefs::RuntimeLimits::from(prefs)));

    // typing "/" opens the list; picking an entry puts its prompt in the input
    type_text(cx, &fx, &panel, "/di");
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("ai-slash-list").is_some() && w.try_find("ai-slash-item-0").is_some());
        w.click("ai-slash-item-0", cx);
    });
    assert_eq!(text_of(cx, &panel), "检查磁盘");
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("ai-slash-list").is_none(),
            "the list closes once the command is replaced"
        );
    });

    // Enter on a bare "/slug" does the same
    win(cx, &fx, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input()
                .update(cx, |i, cx| i.set_value("/disk".to_string(), w, cx))
        })
    });
    win(cx, &fx, |w, cx| panel.update(cx, |p, cx| p.send(w, cx)));
    assert_eq!(text_of(cx, &panel), "检查磁盘");
    // and the picked prompt is a normal message
    win(cx, &fx, |w, cx| panel.update(cx, |p, cx| p.send(w, cx)));
    wait_idle(cx, &sid);
    assert!(mock.all_text().contains("检查磁盘"));
}

#[gpui_kit::test]
fn a_user_skill_joins_the_request_as_context(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, mock, _tp, panel, sid) = ai_fixture(cx);
    let dir = fx.ctx.core.data_dir.join("skills/Deploy Helper");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "---\nname: Deploy Helper\ndescription: ships\n---\nSKILL-BODY-MARK always run smoke tests first").unwrap();

    type_text(cx, &fx, &panel, "/");
    let items = cx.update(|cx| panel.read(cx).slash_items("dep", cx));
    assert_eq!(
        items.iter().map(|i| i.slug.as_str()).collect::<Vec<_>>(),
        vec!["deploy-helper"]
    );
    win(cx, &fx, |w, cx| {
        panel.update(cx, |p, cx| p.apply_slash(items[0].clone(), w, cx))
    });
    assert_eq!(text_of(cx, &panel), "/deploy-helper ");

    win(cx, &fx, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input().update(cx, |i, cx| {
                i.set_value("/deploy-helper 检查磁盘".to_string(), w, cx)
            })
        })
    });
    win(cx, &fx, |w, cx| panel.update(cx, |p, cx| p.send(w, cx)));
    wait_idle(cx, &sid);
    let text = mock.all_text();
    assert!(
        text.contains("SKILL-BODY-MARK") && text.contains("检查磁盘"),
        "{text}"
    );
    cx.update(|cx| {
        assert_eq!(
            ai_store(cx).read(cx).session(&sid).unwrap().messages[0].content,
            "检查磁盘",
            "the chat shows the question, not the skill"
        )
    });
}

#[gpui_kit::test]
fn search_runs_the_query_and_sends_the_results_as_context(cx: &mut TestAppContext) {
    use std::io::{Read, Write};
    let _s = serial();
    let (fx, mock, _tp, panel, sid) = ai_fixture(cx);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            *log.lock().unwrap() = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            let body = r#"{"results":[{"title":"Rust 官网","url":"https://rust-lang.org","content":"SEARCH-RESULT-MARK"}]}"#;
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
        }
    });
    let prefs = set_prefs(&fx, |p| {
        p.web_search = Some(ops_ai::prefs::WebSearchConfig {
            provider_id: "searxng".into(),
            api_host: Some(host),
            enabled: true,
            ..Default::default()
        })
    });
    cx.update(|cx| opsbatch_app::ai::runtime::set(cx, ops_ai::prefs::RuntimeLimits::from(prefs)));

    // "/" lists the search command; with a query Enter runs it
    let items = cx.update(|cx| panel.read(cx).slash_items("se", cx));
    assert_eq!(
        items.iter().map(|i| i.slug.as_str()).collect::<Vec<_>>(),
        vec!["search"]
    );
    win(cx, &fx, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input().update(cx, |i, cx| {
                i.set_value("/search rust gpui".to_string(), w, cx)
            })
        })
    });
    win(cx, &fx, |w, cx| panel.update(cx, |p, cx| p.send(w, cx)));
    wait_until(cx, "search results reached the model", |_| {
        mock.all_text().contains("SEARCH-RESULT-MARK")
    });
    wait_idle(cx, &sid);
    assert!(
        seen.lock()
            .unwrap()
            .contains("GET /search?q=rust%20gpui&format=json"),
        "{}",
        seen.lock().unwrap()
    );
    cx.update(|cx| {
        assert_eq!(
            ai_store(cx).read(cx).session(&sid).unwrap().messages[0].content,
            "rust gpui"
        )
    });
    assert_eq!(text_of(cx, &panel), "");
}
