//! The remote desktop as a BGRA framebuffer that is presented as one GPUI image per UI frame, plus the geometry of
//! the "contain" fit used to map mouse positions back to desktop pixels.

use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::{RenderImage, Window};
use image::{Frame, RgbaImage};

#[derive(Default)]
pub struct RemoteSurface {
    width: u16,
    height: u16,
    /// BGRA, tightly packed (the byte order `RenderImage` expects).
    buf: Vec<u8>,
    dirty: bool,
    has_frame: bool,
}

impl RemoteSurface {
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn has_frame(&self) -> bool {
        self.has_frame
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Replace the framebuffer by a black one of this size.
    pub fn resize(&mut self, width: u16, height: u16) {
        if (width, height) != (self.width, self.height) || self.buf.is_empty() {
            self.width = width;
            self.height = height;
            self.buf = vec![0; usize::from(width) * usize::from(height) * 4];
            self.dirty = true;
        }
    }

    /// Copy a region (`bgra`, `w`×`h`, row-major) to `(x, y)`; the surface grows to `desktop` if needed.
    pub fn apply_bgra(&mut self, desktop: (u16, u16), x: u16, y: u16, w: u16, h: u16, bgra: &[u8]) -> Result<(), String> {
        self.apply(desktop, x, y, w, h, bgra, false)
    }

    /// Same for RGBA input (RDP frames); channels are swapped on the way in.
    pub fn apply_rgba(&mut self, desktop: (u16, u16), x: u16, y: u16, w: u16, h: u16, rgba: &[u8]) -> Result<(), String> {
        self.apply(desktop, x, y, w, h, rgba, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(&mut self, desktop: (u16, u16), x: u16, y: u16, w: u16, h: u16, src: &[u8], swap_rb: bool) -> Result<(), String> {
        if src.len() != usize::from(w) * usize::from(h) * 4 {
            return Err("region size does not match its pixel data".into());
        }
        if u32::from(x) + u32::from(w) > u32::from(desktop.0) || u32::from(y) + u32::from(h) > u32::from(desktop.1) {
            return Err("region outside the desktop".into());
        }
        self.resize(desktop.0, desktop.1);
        let stride = usize::from(self.width) * 4;
        let row_len = usize::from(w) * 4;
        for row in 0..usize::from(h) {
            let dst = (usize::from(y) + row) * stride + usize::from(x) * 4;
            let from = &src[row * row_len..(row + 1) * row_len];
            let to = &mut self.buf[dst..dst + row_len];
            if swap_rb {
                for (d, s) in to.chunks_exact_mut(4).zip(from.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            } else {
                to.copy_from_slice(from);
            }
        }
        self.dirty = true;
        self.has_frame = true;
        Ok(())
    }

    /// A fresh image when something changed since the last call.
    pub fn take_image(&mut self) -> Option<Arc<RenderImage>> {
        if !self.dirty || self.buf.is_empty() {
            return None;
        }
        self.dirty = false;
        let img = RgbaImage::from_raw(u32::from(self.width), u32::from(self.height), self.buf.clone())?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(img)])))
    }

    pub fn pixel(&self, x: u16, y: u16) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let o = (usize::from(y) * usize::from(self.width) + usize::from(x)) * 4;
        self.buf.get(o..o + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// JPEG data URL of the current picture scaled to fit `max_w`×`max_h` (the AI screenshot).
    pub fn screenshot_data_url(&self, max_w: u32, max_h: u32, quality: u8) -> Option<String> {
        if self.width == 0 || self.height == 0 || !self.has_frame {
            return None;
        }
        let (sw, sh) = (u32::from(self.width), u32::from(self.height));
        let scale = (f64::from(max_w) / f64::from(sw)).min(f64::from(max_h) / f64::from(sh)).min(1.0);
        let (w, h) = (((f64::from(sw) * scale).round() as u32).max(1), ((f64::from(sh) * scale).round() as u32).max(1));
        let rgb: Vec<u8> = self.buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
        let full = image::RgbImage::from_raw(sw, sh, rgb)?;
        let scaled = if (w, h) == (sw, sh) { full } else { image::imageops::resize(&full, w, h, image::imageops::FilterType::Triangle) };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).encode_image(&scaled).ok()?;
        Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg)))
    }
}

/// Drop the previous image from the GPU atlas when presenting a new one.
pub fn swap_image(current: &mut Option<Arc<RenderImage>>, new: Arc<RenderImage>, window: &mut Window) {
    if let Some(old) = current.replace(new) {
        let _ = window.drop_image(old);
    }
}

/// The rectangle `(x, y, w, h)` an image of `desktop` size occupies inside `stage` with a "contain" fit.
pub fn contain_rect(stage: (f32, f32, f32, f32), desktop: (u16, u16)) -> Option<(f32, f32, f32)> {
    fit_rect(stage, desktop, true)
}

/// Like [`contain_rect`]; without `upscale` the picture is never shown larger than its own size (the RDP canvas has
/// `max-width/max-height: 100%` and natural size, so a desktop smaller than the stage is centred, not stretched).
pub fn fit_rect(stage: (f32, f32, f32, f32), desktop: (u16, u16), upscale: bool) -> Option<(f32, f32, f32)> {
    let (sx, sy, sw, sh) = stage;
    let (dw, dh) = (f32::from(desktop.0), f32::from(desktop.1));
    if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return None;
    }
    let mut scale = (sw / dw).min(sh / dh);
    if !upscale {
        scale = scale.min(1.0);
    }
    let (w, h) = (dw * scale, dh * scale);
    Some((sx + (sw - w) / 2.0, sy + (sh - h) / 2.0, scale))
}

/// Window position → desktop pixel, clamped to the desktop (like the original `getRemotePoint`).
pub fn map_point(stage: (f32, f32, f32, f32), desktop: (u16, u16), pos: (f32, f32)) -> Option<(u16, u16)> {
    map_point_fit(stage, desktop, pos, true)
}

pub fn map_point_fit(stage: (f32, f32, f32, f32), desktop: (u16, u16), pos: (f32, f32), upscale: bool) -> Option<(u16, u16)> {
    let (ox, oy, scale) = fit_rect(stage, desktop, upscale)?;
    let x = ((pos.0 - ox) / scale).floor().clamp(0.0, f32::from(desktop.0) - 1.0);
    let y = ((pos.1 - oy) / scale).floor().clamp(0.0, f32::from(desktop.1) - 1.0);
    Some((x as u16, y as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_land_at_their_position_and_rgba_is_swapped() {
        let mut s = RemoteSurface::default();
        s.apply_rgba((4, 2), 1, 1, 2, 1, &[10, 20, 30, 0, 40, 50, 60, 0]).unwrap();
        assert_eq!(s.size(), (4, 2));
        assert_eq!(s.pixel(1, 1), Some([30, 20, 10, 255]));
        assert_eq!(s.pixel(2, 1), Some([60, 50, 40, 255]));
        assert_eq!(s.pixel(0, 0), Some([0, 0, 0, 0]));
        assert!(s.has_frame());
        s.apply_bgra((4, 2), 0, 0, 1, 1, &[1, 2, 3, 255]).unwrap();
        assert_eq!(s.pixel(0, 0), Some([1, 2, 3, 255]));
    }

    #[test]
    fn bad_regions_are_rejected_and_resize_clears() {
        let mut s = RemoteSurface::default();
        assert!(s.apply_bgra((4, 2), 3, 0, 2, 1, &[0; 8]).is_err(), "outside");
        assert!(s.apply_bgra((4, 2), 0, 0, 2, 1, &[0; 4]).is_err(), "wrong length");
        s.apply_bgra((4, 2), 0, 0, 1, 1, &[9, 9, 9, 255]).unwrap();
        s.resize(8, 8);
        assert_eq!(s.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(s.pixel(8, 0), None);
    }

    #[test]
    fn dirty_flag_gates_new_images() {
        let mut s = RemoteSurface::default();
        assert!(s.take_image().is_none());
        s.apply_bgra((2, 2), 0, 0, 2, 2, &[0; 16]).unwrap();
        assert!(s.take_image().is_some());
        assert!(s.take_image().is_none());
    }

    #[test]
    fn a_small_desktop_is_centred_not_stretched() {
        let stage = (10.0, 20.0, 1000.0, 800.0);
        let (x, y, scale) = fit_rect(stage, (640, 480), false).unwrap();
        assert_eq!((x, y, scale), (10.0 + 180.0, 20.0 + 160.0, 1.0));
        assert_eq!(map_point_fit(stage, (640, 480), (190.0, 180.0), false), Some((0, 0)));
        assert_eq!(map_point_fit(stage, (640, 480), (190.0 + 100.0, 180.0 + 50.0), false), Some((100, 50)));
        // too big a desktop still shrinks to fit
        assert!(fit_rect((0.0, 0.0, 640.0, 360.0), (1280, 720), false).unwrap().2 < 0.51);
        // the contain fit used by VNC stretches
        assert!(contain_rect(stage, (640, 480)).unwrap().2 > 1.0);
    }

    #[test]
    fn contain_fit_and_point_mapping() {
        // 200x100 stage at (10,20) showing a 100x100 desktop: scale 1, centred horizontally.
        let stage = (10.0, 20.0, 200.0, 100.0);
        assert_eq!(contain_rect(stage, (100, 100)), Some((60.0, 20.0, 1.0)));
        assert_eq!(map_point(stage, (100, 100), (60.0, 20.0)), Some((0, 0)));
        assert_eq!(map_point(stage, (100, 100), (159.9, 119.9)), Some((99, 99)));
        // Outside the picture clamps to the nearest edge.
        assert_eq!(map_point(stage, (100, 100), (0.0, 500.0)), Some((0, 99)));
        // Scaled down 2x.
        assert_eq!(map_point((0.0, 0.0, 100.0, 50.0), (200, 100), (50.0, 25.0)), Some((100, 50)));
        assert_eq!(map_point((0.0, 0.0, 0.0, 50.0), (200, 100), (1.0, 1.0)), None);
    }

    #[test]
    fn screenshots_are_scaled_jpegs() {
        let mut s = RemoteSurface::default();
        assert!(s.screenshot_data_url(100, 100, 70).is_none());
        s.apply_bgra((400, 200), 0, 0, 400, 200, &vec![200; 400 * 200 * 4]).unwrap();
        let url = s.screenshot_data_url(100, 100, 70).unwrap();
        let b64 = url.strip_prefix("data:image/jpeg;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (100, 50));
    }
}
