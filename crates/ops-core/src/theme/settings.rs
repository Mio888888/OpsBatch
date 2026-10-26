//! Persisted appearance settings (`general_settings` keys shared with the Tauri build) and theme resolution.

use std::collections::HashMap;

use super::ui::{current_platform_default_ui_scale, normalize_ui_scale, ui_text_palette, UiTextPalette};
use super::{accent_by_id, dark_themes, light_themes, theme_by_id, Accent, ThemeDef, ThemeTone, ACCENTS, DEFAULT_ACCENT_ID, DEFAULT_THEME_ID, THEMES};

pub const LEGACY_TERMINAL_FONT_FAMILY: &str = "\"Cascadia Code\", Menlo, Monaco, \"Courier New\", monospace";
pub const DEFAULT_TERMINAL_FONT_FAMILY: &str =
    "\"Cascadia Code\", Consolas, \"Lucida Console\", Menlo, Monaco, \"Courier New\", monospace";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSettingsMode {
    /// Follow the OS light/dark appearance.
    System,
    /// Use the chosen theme.
    Manual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeSettings {
    pub theme_mode: ThemeSettingsMode,
    pub terminal_theme_id: String,
    pub terminal_theme_tone: ThemeTone,
    pub accent_color_id: String,
    pub terminal_font_size: u32,
    pub terminal_font_family: String,
    pub terminal_scrollback: u32,
    pub ui_scale: u32,
    pub reduce_motion: bool,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeSettingsMode::Manual,
            terminal_theme_id: DEFAULT_THEME_ID.into(),
            terminal_theme_tone: ThemeTone::Dark,
            accent_color_id: DEFAULT_ACCENT_ID.into(),
            terminal_font_size: 14,
            terminal_font_family: DEFAULT_TERMINAL_FONT_FAMILY.into(),
            terminal_scrollback: 2000,
            ui_scale: current_platform_default_ui_scale(),
            reduce_motion: false,
        }
    }
}

/// Same normalisation as the Tauri build (legacy default -> new default, `consolai` typo fix).
pub fn normalize_terminal_font_family(value: Option<&str>) -> String {
    let trimmed = value.map(str::trim).unwrap_or("");
    if trimmed.is_empty() || trimmed == LEGACY_TERMINAL_FONT_FAMILY {
        return DEFAULT_TERMINAL_FONT_FAMILY.to_string();
    }
    // /(["']?)consolai\1/gi -> "Consolas"
    let lower = trimmed.to_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while i < trimmed.len() {
        let rest = &lower[i..];
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'');
        let q = quote.map(|c| c.len_utf8()).unwrap_or(0);
        let hit = if rest[q..].starts_with("consolai") {
            let end = q + "consolai".len();
            match quote {
                Some(c) if rest[end..].starts_with(c) => Some(end + c.len_utf8()),
                Some(_) => None,
                None => Some(end),
            }
        } else {
            None
        };
        match hit {
            Some(len) => {
                out.push_str("\"Consolas\"");
                i += len;
            }
            None => {
                let ch = trimmed[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

impl ThemeSettings {
    /// Read from the `general_settings` map; missing/invalid values fall back to defaults.
    pub fn from_map(map: &HashMap<String, String>) -> Self {
        let d = Self::default();
        let get = |k: &str| map.get(k).map(String::as_str).filter(|s| !s.is_empty());
        Self {
            theme_mode: match get("themeMode") {
                Some("system") => ThemeSettingsMode::System,
                Some("manual") => ThemeSettingsMode::Manual,
                _ => d.theme_mode,
            },
            terminal_theme_id: get("terminalThemeId").map(str::to_string).unwrap_or(d.terminal_theme_id),
            terminal_theme_tone: get("terminalThemeTone").and_then(ThemeTone::parse).unwrap_or(d.terminal_theme_tone),
            accent_color_id: get("accentColorId").map(str::to_string).unwrap_or(d.accent_color_id),
            terminal_font_size: get("terminalFontSize").and_then(|s| s.parse().ok()).unwrap_or(d.terminal_font_size),
            terminal_font_family: normalize_terminal_font_family(get("terminalFontFamily")),
            terminal_scrollback: get("terminalScrollback").and_then(|s| s.parse().ok()).unwrap_or(d.terminal_scrollback),
            ui_scale: get("uiScale").map(|s| normalize_ui_scale(s, d.ui_scale)).unwrap_or(d.ui_scale),
            reduce_motion: get("reduceMotion") == Some("true"),
        }
    }

    /// Keys/values written by `save_general_settings` (all stringly typed, as before).
    pub fn to_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("themeMode".into(), if self.theme_mode == ThemeSettingsMode::System { "system" } else { "manual" }.into());
        m.insert("terminalThemeId".into(), self.terminal_theme_id.clone());
        m.insert("terminalThemeTone".into(), self.terminal_theme_tone.as_str().into());
        m.insert("accentColorId".into(), self.accent_color_id.clone());
        m.insert("terminalFontSize".into(), self.terminal_font_size.to_string());
        m.insert("terminalFontFamily".into(), self.terminal_font_family.clone());
        m.insert("terminalScrollback".into(), self.terminal_scrollback.to_string());
        m.insert("uiScale".into(), normalize_ui_scale(&self.ui_scale.to_string(), self.ui_scale).to_string());
        m.insert("reduceMotion".into(), self.reduce_motion.to_string());
        m
    }

    pub async fn load(db: &crate::Database) -> Self {
        match crate::settings::get_general_settings(db).await {
            Ok(map) => Self::from_map(&map),
            Err(_) => Self::default(),
        }
    }

    pub async fn save(&self, db: &crate::Database) -> Result<(), String> {
        crate::settings::save_general_settings(db, self.to_map()).await
    }

    /// Whether animations must be disabled (setting OR the OS preference).
    pub fn effective_reduce_motion(&self, os_reduce_motion: bool) -> bool {
        self.reduce_motion || os_reduce_motion
    }

    pub fn resolve(&self, os_prefers_dark: bool) -> ResolvedTheme {
        let accent = accent_by_id(&self.accent_color_id).unwrap_or(&ACCENTS[0]);
        let (is_dark, theme) = match self.theme_mode {
            ThemeSettingsMode::System => {
                let tone = if os_prefers_dark { ThemeTone::Dark } else { ThemeTone::Light };
                (os_prefers_dark, resolve_theme_for_tone(tone, &self.terminal_theme_id))
            }
            ThemeSettingsMode::Manual => (
                self.terminal_theme_tone == ThemeTone::Dark,
                theme_by_id(&self.terminal_theme_id).unwrap_or(&THEMES[0]),
            ),
        };
        ResolvedTheme { theme, accent, is_dark, text: ui_text_palette(is_dark) }
    }
}

/// Keep the stored theme if its tone matches, otherwise the first theme of that tone.
fn resolve_theme_for_tone(tone: ThemeTone, stored_id: &str) -> &'static ThemeDef {
    if let Some(t) = theme_by_id(stored_id).filter(|t| t.tone == tone) {
        return t;
    }
    match tone {
        ThemeTone::Dark => dark_themes().next().unwrap(),
        ThemeTone::Light => light_themes().next().unwrap(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedTheme {
    pub theme: &'static ThemeDef,
    pub accent: &'static Accent,
    pub is_dark: bool,
    pub text: UiTextPalette,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Core;

    #[test]
    fn defaults_match_the_tauri_build() {
        let d = ThemeSettings::default();
        assert_eq!(d.terminal_theme_id, "tokyo-night-storm");
        assert_eq!((d.terminal_font_size, d.terminal_scrollback), (14, 2000));
        assert_eq!(d.theme_mode, ThemeSettingsMode::Manual);
        assert!(!d.reduce_motion);
    }

    #[test]
    fn map_roundtrip_and_invalid_values() {
        let s = ThemeSettings {
            theme_mode: ThemeSettingsMode::System,
            terminal_theme_id: "dracula".into(),
            terminal_theme_tone: ThemeTone::Dark,
            accent_color_id: "violet".into(),
            terminal_font_size: 16,
            terminal_font_family: "\"JetBrains Mono\", monospace".into(),
            terminal_scrollback: 5000,
            ui_scale: 125,
            reduce_motion: true,
        };
        assert_eq!(ThemeSettings::from_map(&s.to_map()), s);

        let mut bad = HashMap::new();
        bad.insert("terminalFontSize".to_string(), "abc".to_string());
        bad.insert("uiScale".to_string(), "999".to_string());
        bad.insert("terminalThemeTone".to_string(), "purple".to_string());
        let parsed = ThemeSettings::from_map(&bad);
        assert_eq!(parsed.terminal_font_size, 14);
        assert_eq!(parsed.ui_scale, 150);
        assert_eq!(parsed.terminal_theme_tone, ThemeTone::Dark);
    }

    #[test]
    fn font_family_normalisation() {
        assert_eq!(normalize_terminal_font_family(None), DEFAULT_TERMINAL_FONT_FAMILY);
        assert_eq!(normalize_terminal_font_family(Some("  ")), DEFAULT_TERMINAL_FONT_FAMILY);
        assert_eq!(normalize_terminal_font_family(Some(LEGACY_TERMINAL_FONT_FAMILY)), DEFAULT_TERMINAL_FONT_FAMILY);
        assert_eq!(normalize_terminal_font_family(Some("\"Consolai\", monospace")), "\"Consolas\", monospace");
        assert_eq!(normalize_terminal_font_family(Some("consolai, 'Fira Code'")), "\"Consolas\", 'Fira Code'");
        assert_eq!(normalize_terminal_font_family(Some("Menlo")), "Menlo");
    }

    #[test]
    fn resolution_manual_vs_system() {
        let mut s = ThemeSettings { terminal_theme_id: "github-light".into(), terminal_theme_tone: ThemeTone::Light, ..Default::default() };
        let r = s.resolve(true);
        assert_eq!((r.theme.id, r.is_dark), ("github-light", false), "manual ignores the OS");
        s.theme_mode = ThemeSettingsMode::System;
        // OS dark + stored light theme -> first dark theme
        let r = s.resolve(true);
        assert!(r.is_dark && r.theme.tone == ThemeTone::Dark);
        // OS light keeps the stored light theme
        let r = s.resolve(false);
        assert_eq!((r.theme.id, r.is_dark), ("github-light", false));
        // unknown ids fall back
        let s = ThemeSettings { terminal_theme_id: "nope".into(), accent_color_id: "nope".into(), ..Default::default() };
        let r = s.resolve(false);
        assert_eq!((r.theme.id, r.accent.id), (THEMES[0].id, "blue"));
    }

    #[test]
    fn reduce_motion_combines_setting_and_os() {
        let mut s = ThemeSettings::default();
        assert!(!s.effective_reduce_motion(false));
        assert!(s.effective_reduce_motion(true));
        s.reduce_motion = true;
        assert!(s.effective_reduce_motion(false));
    }

    #[tokio::test]
    async fn persists_through_general_settings() {
        let core = Core::open_temp("theme-settings").unwrap();
        assert_eq!(ThemeSettings::load(&core.db).await, ThemeSettings::default());
        let s = ThemeSettings { accent_color_id: "rose".into(), ui_scale: 110, ..Default::default() };
        s.save(&core.db).await.unwrap();
        assert_eq!(ThemeSettings::load(&core.db).await, s);
    }
}
