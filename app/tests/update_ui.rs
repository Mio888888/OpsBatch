//! Update button, dialog and install flow against a local release mirror (no network, installer opener stubbed).

mod common;

use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use ops_core::update::{installer_asset_name, UpdateSource};
use opsbatch_app::update::{update_store, Phase, UpdateEvent};

use common::mirror::{payload, start, TAG};
use common::{serial, wait_until};

type Opened = Arc<Mutex<Vec<std::path::PathBuf>>>;

fn configure(cx: &mut TestAppContext, base: &str, current: &str, dir: &std::path::Path) -> Opened {
    let opened: Opened = Arc::default();
    let o = opened.clone();
    let (base, dir, current) = (base.to_string(), dir.to_path_buf(), current.to_string());
    cx.update(move |cx| {
        update_store(cx).update(cx, |s, _| {
            s.configure(
                // the mock release has no .sig files, whatever updater key the build or the environment carries
                UpdateSource {
                    pubkey: None,
                    ..UpdateSource::with_base(&base)
                },
                &current,
                "macos",
                "aarch64",
                dir,
                Arc::new(move |p| {
                    o.lock().unwrap().push(p.to_path_buf());
                    Ok(())
                }),
            )
        })
    });
    opened
}

#[gpui_kit::test]
fn update_available_shows_dot_downloads_verifies_and_hands_over_the_installer(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mirror = start(installer_asset_name(TAG, "macos", "aarch64").unwrap());
    let dir = std::env::temp_dir().join(format!("opsbatch-ui-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let opened = configure(cx, &mirror.base, "0.1.0", &dir);
    let store = cx.update(|cx| update_store(cx));

    // no update known yet: no dot
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("update-dot").is_none());
    })
    .unwrap();

    // silent startup check -> red dot on the button
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let e = events.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&store, move |_, ev: &UpdateEvent, _| {
            e.lock().unwrap().push(format!("{ev:?}"))
        })
    });
    cx.update(|cx| store.update(cx, |s, cx| s.check(true, cx)));
    wait_until(cx, "silent check", |cx| {
        cx.update(|cx| store.read(cx).has_update())
    });
    assert!(
        events.lock().unwrap().is_empty(),
        "silent checks do not announce themselves"
    );
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("update-dot").is_some());
    })
    .unwrap();

    // the button opens the dialog with versions and notes
    cx.update_window(fx.handle.into(), |_, w, cx| w.click("tool-update", cx))
        .unwrap();
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("update-modal").is_some() && w.try_find("update-action").is_some());
    })
    .unwrap();
    cx.update(|cx| {
        let info = store.read(cx).info.clone().unwrap();
        assert_eq!(
            (
                info.current_version.as_str(),
                info.latest_version.as_deref()
            ),
            ("0.1.0", Some(TAG))
        );
        assert!(info.release_notes.unwrap().contains("mirror notes"));
    });

    // download + verify + hand over
    cx.update_window(fx.handle.into(), |_, w, cx| w.click("update-action", cx))
        .unwrap();
    wait_until(cx, "ready", |cx| {
        cx.update(|cx| store.read(cx).phase == Phase::Ready)
    });
    let path = cx.update(|cx| store.read(cx).installer.clone().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), payload());
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        std::slice::from_ref(&path)
    );
    cx.update(|cx| {
        let s = store.read(cx);
        assert_eq!(
            (s.downloaded, s.total, s.percent()),
            (payload().len() as u64, Some(payload().len() as u64), 100)
        );
    });
    assert!(events.lock().unwrap().iter().any(|e| e.contains("Ready")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn tampered_download_fails_without_opening_anything_and_can_be_reset(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let mirror = start(installer_asset_name(TAG, "macos", "aarch64").unwrap());
    mirror
        .corrupt
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("opsbatch-ui-update-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let opened = configure(cx, &mirror.base, "0.1.0", &dir);
    let store = cx.update(|cx| update_store(cx));

    cx.update(|cx| store.update(cx, |s, cx| s.check(true, cx)));
    wait_until(cx, "check", |cx| {
        cx.update(|cx| store.read(cx).has_update())
    });
    cx.update(|cx| store.update(cx, |s, cx| s.install(cx)));
    wait_until(cx, "error", |cx| {
        cx.update(|cx| store.read(cx).phase == Phase::Error)
    });
    cx.update(|cx| assert!(store.read(cx).error.clone().unwrap().contains("SHA-256")));
    assert!(
        opened.lock().unwrap().is_empty(),
        "an unverified installer is never opened"
    );
    assert!(std::fs::read_dir(&dir)
        .map(|d| d.count() == 0)
        .unwrap_or(true));

    cx.update(|cx| store.update(cx, |s, cx| s.reset_install(cx)));
    cx.update(|cx| assert_eq!(store.read(cx).phase, Phase::Idle));

    // fixing the mirror makes a retry succeed
    mirror
        .corrupt
        .store(false, std::sync::atomic::Ordering::SeqCst);
    cx.update(|cx| store.update(cx, |s, cx| s.install(cx)));
    wait_until(cx, "ready after retry", |cx| {
        cx.update(|cx| store.read(cx).phase == Phase::Ready)
    });
    assert_eq!(opened.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn up_to_date_and_failed_checks_report_to_the_user(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mirror = start(installer_asset_name(TAG, "macos", "aarch64").unwrap());
    let dir = std::env::temp_dir().join(format!("opsbatch-ui-update-none-{}", std::process::id()));
    let _opened = configure(cx, &mirror.base, "99.0.0", &dir);
    let store = cx.update(|cx| update_store(cx));
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let e = events.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&store, move |_, ev: &UpdateEvent, _| {
            e.lock().unwrap().push(format!("{ev:?}"))
        })
    });

    cx.update(|cx| store.update(cx, |s, cx| s.check(false, cx)));
    wait_until(cx, "up to date", |cx| {
        cx.update(|cx| store.read(cx).info.is_some())
    });
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.contains("UpToDate")));
    cx.update(|cx| {
        assert!(!store.read(cx).has_update());
        // nothing to install: the action is a no-op
        store.update(cx, |s, cx| s.install(cx));
        assert_eq!(store.read(cx).phase, Phase::Idle);
    });
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("update-dot").is_none());
    })
    .unwrap();

    // unreachable mirror: loud for manual checks
    let _opened = configure(cx, "http://127.0.0.1:9", "0.1.0", &dir);
    cx.update(|cx| store.update(cx, |s, cx| s.check(false, cx)));
    wait_until(cx, "check failed", |_cx| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.contains("CheckFailed"))
    });
}
