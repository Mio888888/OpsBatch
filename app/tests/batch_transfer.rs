//! Batch upload window against a private sshd (two registered hosts pointing at it).

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use ops_ssh::hosts::NewHost;
use opsbatch_app::transfer::window::BatchTransferWindow;

use common::{serial, wait_until};

#[gpui_kit::test]
fn uploads_a_file_to_every_target_with_templates_and_reports_per_host(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut ids = Vec::new();
    let _fx = common::setup(cx, |core| {
        for (name, p) in [("web-1", port), ("web-2", port), ("down", 1)] {
            ids.push(
                ops_core::rt::block_on(ops_ssh::hosts::add_host(
                    &core.db,
                    NewHost {
                        name: name.into(),
                        ip: "127.0.0.1".into(),
                        port: Some(p),
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
                .id,
            );
        }
    });
    let work = std::env::temp_dir().join(format!("opsbatch-bt-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(work.join("dest-web-1")).unwrap();
    std::fs::create_dir_all(work.join("dest-web-2")).unwrap();
    std::fs::write(work.join("payload.txt"), "batch payload").unwrap();

    let targets = ids.clone();
    let (handle, win) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| BatchTransferWindow::new(targets, window, cx))
        })
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });
    wait_until(cx, "hosts loaded", |cx| {
        cx.update(|cx| win.read(cx).hosts_loaded() == 3)
    });

    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find("batch-upload").disabled(),
            Some(true),
            "needs both paths"
        );
    })
    .unwrap();
    let (local, remote) = cx.update(|cx| win.read(cx).inputs());
    cx.update_window(handle.into(), |_, w, cx| {
        local.update(cx, |s, cx| {
            s.set_value(
                work.join("payload.txt").to_string_lossy().to_string(),
                w,
                cx,
            )
        });
        remote.update(cx, |s, cx| {
            s.set_value(format!("{}/dest-{{host}}/", work.display()), w, cx)
        });
    })
    .unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_ne!(w.find("batch-upload").disabled(), Some(true));
        w.click("batch-upload", cx);
    })
    .unwrap();
    wait_until(cx, "all hosts reported", |cx| {
        cx.update(|cx| !win.read(cx).is_transferring() && win.read(cx).results().len() == 3)
    });
    cx.update(|cx| {
        let r = win.read(cx).results();
        let by = |n: &str| r.iter().find(|x| x.host_name == n).unwrap().clone();
        assert!(by("web-1").success && by("web-2").success);
        assert!(!by("down").success && by("down").error.is_some());
    });
    assert_eq!(
        std::fs::read_to_string(work.join("dest-web-1/payload.txt")).unwrap(),
        "batch payload"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("dest-web-2/payload.txt")).unwrap(),
        "batch payload"
    );
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("batch-result", 2usize)).is_some());
    })
    .unwrap();
    // The completion toast can briefly sit on top of the button and swallow a click, so click until it takes.
    wait_until(cx, "results cleared", |cx| {
        if cx.update(|cx| win.read(cx).results().is_empty()) {
            return true;
        }
        cx.update_window(handle.into(), |_, w, cx| {
            w.render_frame(cx);
            w.click("batch-clear", cx);
        })
        .unwrap();
        false
    });
    let _ = std::fs::remove_dir_all(&work);
}
