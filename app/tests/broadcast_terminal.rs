//! Broadcast terminal window against a private sshd.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use ops_ssh::hosts::NewHost;
use opsbatch_app::broadcast::window::BatchTerminalWindow;
use opsbatch_app::terminal::pane::pane_element_id;
use opsbatch_app::terminal::workspace::ConnState;

use common::{serial, wait_until};

#[gpui_kit::test]
fn typing_in_one_pane_reaches_every_connected_host_and_failed_hosts_can_retry(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut ids = Vec::new();
    let _fx = common::setup(cx, |core| {
        for (name, p) in [("alpha", port), ("beta", port), ("down", 1)] {
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
    let targets = ids.clone();
    let (handle, win) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| BatchTerminalWindow::new(targets, window, cx))
        })
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });
    wait_until(cx, "all panes settled", |cx| {
        cx.update(|cx| {
            !win.read(cx).is_loading()
                && win.read(cx).stats().2 == 0
                && !win.read(cx).sessions().is_empty()
        })
    });
    cx.update(|cx| assert_eq!(win.read(cx).stats(), (2, 1, 0), "two connected, one failed"));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("batch-terminal-header").is_some());
        assert!(
            w.try_find(("batch-pane", 0usize)).is_some()
                && w.try_find(("batch-pane", 2usize)).is_some()
        );
        assert!(
            w.try_find(("batch-retry", 2usize)).is_some(),
            "failed host offers retry"
        );
    })
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    cx.run_until_parked();

    // Type into the first pane: both connected hosts run it.
    let sid0 = cx.update(|cx| {
        win.read(cx).sessions()[0]
            .pane
            .as_ref()
            .unwrap()
            .read(cx)
            .session_id
            .clone()
    });
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(pane_element_id(&sid0), cx);
        w.input("echo BC$((1+2))X", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, "both panes echoed", |cx| {
        cx.update(|cx| {
            let s = win.read(cx).sessions();
            [0usize, 1].iter().all(|i| {
                s[*i]
                    .pane
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .buffer_text()
                    .contains("BC3X")
            })
        })
    });
    // The failed host got nothing and has no pane.
    cx.update(|cx| assert!(win.read(cx).sessions()[2].pane.is_none()));

    // Retry goes back to connecting and fails again.
    cx.update_window(handle.into(), |_, w, cx| {
        w.click(("batch-retry", 2usize), cx)
    })
    .unwrap();
    cx.update(|cx| assert_eq!(win.read(cx).sessions()[2].state, ConnState::Connecting));
    wait_until(cx, "retry failed again", |cx| {
        cx.update(|cx| win.read(cx).sessions()[2].state == ConnState::Error)
    });
}
