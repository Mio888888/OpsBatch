//! Batch command execution window against a private sshd.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use opsbatch_app::batch_exec::player::AsciinemaPlayer;
use opsbatch_app::batch_exec::window::{BatchCommandsWindow, Tab};
use opsbatch_app::batch_exec::Status;
use ops_ssh::hosts::NewHost;

use common::{serial, wait_until};

#[gpui_kit::test]
fn executes_on_selected_hosts_retries_failures_keeps_history_and_replays_recordings(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut ids = Vec::new();
    let fx = common::setup(cx, |core| {
        for (name, p) in [("alpha", port), ("beta", port), ("down", 1)] {
            ids.push(
                ops_core::rt::block_on(ops_ssh::hosts::add_host(
                    &core.db,
                    NewHost { name: name.into(), ip: "127.0.0.1".into(), port: Some(p), auth_type: Some("key".into()), username: Some(user.clone()), password: None, private_key: Some(key.clone()), os: Some("linux".into()), tags: None, group_id: None, remark: None, jump_chain: None, rdp_settings: None, proxy_settings: None },
                ))
                .unwrap()
                .id,
            );
        }
    });
    let assets = cx.update(|cx| cx.global::<opsbatch_app::ui_state::UiState>().assets.clone());
    cx.update(|cx| assets.update(cx, |a, cx| a.reload(cx)));
    wait_until(cx, "hosts loaded", |cx| cx.update(|cx| assets.read(cx).hosts.len() == 3));

    let (handle, win) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| BatchCommandsWindow::new(None, window, cx))).unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });

    // Nothing selected / nothing typed: refuses with a warning, runs nothing.
    cx.update_window(handle.into(), |_, w, cx| {
        win.update(cx, |b, cx| b.execute(w, cx));
    })
    .unwrap();
    cx.update(|cx| assert!(win.read(cx).runs().is_empty() && !win.read(cx).is_executing()));

    cx.update(|cx| assets.update(cx, |a, cx| a.set_selected(ids.clone(), cx)));
    cx.update_window(handle.into(), |_, w, cx| {
        win.update(cx, |b, cx| {
            b.set_command("echo BX$((20+22))", w, cx);
            b.set_limits(5, 20, w, cx);
        });
        w.render_frame(cx);
        w.click("bx-execute", cx);
    })
    .unwrap();
    wait_until(cx, "execution finished", |cx| cx.update(|cx| !win.read(cx).is_executing() && win.read(cx).runs().len() == 3 && win.read(cx).runs().iter().all(|r| !r.status.is_active())));
    cx.update(|cx| {
        let runs = win.read(cx).runs();
        let by = |n: &str| runs.iter().find(|r| r.host_name == n).unwrap().clone();
        assert_eq!(by("alpha").status, Status::Success);
        assert!(by("alpha").output.contains("BX42"), "{:?}", by("alpha").output);
        assert_eq!(by("beta").status, Status::Success);
        assert!(by("down").status.is_failure());
    });
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("bx-stats").is_some());
        assert!(w.try_find(("bx-result", 2usize)).is_some());
        assert!(w.try_find("bx-retry").is_some(), "retry offered for failed hosts");
    })
    .unwrap();

    // Retry re-runs only the failed host (still fails), others keep their result.
    cx.update_window(handle.into(), |_, w, cx| w.click("bx-retry", cx)).unwrap();
    wait_until(cx, "retry finished", |cx| cx.update(|cx| !win.read(cx).is_executing()));
    cx.update(|cx| {
        let runs = win.read(cx).runs();
        assert_eq!(runs.iter().filter(|r| r.status == Status::Success).count(), 2);
        assert!(runs.iter().find(|r| r.host_name == "down").unwrap().status.is_failure());
    });

    // Dangerous commands wait for confirmation.
    cx.update_window(handle.into(), |_, w, cx| {
        win.update(cx, |b, cx| {
            b.clear(cx);
            b.set_command("rm -rf /nonexistent-opsbatch-test", w, cx);
            b.execute(w, cx);
        });
    })
    .unwrap();
    cx.update(|cx| assert!(win.read(cx).runs().is_empty(), "blocked until confirmed"));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, "confirmed run", |cx| cx.update(|cx| win.read(cx).runs().len() == 3));
    wait_until(cx, "danger run done", |cx| cx.update(|cx| !win.read(cx).is_executing()));

    // History lists the runs; the recording of a host can be replayed.
    cx.update_window(handle.into(), |_, w, cx| {
        let _ = w;
        win.update(cx, |b, cx| b.set_tab(Tab::History, cx));
    })
    .unwrap();
    wait_until(cx, "history rows", |cx| cx.update(|cx| win.read(cx).history().len() >= 2));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("bx-history-row", 0usize)).is_some());
    })
    .unwrap();
    // Several runs echo BX (the retry of the failed hosts too, and history order of runs started in the same second
    // is arbitrary): use the one that actually ran on the reachable host, i.e. the one with its recording.
    let alpha = ids[0].clone();
    let core = fx.ctx.core.clone();
    let mut echo_run = None;
    wait_until(cx, "a BX run with alpha's recording", |cx| {
        let candidates: Vec<_> = cx.update(|cx| win.read(cx).history().iter().filter(|h| h.command.contains("BX")).cloned().collect());
        echo_run = candidates.into_iter().find(|h| ops_core::asciinema::list_recordings(&core, h.id.clone()).map(|r| r.iter().any(|m| m.host_id == alpha)).unwrap_or(false));
        echo_run.is_some()
    });
    let echo_run = echo_run.unwrap();
    let player = cx.update_window(handle.into(), |_, w, cx| cx.new(|cx| AsciinemaPlayer::new(echo_run.id.clone(), w, cx))).unwrap();
    wait_until(cx, "recordings listed", |cx| cx.update(|cx| !player.read(cx).recordings().is_empty()));
    // Directory order is arbitrary: pick the reachable host's recording explicitly.
    cx.update_window(handle.into(), |_, w, cx| player.update(cx, |p, cx| p.load_recording(&alpha, w, cx))).unwrap();
    wait_until(cx, "alpha recording loaded", |cx| {
        cx.update(|cx| {
            player.update(cx, |p, cx| p.skip_to_end(cx));
            player.read(cx).output().contains("BX42")
        })
    });
    cx.update(|cx| player.update(cx, |p, cx| p.skip_to_end(cx)));
    cx.update(|cx| assert!(player.read(cx).output().contains("BX42"), "{:?}", player.read(cx).output()));
    cx.update(|cx| player.update(cx, |p, cx| {
        p.reset(cx);
        p.set_speed(10.0);
        p.play(cx);
    }));
    // Playback is timer driven: step the test clock instead of sleeping.
    for _ in 0..200 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        if cx.update(|cx| player.read(cx).position() == player.read(cx).event_count()) {
            break;
        }
    }
    cx.update(|cx| assert_eq!(player.read(cx).position(), player.read(cx).event_count(), "playback reached the end"));
    cx.update(|cx| assert!(player.read(cx).output().contains("BX42")));
}
