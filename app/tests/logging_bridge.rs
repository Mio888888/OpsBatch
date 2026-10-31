//! Warnings/errors of our crates, panics and error toasts reach the global log and the diagnostics file.
//! (Own test binary: it installs the process-wide logger and panic hook.)

mod common;

use gpui_kit::{AppContext as _, TestAppContext};
use opsbatch_app::logging;

use common::{serial, wait_until};

#[gpui_kit::test]
fn bridged_logs_panics_and_error_toasts_are_stored_with_secrets_masked(cx: &mut TestAppContext) {
    let _s = serial();
    logging::BridgeLogger::init("warn");
    let fx = common::setup(cx, |_| {});
    logging::attach(&fx.ctx.core);
    let core = fx.ctx.core.clone();
    let history = || ops_core::log::get_log_history(&core, Some(50)).unwrap();

    log::error!(target: "opsbatch_app::test", "connect failed token=abc123");
    log::warn!(target: "gpui_apple::text_system", "font chatter");
    log::info!(target: "opsbatch_app::test", "not bridged");
    let h = history();
    let mine: Vec<_> = h
        .iter()
        .filter(|e| e.source == "opsbatch_app::test")
        .collect();
    assert_eq!(mine.len(), 1, "{h:?}");
    assert_eq!(
        (
            mine[0].level.as_str(),
            mine[0].origin.as_str(),
            mine[0].message.as_str()
        ),
        ("error", "frontend", "connect failed token=***")
    );
    assert!(h.iter().all(|e| e.source != "gpui_apple::text_system"));

    // a panic on another thread is recorded with its location
    let _ = std::thread::spawn(|| panic!("worker exploded")).join();
    assert!(history().iter().any(|e| e.source == "panic"
        && e.level == "error"
        && e.message.contains("worker exploded")
        && e.message.contains("logging_bridge.rs")));

    // error toasts are logged, success toasts are not
    cx.update_window(fx.handle.into(), |_, w, cx| {
        opsbatch_app::widgets::toast::error(w, cx, "保存失败 password=hunter2");
        opsbatch_app::widgets::toast::success(w, cx, "已保存");
    })
    .unwrap();
    wait_until(cx, "toast logged", |_| {
        history()
            .iter()
            .any(|e| e.source == "ui.error" && e.message == "保存失败 password=***")
    });
    assert!(history().iter().all(|e| e.message != "已保存"));

    let diag = std::fs::read_to_string(core.data_dir().join("opsbatch-diagnostics.log")).unwrap();
    assert!(
        diag.contains("worker exploded")
            && diag.contains("[frontend.ui.error]")
            && !diag.contains("hunter2")
            && !diag.contains("abc123")
    );
}
