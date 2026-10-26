//! Remote editor window against a private sshd: file mode edit + save, directory mode tree.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use opsbatch_app::editor::window::EditorWindow;
use ops_ssh::hosts::NewHost;

use common::{serial, wait_until};

fn open_editor(cx: &mut TestAppContext, host: &str, dir: bool, path: &str) -> (WindowHandle<gpui_kit::base::Root>, Entity<EditorWindow>) {
    let (host, path) = (host.to_string(), path.to_string());
    cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(700.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| EditorWindow::new(host, dir, path, window, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    })
}

#[gpui_kit::test]
fn editing_and_saving_a_remote_file_and_browsing_a_directory(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let _fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "ed".into(),
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
    let work = std::env::temp_dir().join(format!("opsbatch-editor-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(work.join("conf.d")).unwrap();
    std::fs::write(work.join("app.conf"), "port=80\n中文注释\n").unwrap();
    std::fs::write(work.join("conf.d/extra.sh"), "echo extra\n").unwrap();
    std::fs::write(work.join("zz.txt"), "second file\n").unwrap();
    let file = work.join("app.conf").to_string_lossy().to_string();

    // ---- file mode ----
    let (handle, ed) = open_editor(cx, &host_id, false, &file);
    cx.update(|cx| assert!(ed.read(cx).is_loading() || ed.read(cx).current_path().is_some()));
    wait_until(cx, "file loaded", |cx| cx.update(|cx| ed.read(cx).current_path().is_some()));
    cx.update(|cx| {
        assert_eq!(ed.read(cx).text(cx), "port=80\n中文注释\n", "UTF-8 content intact");
        assert!(!ed.read(cx).is_dirty());
    });
    let state_id = cx.update(|cx| ed.read(cx).editor_state().unwrap().entity_id());
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("editor-window").is_some());
        assert_eq!(w.find("editor-save").disabled(), Some(true), "nothing to save yet");
        w.click(("input", state_id), cx);
        w.input("# edited\n", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert!(ed.read(cx).is_dirty(), "typing marks the buffer dirty");
        assert!(ed.read(cx).text(cx).contains("# edited"));
    });
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("editor-dirty-dot").is_some());
        assert_ne!(w.find("editor-save").disabled(), Some(true));
        w.press(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" }, cx);
    })
    .unwrap();
    wait_until(cx, "saved", |cx| cx.update(|cx| !ed.read(cx).is_saving() && !ed.read(cx).is_dirty()));
    let on_disk = std::fs::read_to_string(work.join("app.conf")).unwrap();
    assert!(on_disk.contains("# edited") && on_disk.contains("中文注释"), "{on_disk:?}");
    cx.update(|cx| assert!(ed.read(cx).error().is_none()));

    // Missing file: error bar, no editor.
    let (h2, missing) = open_editor(cx, &host_id, false, &work.join("nope.txt").to_string_lossy());
    wait_until(cx, "error shown", |cx| cx.update(|cx| missing.read(cx).error().is_some()));
    cx.update_window(h2.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("editor-error").is_some());
    })
    .unwrap();

    // ---- directory mode ----
    let (h3, dir) = open_editor(cx, &host_id, true, &work.to_string_lossy());
    wait_until(cx, "tree loaded", |cx| cx.update(|cx| dir.read(cx).tree().is_some()));
    cx.update_window(h3.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("editor-sidebar").is_some());
        assert!(w.try_find("editor-empty").is_some(), "no file picked yet");
        // conf.d (dir) is row 0 and starts expanded: its child is row 1.
        assert!(w.try_find(("editor-tree-row", 1usize)).is_some());
        // collapse conf.d
        w.click(("editor-tree-row", 0usize), cx);
        w.render_frame(cx);
        // rows now: conf.d, app.conf, zz.txt -> picking row 1 opens app.conf
        w.click(("editor-tree-row", 1usize), cx);
    })
    .unwrap();
    wait_until(cx, "file from tree loaded", |cx| cx.update(|cx| dir.read(cx).current_path().is_some_and(|p| p.ends_with("app.conf"))));
    cx.update(|cx| assert!(dir.read(cx).text(cx).contains("# edited")));

    let _ = std::fs::remove_dir_all(&work);
}
