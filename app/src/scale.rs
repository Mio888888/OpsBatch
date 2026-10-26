//! Interface scale (Settings → Appearance → UI scale). The Tauri build zooms the whole webview, so every length is
//! scaled; here the `px` of the UI modules is this function instead of GPUI's, which multiplies by the current
//! scale. Rem based sizes follow through the theme's font size. Geometry that is not interface (the workflow canvas,
//! window sizes) keeps using GPUI's own `px`.

use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::Pixels;

static SCALE_PERCENT: AtomicU32 = AtomicU32::new(100);

pub fn set_percent(percent: u32) {
    SCALE_PERCENT.store(percent.clamp(50, 300), Ordering::Relaxed);
}

pub fn percent() -> u32 {
    SCALE_PERCENT.load(Ordering::Relaxed)
}

pub fn factor() -> f32 {
    percent() as f32 / 100.0
}

/// A length in logical pixels at 100%, scaled by the interface scale.
pub fn px(value: f32) -> Pixels {
    gpui_kit::px(value * factor())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lengths_follow_the_percentage() {
        super::set_percent(100);
        assert_eq!(f32::from(super::px(10.0)), 10.0);
        super::set_percent(125);
        assert_eq!(f32::from(super::px(10.0)), 12.5);
        super::set_percent(100);
    }
}
