//! UI scale and readable text colours (ports of `uiScale.ts` / `uiReadability.ts`).

pub const UI_SCALE_MIN: u32 = 90;
pub const UI_SCALE_MAX: u32 = 150;
pub const WINDOWS_DEFAULT_UI_SCALE: u32 = 115;
pub const OTHER_PLATFORM_DEFAULT_UI_SCALE: u32 = 100;
pub const UI_SCALE_OPTIONS: [u32; 5] = [100, 110, 115, 125, 140];

/// Round to the nearest multiple of 5 and clamp to 90..=150; unparsable input -> `fallback`.
pub fn normalize_ui_scale(value: &str, fallback: u32) -> u32 {
    match value.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
        Some(v) => ((v / 5.0).round() * 5.0).clamp(UI_SCALE_MIN as f64, UI_SCALE_MAX as f64) as u32,
        None => fallback,
    }
}

pub fn default_ui_scale_for_platform(platform_text: &str) -> u32 {
    if platform_text.to_lowercase().contains("win") {
        WINDOWS_DEFAULT_UI_SCALE
    } else {
        OTHER_PLATFORM_DEFAULT_UI_SCALE
    }
}

pub fn current_platform_default_ui_scale() -> u32 {
    default_ui_scale_for_platform(std::env::consts::OS)
}

#[derive(Debug, Clone, Copy)]
pub struct UiTextPalette {
    pub primary: &'static str,
    pub secondary: &'static str,
    pub muted: &'static str,
}

pub fn ui_text_palette(is_dark: bool) -> UiTextPalette {
    if is_dark {
        UiTextPalette { primary: "#eef1f8", secondary: "#bec4d3", muted: "#9aa1b5" }
    } else {
        UiTextPalette { primary: "#303446", secondary: "#4d5366", muted: "#535a6d" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_defaults_to_115_percent() {
        assert_eq!(default_ui_scale_for_platform("Windows NT 10.0"), WINDOWS_DEFAULT_UI_SCALE);
        assert_eq!(default_ui_scale_for_platform("windows"), 115);
        assert_eq!(default_ui_scale_for_platform("MacIntel"), 100);
        assert_eq!(default_ui_scale_for_platform("macos"), 100);
    }

    #[test]
    fn scale_is_rounded_and_clamped() {
        assert_eq!(normalize_ui_scale("117", 100), 115);
        assert_eq!(normalize_ui_scale("999", 100), 150);
        assert_eq!(normalize_ui_scale("10", 100), 90);
        assert_eq!(normalize_ui_scale("abc", 110), 110);
        assert_eq!(normalize_ui_scale("", 100), 100);
        assert_eq!(normalize_ui_scale("112.6", 100), 115);
    }
}
