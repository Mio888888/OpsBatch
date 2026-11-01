//! Shared fixture for headless UI tests: temp database, services, a Workbench window.
#![allow(dead_code)]

pub mod mirror;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle};
use ops_core::theme::ThemeSettings;
use ops_ssh::hosts::NewHost;
use opsbatch_app::app::init_globals;
use opsbatch_app::stores::assets::AssetsStore;
use opsbatch_app::ui_state::UiState;
use opsbatch_app::workbench::Workbench;

/// The UI language is process-wide, so tests in one binary must not interleave.
pub static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn new_host(name: &str, ip: &str, os: &str, group: Option<&str>) -> NewHost {
    NewHost {
        name: name.into(),
        ip: ip.into(),
        port: Some(22),
        auth_type: Some("password".into()),
        username: Some("root".into()),
        password: None,
        private_key: None,
        os: Some(os.into()),
        tags: None,
        group_id: group.map(Into::into),
        remark: Some(String::new()),
        jump_chain: None,
        rdp_settings: None,
        proxy_settings: None,
    }
}

pub struct Fixture {
    pub handle: WindowHandle<gpui_kit::base::Root>,
    pub workbench: Entity<Workbench>,
    pub ctx: Arc<ops_ssh::Ctx>,
}

/// `seed` runs against the real services before the UI exists.
pub fn setup(cx: &mut TestAppContext, seed: impl FnOnce(&ops_core::Core)) -> Fixture {
    cx.executor().allow_parking();
    let core = ops_core::Core::open_temp("ui").unwrap();
    seed(&core);
    let ctx = ops_ssh::Ctx::new(core);
    let c2 = ctx.clone();
    cx.update(move |cx| {
        init_globals(
            c2,
            ops_i18n::LanguageMode::ZhCn,
            // Windows starts at 115%; the tests lay things out for 100% like the other platforms
            ThemeSettings {
                ui_scale: 100,
                ..ThemeSettings::default()
            },
            cx,
        )
    });
    let (handle, workbench) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let assets = cx.global::<UiState>().assets.clone();
                cx.new(|cx| Workbench::new(assets, window, cx))
            },
        )
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    Fixture {
        handle,
        workbench,
        ctx,
    }
}

/// DB work runs on the tokio runtime; wait for the result to reach the UI thread.
pub fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut cond: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if cond(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Click `id`, and click it again until `taken` reports that the click had its effect. A click on a dialog button
/// right after the dialog opened sometimes goes nowhere on a loaded CI machine.
pub fn click_until(
    cx: &mut TestAppContext,
    fx: &Fixture,
    id: &'static str,
    mut taken: impl FnMut(&mut TestAppContext) -> bool,
) {
    use gpui_kit::test::TestWindowExt;
    for attempt in 0..20 {
        cx.update_window(fx.handle.into(), |_, w, cx| {
            w.render_frame(cx);
            w.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
        if taken(cx) {
            return;
        }
        assert!(attempt < 19, "clicking {id} never had an effect");
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn assets(cx: &mut TestAppContext) -> Entity<AssetsStore> {
    cx.update(|cx| cx.global::<UiState>().assets.clone())
}

/// Open the sidebar and wait until the store has loaded `expected` hosts.
pub fn open_sidebar(cx: &mut TestAppContext, fx: &Fixture, expected: usize) -> Entity<AssetsStore> {
    use gpui_kit::test::TestWindowExt;
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("open-assets", cx)
    })
    .unwrap();
    let store = assets(cx);
    wait_until(cx, "hosts loaded", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == expected && !store.read(cx).loading)
    });
    store
}
