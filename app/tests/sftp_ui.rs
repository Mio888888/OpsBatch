//! SFTP panel inside the terminal page, driven through the window (private sshd).

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use ops_ssh::hosts::NewHost;
use opsbatch_app::sftp::store::{sftp_store, SessionState, SftpStore, Side, TransferStatus};
use opsbatch_app::terminal::workspace::{workspace, ConnState};

use common::{serial, wait_until, Fixture};

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

fn store(cx: &mut TestAppContext) -> Entity<SftpStore> {
    cx.update(|cx| sftp_store(cx))
}

#[gpui_kit::test]
fn sftp_tab_lists_files_navigates_transfers_and_edits_through_the_ui(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "sftp-ui".into(),
                ip: "127.0.0.1".into(),
                port: Some(port),
                auth_type: Some("key".into()),
                username: Some(user.clone()),
                password: None,
                private_key: Some(key.clone()),
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
        .id;
    });
    let work = std::env::temp_dir().join(format!("opsbatch-sftpui-{}", uuid::Uuid::new_v4()));
    let (remote_dir, local_dir) = (work.join("remote"), work.join("local"));
    std::fs::create_dir_all(remote_dir.join("folder")).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::write(remote_dir.join("a.txt"), "alpha").unwrap();
    std::fs::write(remote_dir.join("b.log"), "beta").unwrap();
    std::fs::write(local_dir.join("up.txt"), "local file").unwrap();

    // Open the SSH tab; the SFTP bottom tab is selected by default and connects.
    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| {
        wsp.update(cx, |w, cx| {
            w.open_host(&host_id, "sftp-ui", "127.0.0.1", cx)
        })
    });
    wait_until(cx, "ssh tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    let st = store(cx);
    wait_until(cx, "sftp ready", |cx| {
        cx.update(|cx| st.read(cx).session == SessionState::Ready && !st.read(cx).remote_loading)
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("sftp-panel").is_some());
        assert!(
            w.try_find("sftp-list-remote").is_some() && w.try_find("sftp-list-local").is_some()
        );
        assert!(
            w.try_find("sftp-choose-local-empty").is_some(),
            "local side starts empty until a folder is chosen"
        );
    });

    // Remote path box: Enter navigates.
    cx.update(|cx| {
        st.update(cx, |s, cx| {
            s.navigate(Side::Remote, &remote_dir.to_string_lossy(), cx)
        })
    });
    wait_until(cx, "remote listed", |cx| {
        cx.update(|cx| st.read(cx).remote_entries.len() == 3)
    });
    cx.update(|cx| st.update(cx, |s, cx| s.authorize_local(&local_dir, cx)))
        .unwrap();
    wait_until(cx, "local listed", |cx| {
        cx.update(|cx| st.read(cx).local_entries.len() == 1)
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        // directories sort first, then files
        assert!(w.try_find(("sftp-row-remote", 0usize)).is_some());
        assert!(w.try_find(("sftp-row-remote", 2usize)).is_some());
        assert!(w.try_find(("sftp-row-local", 0usize)).is_some());
    });
    cx.update(|cx| assert_eq!(st.read(cx).remote_entries[0].name, "folder"));

    // Click selects, double-click on a directory enters it, "up" returns.
    win(cx, &fx, |w, cx| w.click(("sftp-row-remote", 1usize), cx));
    cx.update(|cx| {
        assert_eq!(
            st.read(cx)
                .selected(Side::Remote)
                .map(|e| e.name.clone())
                .as_deref(),
            Some("a.txt")
        )
    });
    win(cx, &fx, |w, cx| {
        w.double_click(("sftp-row-remote", 0usize), cx)
    });
    wait_until(cx, "entered folder", |cx| {
        cx.update(|cx| st.read(cx).remote_path.ends_with("/folder"))
    });
    win(cx, &fx, |w, cx| w.click("sftp-up-remote", cx));
    wait_until(cx, "back in parent", |cx| {
        cx.update(|cx| st.read(cx).remote_path == remote_dir.to_string_lossy())
    });

    // Keyboard: focus the list, arrow down selects, enter on a file asks for the editor.
    win(cx, &fx, |w, cx| w.click("sftp-list-remote", cx));
    win(cx, &fx, |w, cx| w.press("down", cx));
    cx.update(|cx| assert!(st.read(cx).selected(Side::Remote).is_some()));

    // Drag a local file onto the remote list uploads it.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.drag_to(("sftp-row-local", 0usize), "sftp-list-remote", cx);
    });
    wait_until(cx, "dragged upload finished", |cx| {
        cx.update(|cx| {
            st.read(cx)
                .transfers
                .iter()
                .any(|t| t.status == TransferStatus::Done)
        })
    });
    assert_eq!(
        std::fs::read_to_string(remote_dir.join("up.txt")).unwrap(),
        "local file"
    );
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("sftp-transfers").is_some(), "queue visible");
        assert!(w.try_find("sftp-toggle-transfers").is_some());
    });

    // Preview of a text file shows the overlay and closes again.
    let entry = cx.update(|cx| {
        st.read(cx)
            .remote_entries
            .iter()
            .find(|e| e.name == "a.txt")
            .cloned()
            .unwrap()
    });
    cx.update(|cx| st.update(cx, |s, cx| s.open_preview(Side::Remote, entry, cx)));
    wait_until(cx, "preview data", |cx| {
        cx.update(|cx| {
            st.read(cx)
                .preview
                .as_ref()
                .is_some_and(|p| p.data.is_some())
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("sftp-preview").is_some());
        w.click("sftp-preview-close", cx);
    });
    cx.update(|cx| assert!(st.read(cx).preview.is_none()));

    // Context menus on rows build for both sides.
    win(cx, &fx, |w, cx| {
        w.right_click(("sftp-row-remote", 1usize), cx)
    });
    win(cx, &fx, |w, cx| w.press("escape", cx));
    win(cx, &fx, |w, cx| {
        w.right_click(("sftp-row-local", 0usize), cx)
    });
    win(cx, &fx, |w, cx| w.press("escape", cx));

    let _ = std::fs::remove_dir_all(&work);
}

#[gpui_kit::test]
fn port_forward_panel_adds_a_working_rule_then_stops_and_removes_it(cx: &mut TestAppContext) {
    use opsbatch_app::terminal::page::BottomTab;
    use std::io::Read;
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "pf".into(),
                ip: "127.0.0.1".into(),
                port: Some(port),
                auth_type: Some("key".into()),
                username: Some(user.clone()),
                password: None,
                private_key: Some(key.clone()),
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
        .id;
    });
    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&host_id, "pf", "127.0.0.1", cx)));
    wait_until(cx, "connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("bottom-tab-forward", cx);
    });
    let page = cx.update(|cx| fx.workbench.read(cx).terminal_page().unwrap().clone());
    assert_eq!(
        cx.update(|cx| page.read(cx).bottom_tab()),
        BottomTab::Forward
    );
    let panel = cx
        .update(|cx| page.read(cx).forward_panel().cloned())
        .expect("forward panel");
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("pf-panel").is_some());
        w.click("pf-toggle-form", cx);
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("pf-form").is_some());
        assert_eq!(
            w.find("pf-submit").disabled(),
            Some(true),
            "needs addresses first"
        );
    });

    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (local, remote, note) = cx.update(|cx| panel.read(cx).form_inputs());
    cx.update_window(fx.handle.into(), |_, w, cx| {
        local.update(cx, |s, cx| s.set_value(format!("127.0.0.1:{free}"), w, cx));
        remote.update(cx, |s, cx| {
            s.set_value(format!("127.0.0.1:{}", sshd.port), w, cx)
        });
        note.update(cx, |s, cx| s.set_value("ssh via tunnel", w, cx));
    })
    .unwrap();
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert_ne!(w.find("pf-submit").disabled(), Some(true));
        w.click("pf-submit", cx);
    });
    wait_until(cx, "rule listed", |cx| {
        cx.update(|cx| panel.read(cx).forwards().len() == 1)
    });
    cx.update(|cx| {
        let f = &panel.read(cx).forwards()[0];
        assert_eq!(f.config.label.as_deref(), Some("ssh via tunnel"));
        assert_eq!(f.config.local_addr, format!("127.0.0.1:{free}"));
    });

    // The tunnel really forwards: connecting to the local port yields the SSH banner.
    let mut banner = String::new();
    for _ in 0..50 {
        if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", free)) {
            s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .ok();
            let mut buf = [0u8; 16];
            if let Ok(n) = s.read(&mut buf) {
                banner = String::from_utf8_lossy(&buf[..n]).to_string();
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        banner.starts_with("SSH-"),
        "banner through the tunnel: {banner:?}"
    );

    // Stop button, then delete.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click(("pf-stop", 0usize), cx);
    });
    wait_until(cx, "status left active", |cx| {
        cx.update(|cx| {
            !matches!(
                panel.read(cx).forwards()[0].status,
                ops_ssh::forward::ForwardStatus::Active
            )
        })
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click(("pf-remove", 0usize), cx);
    });
    wait_until(cx, "rule removed", |cx| {
        cx.update(|cx| panel.read(cx).forwards().is_empty())
    });

    // Reopening the panel (new window state) lists real rules, not blank placeholders.
    win(cx, &fx, |w, cx| w.click("pf-toggle-form", cx));
}

#[gpui_kit::test]
fn sftp_dialogs_create_rename_delete_and_double_click_asks_for_the_editor(cx: &mut TestAppContext) {
    use opsbatch_app::sftp::panel::SftpPanelEvent;
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "dlg".into(),
                ip: "127.0.0.1".into(),
                port: Some(port),
                auth_type: Some("key".into()),
                username: Some(user.clone()),
                password: None,
                private_key: Some(key.clone()),
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
        .id;
    });
    let work = std::env::temp_dir().join(format!("opsbatch-sftpdlg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("old.txt"), "x").unwrap();

    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| wsp.update(cx, |w, cx| w.open_host(&host_id, "dlg", "127.0.0.1", cx)));
    wait_until(cx, "connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    let st = store(cx);
    wait_until(cx, "sftp ready", |cx| {
        cx.update(|cx| st.read(cx).session == SessionState::Ready)
    });
    cx.update(|cx| {
        st.update(cx, |s, cx| {
            s.navigate(Side::Remote, &work.to_string_lossy(), cx)
        })
    });
    wait_until(cx, "listed", |cx| {
        cx.update(|cx| {
            st.read(cx)
                .remote_entries
                .iter()
                .any(|e| e.name == "old.txt")
        })
    });
    let page = cx.update(|cx| fx.workbench.read(cx).terminal_page().unwrap().clone());
    let panel = cx
        .update(|cx| page.read(cx).sftp_panel().cloned())
        .expect("sftp panel");

    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&panel, move |_, ev: &SftpPanelEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
    });

    // Double-click a remote file: the panel asks for the editor window.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.double_click(("sftp-row-remote", 0usize), cx);
    });
    assert!(
        matches!(events.borrow().first(), Some(SftpPanelEvent::OpenEditor { dir: false, path, .. }) if path.ends_with("old.txt")),
        "{:?}",
        events.borrow()
    );

    // New file dialog.
    let prompt = cx
        .update_window(fx.handle.into(), |_, w, cx| {
            panel.update(cx, |p, cx| p.prompt_create(Side::Remote, false, w, cx))
        })
        .unwrap();
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("text-prompt").is_some());
        assert_eq!(w.find("prompt-ok").disabled(), Some(true), "empty name");
        w.input("created.txt", cx);
        w.render_frame(cx);
        assert_eq!(
            prompt.read(cx).input().read(cx).value().as_ref(),
            "created.txt",
            "typing reaches the dialog field"
        );
    });
    // Let the field's change reach the dialog and redraw it before pressing OK.
    cx.run_until_parked();
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert_ne!(w.find("prompt-ok").disabled(), Some(true), "name entered");
        w.click("prompt-ok", cx);
    });
    // On failure say what the panel and the disk looked like: this only ever failed on one CI image.
    let listed = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            st.read(cx)
                .remote_entries
                .iter()
                .any(|e| e.name == "created.txt")
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if listed(cx) {
            break;
        }
        if std::time::Instant::now() > deadline {
            let (state, dialog) = (
                cx.update(|cx| {
                    let s = st.read(cx);
                    format!(
                        "path={:?} loading={} error={:?} entries={:?}",
                        s.remote_path,
                        s.remote_loading,
                        s.remote_error,
                        s.remote_entries.iter().map(|e| &e.name).collect::<Vec<_>>()
                    )
                }),
                win(cx, &fx, |w, cx| {
                    w.render_frame(cx);
                    format!(
                        "open={} ok button={:?} window={:?}",
                        w.try_find("text-prompt").is_some(),
                        w.try_find("prompt-ok").map(|b| b.bounds()),
                        w.viewport_size()
                    )
                }),
            );
            panic!(
                "created.txt never listed: on disk={} prompt busy={} error={:?} dialog: {dialog} {state}",
                work.join("created.txt").exists(),
                cx.update(|cx| prompt.read(cx).busy()),
                cx.update(|cx| prompt.read(cx).last_error().map(str::to_string))
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(work.join("created.txt").exists());

    // Rename dialog is prefilled with the old name.
    let entry = cx.update(|cx| {
        st.read(cx)
            .remote_entries
            .iter()
            .find(|e| e.name == "created.txt")
            .cloned()
            .unwrap()
    });
    let prompt = cx
        .update_window(fx.handle.into(), |_, w, cx| {
            panel.update(cx, |p, cx| {
                p.prompt_rename(Side::Remote, entry.clone(), w, cx)
            })
        })
        .unwrap();
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert_eq!(
            prompt.read(cx).input().read(cx).value().as_ref(),
            "created.txt",
            "prefilled with the current name"
        );
        prompt
            .read(cx)
            .input()
            .clone()
            .update(cx, |s, cx| s.set_value("renamed.txt", w, cx));
        w.press("enter", cx);
    });
    wait_until(cx, "renamed", |cx| {
        cx.update(|cx| {
            st.read(cx)
                .remote_entries
                .iter()
                .any(|e| e.name == "renamed.txt")
        })
    });
    assert!(work.join("renamed.txt").exists() && !work.join("created.txt").exists());

    // Delete asks for confirmation first.
    let entry = cx.update(|cx| {
        st.read(cx)
            .remote_entries
            .iter()
            .find(|e| e.name == "renamed.txt")
            .cloned()
            .unwrap()
    });
    cx.update_window(fx.handle.into(), |_, w, cx| {
        panel.update(cx, |p, cx| {
            p.confirm_delete(Side::Remote, entry.clone(), w, cx)
        })
    })
    .unwrap();
    win(cx, &fx, |w, cx| w.render_frame(cx));
    assert!(
        work.join("renamed.txt").exists(),
        "nothing deleted before confirming"
    );
    win(cx, &fx, |w, cx| w.press("enter", cx));
    let start = std::time::Instant::now();
    while work.join("renamed.txt").exists() && start.elapsed() < std::time::Duration::from_secs(10)
    {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!work.join("renamed.txt").exists());
    let _ = std::fs::remove_dir_all(&work);
}

fn wav_bytes() -> Vec<u8> {
    let (rate, n) = (8000u32, 4000usize);
    let mut data = Vec::new();
    for i in 0..n {
        let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0001_0001, rate, rate * 2, 0x0010_0002] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

#[gpui_kit::test]
fn audio_plays_in_the_preview_and_video_pdf_and_fonts_open_with_the_system_app(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "sftp-media".into(),
                ip: "127.0.0.1".into(),
                port: Some(port),
                auth_type: Some("key".into()),
                username: Some(user.clone()),
                password: None,
                private_key: Some(key.clone()),
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
        .id;
    });
    let dir = std::env::temp_dir().join(format!("opsbatch-media-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tone.wav"), wav_bytes()).unwrap();
    std::fs::write(dir.join("clip.mp4"), b"not really a video").unwrap();
    std::fs::write(dir.join("doc.pdf"), b"%PDF-1.4 fake").unwrap();

    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| {
        wsp.update(cx, |w, cx| {
            w.open_host(&host_id, "sftp-media", "127.0.0.1", cx)
        })
    });
    wait_until(cx, "ssh tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    let st = store(cx);
    let opened: std::rc::Rc<std::cell::RefCell<Vec<std::path::PathBuf>>> = Default::default();
    let o = opened.clone();
    cx.update(|cx| {
        st.update(cx, |s, _| {
            s.set_system_opener(std::rc::Rc::new(move |p, _| {
                o.borrow_mut().push(p.to_path_buf())
            }))
        })
    });
    wait_until(cx, "sftp ready", |cx| {
        cx.update(|cx| st.read(cx).session == SessionState::Ready && !st.read(cx).remote_loading)
    });
    cx.update(|cx| {
        st.update(cx, |s, cx| {
            s.navigate(Side::Remote, &dir.to_string_lossy(), cx)
        })
    });
    wait_until(cx, "listed", |cx| {
        cx.update(|cx| st.read(cx).remote_entries.len() == 3)
    });
    let entry = |cx: &mut TestAppContext, name: &str| {
        cx.update(|cx| {
            st.read(cx)
                .remote_entries
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .unwrap()
        })
    };

    // audio: decoded and offered with playback controls (or a clear error when the machine has no output device)
    let wav = entry(cx, "tone.wav");
    cx.update(|cx| st.update(cx, |s, cx| s.open_preview(Side::Remote, wav, cx)));
    wait_until(cx, "audio data", |cx| {
        cx.update(|cx| {
            st.read(cx)
                .preview
                .as_ref()
                .is_some_and(|p| p.data.is_some())
        })
    });
    win(cx, &fx, |w, cx| w.render_frame(cx));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("sftp-preview-audio").is_some());
        let player =
            w.try_find("sftp-audio-clock").is_some() && w.try_find("sftp-audio-toggle").is_some();
        let no_device = w.try_find("sftp-audio-error").is_some();
        assert!(player ^ no_device, "exactly one of player / error is shown");
        if player {
            w.click("sftp-audio-toggle", cx); // pause
            w.render_frame(cx);
            w.click("sftp-audio-toggle", cx); // resume
        }
        w.click("sftp-preview-close", cx);
    });

    // video / pdf: no hex dump, a button hands the file to the system (remote files are downloaded first)
    for (name, bytes) in [
        ("clip.mp4", b"not really a video".to_vec()),
        ("doc.pdf", b"%PDF-1.4 fake".to_vec()),
    ] {
        let e = entry(cx, name);
        cx.update(|cx| st.update(cx, |s, cx| s.open_preview(Side::Remote, e, cx)));
        win(cx, &fx, |w, cx| {
            w.render_frame(cx);
            assert!(w.try_find("sftp-preview-external").is_some(), "{name}");
            w.click("sftp-open-external", cx);
        });
        let previews = fx.ctx.core.data_dir().join("previews");
        wait_until(cx, "downloaded for the system app", |_| {
            std::fs::read_dir(&previews)
                .into_iter()
                .flatten()
                .flatten()
                .any(|d| {
                    std::fs::read(d.path().join(name))
                        .map(|b| b == bytes)
                        .unwrap_or(false)
                })
        });
        cx.update(|cx| {
            assert!(st
                .read(cx)
                .preview
                .as_ref()
                .is_some_and(|p| !p.loading && p.error.is_none()))
        });
        wait_until(cx, "system app asked to open it", |_| {
            opened
                .borrow()
                .last()
                .is_some_and(|p| p.file_name().unwrap() == name)
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
}
