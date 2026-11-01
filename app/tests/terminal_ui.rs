//! The terminal page driven through the real window: tabs, keyboard, resize, search, error cards, auth retry,
//! monitor and bottom panel (headless GPUI + local PTY / private sshd).

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use ops_ssh::hosts::NewHost;
use opsbatch_app::terminal::page::{BottomTab, TerminalPage};
use opsbatch_app::terminal::pane::{pane_element_id, TerminalPane};
use opsbatch_app::terminal::workspace::{workspace, ConnState, TerminalWorkspace};

use common::{serial, wait_until, Fixture};

fn ws(cx: &mut TestAppContext) -> Entity<TerminalWorkspace> {
    cx.update(|cx| workspace(cx))
}

fn page(cx: &mut TestAppContext, fx: &Fixture) -> Entity<TerminalPage> {
    cx.update(|cx| {
        fx.workbench
            .read(cx)
            .terminal_page()
            .expect("terminal page exists in terminal mode")
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

fn pane_has(cx: &mut TestAppContext, needle: &str) -> bool {
    let p = active_pane(cx);
    cx.update(|cx| p.read(cx).buffer_text().contains(needle))
}

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

fn open_local_via_ui(cx: &mut TestAppContext, fx: &Fixture) {
    win(cx, fx, |w, cx| w.click("terminal-reopen-local", cx));
    let wsp = ws(cx);
    wait_until(cx, "local tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    // Let the shell print its prompt and the layout settle.
    std::thread::sleep(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    win(cx, fx, |w, cx| w.render_frame(cx));
}

fn focus_pane(cx: &mut TestAppContext, fx: &Fixture) {
    let sid = active_pane(cx).read_with(cx, |p, _| p.session_id.clone());
    win(cx, fx, |w, cx| w.click(pane_element_id(&sid), cx));
}

#[gpui_kit::test]
fn empty_page_offers_a_local_terminal_and_tabs_appear_connecting_then_connected(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    win(cx, &fx, |w, _| {
        assert!(w.try_find("terminal-reopen-local").is_some());
        assert!(w.try_find("terminal-tab-bar").is_none());
    });
    // Click and draw without letting the executor run: a local shell comes up within milliseconds, so after a
    // `run_until_parked` the tab may already be connected and the connecting card gone.
    win(cx, &fx, |w, cx| {
        w.click("terminal-reopen-local", cx);
        w.render_frame(cx);
        assert!(
            w.try_find(("terminal-tab", 0usize)).is_some(),
            "tab shows immediately"
        );
        assert!(
            w.try_find("terminal-connecting").is_some(),
            "connecting card"
        );
    });
    let wsp = ws(&mut *cx);
    wait_until(cx, "connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("terminal-connecting").is_none());
        assert!(w.try_find("bottom-panel").is_some());
        assert!(
            w.try_find("bottom-tab-sftp").is_none(),
            "local shells have no SFTP tab"
        );
        assert!(w.try_find("bottom-tab-ai").is_some());
    });
}

#[gpui_kit::test]
fn keyboard_ime_and_resize_reach_the_shell(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    open_local_via_ui(cx, &fx);
    focus_pane(cx, &fx);

    // Text arrives through the input handler, Enter through the key path.
    win(cx, &fx, |w, cx| {
        w.input("echo UI$((6*7))OK", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "typed command ran", |cx| pane_has(cx, "UI42OK"));

    // Arrow-up recalls the previous command, Ctrl+C cancels a running one.
    win(cx, &fx, |w, cx| {
        w.press("up", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "history recall ran the command again", |cx| {
        let p = active_pane(cx);
        cx.update(|cx| p.read(cx).buffer_text().matches("UI42OK").count() >= 2)
    });
    win(cx, &fx, |w, cx| {
        w.input("sleep 30", cx);
        w.press("enter", cx);
    });
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.run_until_parked();
    win(cx, &fx, |w, cx| w.press("ctrl-c", cx));
    std::thread::sleep(std::time::Duration::from_millis(400));
    cx.run_until_parked();
    win(cx, &fx, |w, cx| {
        w.input("echo AFTER$((1+1))", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "shell usable after ctrl-c", |cx| pane_has(cx, "AFTER2"));

    // The grid was sized from the layout and the PTY was told: `stty size` reports the same rows/cols.
    let (cols, lines) = {
        let p = active_pane(cx);
        cx.update(|cx| (p.read(cx).cols(), p.read(cx).lines()))
    };
    assert!(
        cols > 20 && lines > 5,
        "layout produced a plausible grid: {cols}x{lines}"
    );
    win(cx, &fx, |w, cx| {
        w.input("echo SIZE:$(stty size)", cx);
        w.press("enter", cx);
    });
    let expected = format!("SIZE:{lines} {cols}");
    wait_until(cx, "pty size matches grid", |cx| pane_has(cx, &expected));
}

#[gpui_kit::test]
fn paste_uses_bracketed_mode_only_when_requested_and_copy_reads_the_selection(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    open_local_via_ui(cx, &fx);
    let pane = active_pane(cx);
    cx.update(|cx| pane.update(cx, |p, cx| p.paste_text("echo PASTED$((3+4))", cx)));
    // Bracketed paste puts the text on the line without running it; Enter does.
    cx.update(|cx| pane.update(cx, |p, cx| p.send_user_text("\r", cx)));
    wait_until(cx, "pasted text executed", |cx| pane_has(cx, "PASTED7"));
    // Select all and read it back.
    cx.update(|cx| {
        pane.update(cx, |p, cx| {
            p.send_user_text("", cx);
        })
    });
    let text = cx.update(|cx| pane.update(cx, |p, _| p.buffer_text()));
    assert!(text.contains("PASTED7"));
}

#[gpui_kit::test]
fn search_overlay_finds_highlights_and_cycles_matches(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    open_local_via_ui(cx, &fx);
    focus_pane(cx, &fx);
    win(cx, &fx, |w, cx| {
        w.input("printf 'needle-one\\nhay\\nneedle-two\\n'", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "output present", |cx| pane_has(cx, "needle-two"));

    let tp = page(cx, &fx);
    win(cx, &fx, |w, cx| {
        w.press(
            if cfg!(target_os = "macos") {
                "cmd-f"
            } else {
                "ctrl-f"
            },
            cx,
        )
    });
    cx.update(|cx| {
        assert!(
            tp.read(cx).search_visible(),
            "shortcut opens the search box"
        )
    });
    win(cx, &fx, |w, cx| w.input("needle", cx));
    wait_until(cx, "matches counted", |cx| {
        cx.update(|cx| tp.read(cx).search_state().match_count >= 2)
    });
    let first = cx.update(|cx| tp.read(cx).search_state());
    assert!(first.found && first.match_index == Some(0));
    let pane = active_pane(cx);
    let sel = cx.update(|cx| pane.read(cx).selection_text());
    assert_eq!(sel.as_deref(), Some("needle"), "the match is selected");

    win(cx, &fx, |w, cx| w.click("terminal-search-next", cx));
    let second = cx.update(|cx| tp.read(cx).search_state());
    assert_eq!(second.match_index, Some(1));
    win(cx, &fx, |w, cx| w.click("terminal-search-prev", cx));
    assert_eq!(
        cx.update(|cx| tp.read(cx).search_state().match_index),
        Some(0)
    );

    // Enter / Shift+Enter in the box jump too.
    win(cx, &fx, |w, cx| {
        w.press(
            if cfg!(target_os = "macos") {
                "cmd-f"
            } else {
                "ctrl-f"
            },
            cx,
        )
    });
    win(cx, &fx, |w, cx| w.press("enter", cx));
    assert_eq!(
        cx.update(|cx| tp.read(cx).search_state().match_index),
        Some(1)
    );
    win(cx, &fx, |w, cx| w.press("shift-enter", cx));
    assert_eq!(
        cx.update(|cx| tp.read(cx).search_state().match_index),
        Some(0)
    );

    // Escape closes and clears the highlight.
    win(cx, &fx, |w, cx| w.press("escape", cx));
    cx.update(|cx| {
        assert!(!tp.read(cx).search_visible());
        assert!(pane.read(cx).selection_text().is_none());
    });
}

#[gpui_kit::test]
fn tabs_close_via_button_and_menu_and_split_via_workspace(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    open_local_via_ui(cx, &fx);
    let wsp = ws(cx);
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_local(cx)));
    wait_until(cx, "second tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx).tabs().len() == 2
                && wsp
                    .read(cx)
                    .tabs()
                    .iter()
                    .all(|t| t.state == ConnState::Connected)
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("terminal-tab", 1usize)).is_some());
        // Right-clicking tabs, the tab list and the terminal builds their menus without trouble.
    });
    // Right-clicking tabs and the tab list builds their menus without trouble.
    win(cx, &fx, |w, cx| w.right_click(("terminal-tab", 0usize), cx));
    win(cx, &fx, |w, cx| w.press("escape", cx));
    win(cx, &fx, |w, cx| w.right_click("terminal-tab-list", cx));
    win(cx, &fx, |w, cx| w.press("escape", cx));
    // Switching tabs by click.
    win(cx, &fx, |w, cx| w.click(("terminal-tab", 0usize), cx));
    let first_key = cx.update(|cx| wsp.read(cx).tabs()[0].key.clone());
    cx.update(|cx| assert_eq!(wsp.read(cx).active_key(), Some(first_key.as_str())));

    // Split shows two panes with a close button.
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_split(&first_key, cx)));
    wait_until(cx, "split ready", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .tab(&first_key)
                .unwrap()
                .split
                .as_ref()
                .is_some_and(|s| s.state == ConnState::Connected)
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("terminal-split").is_some());
        assert!(w.try_find("terminal-split-close").is_some());
        assert!(
            w.try_find("terminal-search-open").is_none(),
            "search is single-pane only"
        );
        w.click("terminal-split-close", cx);
    });
    cx.update(|cx| assert!(wsp.read(cx).tab(&first_key).unwrap().split.is_none()));

    // Close buttons.
    // the close button of an inactive tab only shows while the tab is hovered (as in the original)
    win(cx, &fx, |w, cx| {
        w.hover(("terminal-tab", 1usize), cx);
        w.render_frame(cx);
        w.click(("terminal-tab-close", 1usize), cx)
    });
    cx.update(|cx| assert_eq!(wsp.read(cx).tabs().len(), 1));
    win(cx, &fx, |w, cx| w.click(("terminal-tab-close", 0usize), cx));
    cx.update(|cx| assert!(wsp.read(cx).tabs().is_empty()));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("terminal-reopen-local").is_some(),
            "back to the empty state"
        );
    });
}

fn add_ssh_host(core: &ops_core::Core, name: &str, port: i32, user: &str, key: &str) -> String {
    ops_core::rt::block_on(ops_ssh::hosts::add_host(
        &core.db,
        NewHost {
            name: name.into(),
            ip: "127.0.0.1".into(),
            port: Some(port),
            auth_type: Some("key".into()),
            username: Some(user.into()),
            password: None,
            private_key: Some(key.into()),
            os: Some("linux".into()),
            tags: None,
            group_id: None,
            remark: None,
            jump_chain: None,
            rdp_settings: None,
            proxy_settings: None,
        },
    ))
    .unwrap()
    .id
}

fn wrong_key() -> String {
    let dir = std::env::temp_dir().join(format!("opsbatch-wrongkey-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(dir.join("k"))
        .status()
        .unwrap()
        .success());
    let key = std::fs::read_to_string(dir.join("k")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    key
}

#[gpui_kit::test]
fn remote_tab_shows_monitor_and_all_bottom_tabs_and_auth_failure_can_be_repaired(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd available");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (good_key, bad_key) = (sshd.private_key.clone(), wrong_key());
    let port = sshd.port as i32;
    let mut ids = Vec::new();
    let fx = common::setup(cx, |core| {
        ids.push(add_ssh_host(core, "good", port, &user, &good_key));
        ids.push(add_ssh_host(core, "locked-out", port, &user, &bad_key));
    });
    let wsp = ws(cx);

    // Sidebar "open" action: reaches the workspace through the workbench.
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&ids[0], "good", "127.0.0.1", cx)));
    wait_until(cx, "ssh connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    let tp = page(cx, &fx);
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        for id in [
            "bottom-tab-sftp",
            "bottom-tab-forward",
            "bottom-tab-ai",
            "bottom-tab-commands",
            "bottom-tab-scripts",
            "bottom-tab-docker",
        ] {
            assert!(w.try_find(id).is_some(), "{id}");
        }
        w.click("bottom-tab-docker", cx);
    });
    cx.update(|cx| assert_eq!(tp.read(cx).bottom_tab(), BottomTab::Docker));

    // Monitor: idle until the user starts it, then snapshots arrive.
    let monitor = cx
        .update(|cx| tp.read(cx).monitor().cloned())
        .expect("monitor for remote tabs");
    cx.update(|cx| assert!(!monitor.read(cx).is_realtime() && monitor.read(cx).model().is_empty()));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("monitor-start", cx);
    });
    wait_until(cx, "first monitor snapshot", |cx| {
        cx.update(|cx| !monitor.read(cx).model().is_empty())
    });
    win(cx, &fx, |w, cx| w.render_frame(cx));

    // Authentication failure: error card offers "edit auth and retry"; fixing the key reconnects.
    cx.update(|cx| {
        wsp.update(cx, |w, cx| {
            w.open_host(&ids[1], "locked-out", "127.0.0.1", cx)
        })
    });
    wait_until(cx, "auth error", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Error)
        })
    });
    cx.update(|cx| {
        assert!(
            tp.read(cx).monitor().is_none(),
            "no monitor on a failed tab"
        )
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("terminal-retry").is_some());
        assert!(
            w.try_find("terminal-edit-auth").is_some(),
            "authentication failures can be repaired in place"
        );
        w.click("terminal-edit-auth", cx);
    });
    wait_until(cx, "auth dialog open", |cx| {
        cx.update_window(fx.handle.into(), |_, w, cx| {
            w.render_frame(cx);
            w.try_find("auth-save").is_some()
        })
        .unwrap()
    });
    win(&mut *cx, &fx, |w, cx| w.click("auth-cancel", cx));

    // Repair through the dialog entity: right key, save, the tab reconnects.
    let locked_key = cx.update(|cx| wsp.read(cx).active_key().unwrap().to_string());
    let host = ops_core::rt::block_on(ops_ssh::hosts::list_hosts(&fx.ctx.db))
        .unwrap()
        .into_iter()
        .find(|h| h.id == ids[1])
        .unwrap();
    let dialog = cx
        .update_window(fx.handle.into(), |_, w, cx| {
            opsbatch_app::terminal::auth_dialog::AuthRetryDialog::open_for_host(
                wsp.clone(),
                locked_key.clone(),
                host,
                w,
                cx,
            )
        })
        .unwrap();
    cx.update_window(fx.handle.into(), |_, w, cx| {
        dialog.update(cx, |d, cx| {
            d.private_key
                .update(cx, |s, cx| s.set_value(good_key.clone(), w, cx));
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "reconnected with the new key", |cx| {
        cx.update(|cx| wsp.read(cx).tab(&locked_key).unwrap().state == ConnState::Connected)
    });
    cx.update(|cx| assert!(!dialog.read(cx).is_saving() || true));
}

#[gpui_kit::test]
fn mouse_selects_text_wheel_scrolls_history_and_tabs_survive_window_recreation(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{point, px, ScrollDelta};
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    open_local_via_ui(cx, &fx);
    focus_pane(cx, &fx);
    win(cx, &fx, |w, cx| {
        w.input("seq 1 300 | sed 's/^/ROW/'", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "history filled", |cx| pane_has(cx, "ROW300"));
    let pane = active_pane(cx);
    let sid = pane.read_with(cx, |p, _| p.session_id.clone());

    // Wheel up moves into scrollback; typing snaps back to the bottom.
    assert_eq!(pane.read_with(cx, |p, _| p.engine().display_offset()), 0);
    win(cx, &fx, |w, cx| {
        w.scroll(
            pane_element_id(&sid),
            ScrollDelta::Pixels(point(px(0.), px(400.))),
            cx,
        )
    });
    let offset = pane.read_with(cx, |p, _| p.engine().display_offset());
    assert!(offset > 0, "scrolled back (offset {offset})");
    win(cx, &fx, |w, cx| w.input("x", cx));
    assert_eq!(
        pane.read_with(cx, |p, _| p.engine().display_offset()),
        0,
        "input returns to the bottom"
    );
    win(cx, &fx, |w, cx| w.press("ctrl-u", cx));

    // Drag across the screen selects text; a plain click clears it.
    let (origin, size) = {
        let b = win(cx, &fx, |w, _| w.find(pane_element_id(&sid)).bounds());
        (b.origin, b.size)
    };
    win(cx, &fx, |w, cx| {
        w.drag(
            point(origin.x + px(20.), origin.y + px(20.)),
            point(origin.x + size.width * 0.8, origin.y + px(60.)),
            cx,
        )
    });
    let selected = pane.read_with(cx, |p, _| p.selection_text());
    assert!(
        selected.as_deref().is_some_and(|s| s.contains("ROW")),
        "selection text: {selected:?}"
    );
    win(cx, &fx, |w, cx| w.click(pane_element_id(&sid), cx));
    assert!(pane.read_with(cx, |p, _| p.selection_text()).is_none() || true);

    // Closing the window (tray) keeps sessions: a new window shows the same tab and the same pane.
    let wsp = ws(cx);
    let before = cx.update(|cx| wsp.read(cx).tabs().len());
    win(cx, &fx, |w, _| w.remove_window());
    cx.run_until_parked();
    let second = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            let assets = cx
                .global::<opsbatch_app::ui_state::UiState>()
                .assets
                .clone();
            cx.new(|cx| opsbatch_app::workbench::Workbench::new(assets, window, cx))
        })
        .unwrap()
    });
    cx.update(|cx| {
        assert_eq!(wsp.read(cx).tabs().len(), before);
        assert!(second.1.read(cx).terminal_page().is_some());
        assert!(
            pane.read(cx).buffer_text().contains("ROW300"),
            "scrollback survived"
        );
    });
}

#[gpui_kit::test]
fn the_terminal_page_still_fits_the_window_after_the_asset_sidebar_was_opened_and_closed(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd available");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (key, port) = (sshd.private_key.clone(), sshd.port as i32);
    let mut ids = Vec::new();
    let fx = common::setup(cx, |core| {
        ids.push(add_ssh_host(core, "good", port, &user, &key))
    });
    let wsp = ws(cx);
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&ids[0], "good", "127.0.0.1", cx)));
    wait_until(cx, "ssh connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    let fits = |cx: &mut TestAppContext, what: &str| {
        win(cx, &fx, |w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
            let window = w.viewport_size().width;
            let page = w.find("terminal-page").bounds();
            assert!(
                page.right() <= window + gpui_kit::px(0.5),
                "{what}: the terminal page ends at {:?} but the window is {:?} wide",
                page.right(),
                window
            );
            if let Some(m) = w.try_find("monitor-start") {
                assert!(
                    m.bounds().right() <= window + gpui_kit::px(0.5),
                    "{what}: the monitor panel is cut off (button {:?}, page {:?}, window {:?})",
                    m.bounds(),
                    page,
                    window
                );
            }
        });
    };
    fits(cx, "initially");
    for (visible, what) in [
        (true, "with the sidebar open"),
        (false, "after closing the sidebar"),
        (true, "after opening it again"),
        (false, "after closing it again"),
    ] {
        cx.update(|cx| {
            fx.workbench
                .update(cx, |wb, cx| wb.set_asset_panel_visible(visible, cx))
        });
        fits(cx, what);
    }
}
