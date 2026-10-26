//! Application bootstrap shared by `main` and the headless tests.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, KeyBinding};
use ops_core::theme::ThemeSettings;
use ops_i18n::t;
use ops_ssh::Ctx;

use crate::window::{self, WindowKind};
use crate::workbench::Workbench;

gpui_kit::actions!(opsbatch, [Quit]);

/// Open the main window (or focus it). Shared state lives in `UiState`, so a window re-created from the tray
/// comes back with the same mode, sidebar and selection.
pub fn open_main_window(cx: &mut App) {
    let options = window::window_options(WindowKind::Main, &t!("app.title"), cx);
    window::open_or_focus("main", options, cx, |window, cx| {
        window
            .observe_window_appearance(|_, cx| crate::theme::on_system_appearance_changed(cx))
            .detach();
        let assets = cx.global::<crate::ui_state::UiState>().assets.clone();
        cx.new(|cx| Workbench::new(assets, window, cx))
    });
}

/// Everything that must exist before the first window: services, language, theme, registry, UI state.
pub fn init_globals(ctx: Arc<Ctx>, language_mode: ops_i18n::LanguageMode, theme: ThemeSettings, cx: &mut App) {
    gpui_kit::init(cx);
    crate::services::init(ctx, cx);
    crate::language::init(language_mode, cx);
    crate::theme::init(theme, cx);
    window::init(cx);
    crate::ui_state::init(cx);
    let ctx = crate::services::ctx(cx);
    crate::terminal::workspace::init(ctx, false, cx);
    crate::sftp::store::init(cx);
    crate::library::store::init(cx);
    crate::ai::store::init(cx);
    crate::ai::runtime::init(cx);
    crate::workflow::store::init(cx);
    crate::log::store::init(cx);
    crate::update::init(cx);
    crate::workbench::motion::init(cx);
}

pub fn run(ctx: Arc<Ctx>) {
    let (language_mode, theme) = ops_core::rt::block_on(async {
        (crate::language::load_mode(&ctx.db).await, ThemeSettings::load(&ctx.db).await)
    });
    let exit_after: Option<u64> = std::env::var("OPSBATCH_EXIT_AFTER").ok().and_then(|s| s.parse().ok());

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .run(move |cx| {
            init_globals(ctx.clone(), language_mode, theme, cx);
            crate::terminal::workspace::workspace(cx).update(cx, |ws, _| ws.set_auto_local(true));

            cx.on_action(|_: &Quit, cx| {
                // Hang up every shell / SSH channel instead of leaving them to time out remotely.
                crate::terminal::workspace::workspace(cx).update(cx, |w, cx| w.close_all_sessions(cx));
                cx.quit();
            });
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("ctrl-q", Quit, None)]);

            // Closing the last window quits unless "minimize to tray" is on.
            let db = ctx.db.clone();
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() && !ops_core::settings::minimize_to_tray(&db) {
                    cx.quit();
                }
            })
            .detach();

            crate::tray::init(cx, open_main_window);
            open_main_window(cx);

            // Quietly look for a newer release (the update button shows a dot when there is one).
            if std::env::var_os("OPSBATCH_SKIP_UPDATE_CHECK").is_none() {
                let store = crate::update::update_store(cx);
                store.update(cx, |s, cx| s.check(true, cx));
            }

            // Repositories marked "update on startup" are pulled in the background; the libraries refresh afterwards.
            if std::env::var_os("OPSBATCH_SKIP_REPO_UPDATE").is_none() {
                let core = ctx.core.clone();
                cx.spawn(async move |cx| {
                    let results = ops_core::rt::run(async move { ops_library::github::run_startup_repo_updates(&core).await }).await.and_then(|r| r);
                    match results {
                        Ok(r) if !r.is_empty() => {
                            let _ = cx.update(|cx| crate::library::store::library_store(cx).update(cx, |l, cx| l.load_all(cx)));
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("startup repository update failed: {e}"),
                    }
                })
                .detach();
            }

            if let Some(secs) = exit_after {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(secs)).await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
}
