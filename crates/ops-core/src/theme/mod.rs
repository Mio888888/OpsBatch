//! Themes: the 14 terminal/UI palettes and 13 accent colours exported from the Tauri build, plus colour math,
//! UI scale rules and the persisted `ThemeSettings` (TASKS M2-02/03). Pure data/logic — the GPUI mapping lives in
//! the app crate.

mod color;
mod data;
mod settings;
mod ui;

pub use color::{contrast_ratio, darken, lighten, parse_css_color, Rgba8};
pub use data::{ACCENTS, THEMES};
pub use settings::{
    normalize_terminal_font_family, ResolvedTheme, ThemeSettings, ThemeSettingsMode,
};
pub use ui::{
    default_ui_scale_for_platform, normalize_ui_scale, ui_text_palette, UiTextPalette,
    OTHER_PLATFORM_DEFAULT_UI_SCALE, UI_SCALE_MAX, UI_SCALE_MIN, UI_SCALE_OPTIONS,
    WINDOWS_DEFAULT_UI_SCALE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeTone {
    Dark,
    Light,
}

impl ThemeTone {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeTone::Dark => "dark",
            ThemeTone::Light => "light",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// CSS colour strings (`#rrggbb`, `#rgb` or `rgba(r, g, b, a)`), exactly as in the Tauri theme data.
#[derive(Debug, Clone, Copy)]
pub struct PreviewPalette {
    pub background: &'static str,
    pub line_strong: &'static str,
    pub line_medium: &'static str,
    pub line_weak: &'static str,
    pub accent: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct UiPalette {
    pub page_bg: &'static str,
    pub sidebar_bg: &'static str,
    pub main_bg: &'static str,
    pub card_bg: &'static str,
    pub header_bg: &'static str,
    pub border: &'static str,
    pub border_strong: &'static str,
    pub input_bg: &'static str,
    pub input_border: &'static str,
    pub nav_hover: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalPalette {
    pub background: &'static str,
    pub foreground: &'static str,
    pub cursor: &'static str,
    pub selection_background: &'static str,
    pub black: &'static str,
    pub red: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub blue: &'static str,
    pub magenta: &'static str,
    pub cyan: &'static str,
    pub white: &'static str,
    pub bright_black: &'static str,
    pub bright_red: &'static str,
    pub bright_green: &'static str,
    pub bright_yellow: &'static str,
    pub bright_blue: &'static str,
    pub bright_magenta: &'static str,
    pub bright_cyan: &'static str,
    pub bright_white: &'static str,
}

impl TerminalPalette {
    /// ANSI colours 0..=15 in order.
    pub fn ansi16(&self) -> [&'static str; 16] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
            self.bright_black,
            self.bright_red,
            self.bright_green,
            self.bright_yellow,
            self.bright_blue,
            self.bright_magenta,
            self.bright_cyan,
            self.bright_white,
        ]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeDef {
    pub id: &'static str,
    pub name: &'static str,
    pub tone: ThemeTone,
    pub preview: PreviewPalette,
    pub ui: UiPalette,
    pub terminal: TerminalPalette,
}

#[derive(Debug, Clone, Copy)]
pub struct Accent {
    pub id: &'static str,
    pub name: &'static str,
    pub base: &'static str,
    pub strong: &'static str,
}

pub fn dark_themes() -> impl Iterator<Item = &'static ThemeDef> {
    THEMES.iter().filter(|t| t.tone == ThemeTone::Dark)
}

pub fn light_themes() -> impl Iterator<Item = &'static ThemeDef> {
    THEMES.iter().filter(|t| t.tone == ThemeTone::Light)
}

pub fn theme_by_id(id: &str) -> Option<&'static ThemeDef> {
    THEMES.iter().find(|t| t.id == id)
}

pub fn accent_by_id(id: &str) -> Option<&'static Accent> {
    ACCENTS.iter().find(|a| a.id == id)
}

pub const DEFAULT_THEME_ID: &str = "tokyo-night-storm";
pub const DEFAULT_ACCENT_ID: &str = "blue";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_is_complete() {
        assert_eq!(THEMES.len(), 14);
        assert_eq!(dark_themes().count(), 8);
        assert_eq!(light_themes().count(), 6);
        assert_eq!(ACCENTS.len(), 13);
        assert!(theme_by_id(DEFAULT_THEME_ID).is_some());
        assert!(accent_by_id(DEFAULT_ACCENT_ID).is_some());
    }

    #[test]
    fn every_colour_string_parses() {
        for t in THEMES {
            for c in [
                t.preview.background,
                t.preview.line_strong,
                t.preview.line_medium,
                t.preview.line_weak,
                t.preview.accent,
                t.ui.page_bg,
                t.ui.sidebar_bg,
                t.ui.main_bg,
                t.ui.card_bg,
                t.ui.header_bg,
                t.ui.border,
                t.ui.border_strong,
                t.ui.input_bg,
                t.ui.input_border,
                t.ui.nav_hover,
                t.terminal.background,
                t.terminal.foreground,
                t.terminal.cursor,
                t.terminal.selection_background,
            ] {
                assert!(parse_css_color(c).is_some(), "{}: cannot parse {c:?}", t.id);
            }
            for c in t.terminal.ansi16() {
                assert!(parse_css_color(c).is_some(), "{}: bad ansi {c:?}", t.id);
            }
        }
        for a in ACCENTS {
            assert!(
                parse_css_color(a.base).is_some() && parse_css_color(a.strong).is_some(),
                "{}",
                a.id
            );
        }
    }

    #[test]
    fn ids_are_unique_and_tone_matches_background_luminance() {
        let mut ids: Vec<_> = THEMES.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), THEMES.len());
        for t in THEMES {
            let bg = parse_css_color(t.terminal.background).unwrap();
            let dark = bg.relative_luminance() < 0.4;
            assert_eq!(
                dark,
                t.tone == ThemeTone::Dark,
                "{} claims {:?}",
                t.id,
                t.tone
            );
        }
    }

    /// Port of windowsUiReadability.test.ts: light-theme secondary/muted UI text stays >= 4.5:1.
    #[test]
    fn light_theme_secondary_and_muted_text_meet_contrast() {
        let p = ui_text_palette(false);
        for t in light_themes() {
            for bg in [t.ui.page_bg, t.ui.sidebar_bg, t.ui.main_bg, t.ui.card_bg] {
                let bg = parse_css_color(bg).unwrap();
                assert!(
                    contrast_ratio(parse_css_color(p.secondary).unwrap(), bg) >= 4.5,
                    "{} secondary",
                    t.id
                );
                assert!(
                    contrast_ratio(parse_css_color(p.muted).unwrap(), bg) >= 4.5,
                    "{} muted",
                    t.id
                );
            }
        }
    }
}
