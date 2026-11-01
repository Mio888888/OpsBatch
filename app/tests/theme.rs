//! Theme + language application through the real settings store (no window needed).

use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::TestAppContext;
use ops_core::theme::{ThemeSettings, ThemeSettingsMode, ThemeTone};
use opsbatch_app::app::init_globals;
use opsbatch_app::theme::{self, css};

/// The UI language is process-wide, so tests in this binary must not interleave.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn boot(cx: &mut TestAppContext, settings: ThemeSettings) -> std::sync::Arc<ops_ssh::Ctx> {
    cx.executor().allow_parking();
    let ctx = ops_ssh::Ctx::new(ops_core::Core::open_temp("theme-ui").unwrap());
    let c = ctx.clone();
    cx.update(move |cx| init_globals(c, ops_i18n::LanguageMode::ZhCn, settings, cx));
    ctx
}

#[gpui_kit::test]
fn default_theme_maps_the_tokyo_night_storm_palette(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    boot(cx, ThemeSettings::default());
    cx.update(|cx| {
        let t = cx.theme();
        assert!(t.is_dark());
        assert_eq!(t.background, css("#222338"), "ui.mainBg");
        assert_eq!(t.sidebar, css("#1f2035"), "ui.sidebarBg");
        assert_eq!(
            t.title_bar,
            css("#282a40"),
            "standalone title bars use ui.cardBg (the main window overrides it with ui.pageBg)"
        );
        assert_eq!(t.primary, css("#1677ff"), "blue accent");
        // macOS/Linux take the text colours from the theme itself, Windows from the fixed UI palette
        assert_eq!(
            t.foreground,
            css(if cfg!(target_os = "windows") {
                "#eef1f8"
            } else {
                "#c0caf5"
            }),
            "UI text"
        );
        assert_eq!(t.danger, css("#ff4d4f"));
    });
}

#[gpui_kit::test]
fn switching_to_a_light_theme_and_accent_repaints_and_persists(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = boot(cx, ThemeSettings::default());
    cx.update(|cx| {
        theme::update(cx, |s| {
            s.terminal_theme_id = "github-light".into();
            s.terminal_theme_tone = ThemeTone::Light;
            s.accent_color_id = "rose".into();
        });
        let t = cx.theme();
        assert!(!t.is_dark());
        assert_eq!(t.primary, css("#f43f5e"));
        let light = ops_core::theme::theme_by_id("github-light").unwrap();
        assert_eq!(
            t.foreground,
            css(if cfg!(target_os = "windows") {
                "#303446"
            } else {
                light.preview.line_strong
            }),
            "UI text"
        );
        assert_eq!(
            t.background,
            css(ops_core::theme::theme_by_id("github-light")
                .unwrap()
                .ui
                .main_bg)
        );
    });
    // The save runs on the background runtime.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        // the settings are written key by key, so a read can see only some of the new values
        let saved = ops_core::rt::block_on(ThemeSettings::load(&ctx.db));
        if saved.accent_color_id == "rose"
            && saved.terminal_theme_id == "github-light"
            && saved.terminal_theme_tone == ThemeTone::Light
        {
            break;
        }
        assert!(Instant::now() < deadline, "settings were not persisted");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn follow_system_mode_tracks_the_os_appearance(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    boot(
        cx,
        ThemeSettings {
            theme_mode: ThemeSettingsMode::System,
            ..Default::default()
        },
    );
    cx.update(|cx| {
        let os_dark = matches!(
            cx.window_appearance(),
            gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
        );
        assert_eq!(cx.theme().is_dark(), os_dark);
        assert_eq!(theme::current(cx).resolved.is_dark, os_dark);
    });
}

#[gpui_kit::test]
fn language_switch_is_immediate_and_persisted(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = boot(cx, ThemeSettings::default());
    cx.update(|cx| {
        opsbatch_app::language::set_mode(ops_i18n::LanguageMode::EnUs, cx);
        assert_eq!(ops_i18n::t!("nav.settings"), "Settings");
        assert_eq!(
            opsbatch_app::language::current_mode(cx),
            ops_i18n::LanguageMode::EnUs
        );
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mode = ops_core::rt::block_on(opsbatch_app::language::load_mode(&ctx.db));
        if mode == ops_i18n::LanguageMode::EnUs {
            break;
        }
        assert!(Instant::now() < deadline, "language not persisted");
        std::thread::sleep(Duration::from_millis(20));
    }
    cx.update(|cx| opsbatch_app::language::set_mode(ops_i18n::LanguageMode::ZhCn, cx));
    assert_eq!(ops_i18n::t!("nav.settings"), "设置");
}
