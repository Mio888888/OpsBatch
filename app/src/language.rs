//! UI language (system / zh-CN / en-US), persisted as `languageMode` in `general_settings`.

use gpui_kit::{App, Global};
use ops_i18n::{Language, LanguageMode};

const KEY: &str = "languageMode";

pub struct LanguageState {
    pub mode: LanguageMode,
}

impl Global for LanguageState {}

fn apply(mode: LanguageMode) {
    let lang = mode.resolve();
    ops_i18n::set_current_language(lang);
    // gpui-component ships its own strings (dialog buttons, calendar, ...).
    gpui_kit::component::set_locale(match lang {
        Language::ZhCn => "zh-CN",
        Language::EnUs => "en",
    });
}

/// Read the stored mode (default: follow the system) and make it current.
pub async fn load_mode(db: &ops_core::Database) -> LanguageMode {
    let map = ops_core::settings::get_general_settings(db).await.unwrap_or_default();
    map.get(KEY).and_then(|v| LanguageMode::parse(v)).unwrap_or(LanguageMode::System)
}

pub fn init(mode: LanguageMode, cx: &mut App) {
    apply(mode);
    cx.set_global(LanguageState { mode });
}

pub fn current_mode(cx: &App) -> LanguageMode {
    cx.global::<LanguageState>().mode
}

/// Switch language everywhere immediately and persist it.
pub fn set_mode(mode: LanguageMode, cx: &mut App) {
    apply(mode);
    cx.global_mut::<LanguageState>().mode = mode;
    let ctx = crate::services::ctx(cx);
    ops_core::rt::spawn(async move {
        let mut m = std::collections::HashMap::new();
        m.insert(KEY.to_string(), mode.as_str().to_string());
        if let Err(error) = ops_core::settings::save_general_settings(&ctx.db, m).await {
            log::warn!("could not save language mode: {error}");
        }
    });
    cx.refresh_windows();
}
