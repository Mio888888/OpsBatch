//! The remote desktop as a BGRA framebuffer that is presented as GPUI images (one whole image, or tiles of which
//! only the changed ones are rebuilt and uploaded), plus the geometry of the "contain" fit used to map mouse
//! positions back to desktop pixels.

use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::{point, px, size, Bounds, Corners, RenderImage, Window};
use image::{Frame, RgbaImage};

/// Edge of a square tile, in desktop pixels.
pub const TILE: usize = 256;

#[derive(Default)]
pub struct RemoteSurface {
    width: u16,
    height: u16,
    /// BGRA, tightly packed (the byte order `RenderImage` expects).
    buf: Vec<u8>,
    /// Per tile (row-major over [`tile_grid`](Self::tile_grid)): changed since it was last taken.
    tile_dirty: Vec<bool>,
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
            let (cols, rows) = self.tile_grid();
            self.tile_dirty = vec![true; cols * rows];
        }
    }

    /// Tiles across and down.
    pub fn tile_grid(&self) -> (usize, usize) {
        (
            usize::from(self.width).div_ceil(TILE),
            usize::from(self.height).div_ceil(TILE),
        )
    }

    fn mark_tiles(&mut self, x: u16, y: u16, w: u16, h: u16) {
        if w == 0 || h == 0 {
            return;
        }
        let (cols, _) = self.tile_grid();
        let (x0, x1) = (
            usize::from(x) / TILE,
            (usize::from(x) + usize::from(w) - 1) / TILE,
        );
        let (y0, y1) = (
            usize::from(y) / TILE,
            (usize::from(y) + usize::from(h) - 1) / TILE,
        );
        for ty in y0..=y1 {
            self.tile_dirty[ty * cols + x0..=ty * cols + x1].fill(true);
        }
    }

    /// Images for the tiles that changed since the last call, as `(tile index, image)`. A tile image is the tile plus a
    /// one pixel border copied from its neighbours (edges repeat the outermost pixel), so that drawing it scaled
    /// samples the neighbouring picture at the seams instead of whatever sits next to it in the GPU atlas.
    pub fn take_dirty_tiles(&mut self) -> Vec<(usize, Arc<RenderImage>)> {
        let (cols, _) = self.tile_grid();
        let mut out = Vec::new();
        for index in 0..self.tile_dirty.len() {
            if std::mem::take(&mut self.tile_dirty[index]) {
                if let Some(image) = self.tile_image(index % cols, index / cols) {
                    out.push((index, image));
                }
            }
        }
        out
    }

    /// The bytes of tile `(tx, ty)` with the one pixel border, and the size of that padded image.
    fn tile_pixels(&self, tx: usize, ty: usize) -> Option<(Vec<u8>, usize, usize)> {
        let (dw, dh) = (usize::from(self.width), usize::from(self.height));
        if self.buf.len() != dw * dh * 4 || tx * TILE >= dw || ty * TILE >= dh {
            return None;
        }
        let (x0, y0) = (tx * TILE, ty * TILE);
        let (w, h) = ((dw - x0).min(TILE), (dh - y0).min(TILE));
        let (pw, ph) = (w + 2, h + 2);
        let mut out = vec![0u8; pw * ph * 4];
        let pixel = |x: usize, y: usize| -> &[u8] {
            let o = (y * dw + x) * 4;
            &self.buf[o..o + 4]
        };
        for py in 0..ph {
            let sy = (y0 + py).saturating_sub(1).min(dh - 1);
            let row = &mut out[py * pw * 4..(py + 1) * pw * 4];
            row[4..4 + w * 4]
                .copy_from_slice(&self.buf[(sy * dw + x0) * 4..(sy * dw + x0 + w) * 4]);
            row[..4].copy_from_slice(pixel(x0.saturating_sub(1), sy));
            row[(w + 1) * 4..].copy_from_slice(pixel((x0 + w).min(dw - 1), sy));
        }
        Some((out, pw, ph))
    }

    fn tile_image(&self, tx: usize, ty: usize) -> Option<Arc<RenderImage>> {
        let (bytes, pw, ph) = self.tile_pixels(tx, ty)?;
        let img = RgbaImage::from_raw(pw as u32, ph as u32, bytes)?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(img)])))
    }

    /// Copy a region (`bgra`, `w`×`h`, row-major) to `(x, y)`; the surface grows to `desktop` if needed.
    pub fn apply_bgra(
        &mut self,
        desktop: (u16, u16),
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        bgra: &[u8],
    ) -> Result<(), String> {
        self.apply(desktop, x, y, w, h, bgra, false)
    }

    /// Same for RGBA input (RDP frames); channels are swapped on the way in.
    pub fn apply_rgba(
        &mut self,
        desktop: (u16, u16),
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        rgba: &[u8],
    ) -> Result<(), String> {
        self.apply(desktop, x, y, w, h, rgba, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        desktop: (u16, u16),
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        src: &[u8],
        swap_rb: bool,
    ) -> Result<(), String> {
        if src.len() != usize::from(w) * usize::from(h) * 4 {
            return Err("region size does not match its pixel data".into());
        }
        if u32::from(x) + u32::from(w) > u32::from(desktop.0)
            || u32::from(y) + u32::from(h) > u32::from(desktop.1)
        {
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
                for (d, s) in to
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(from.as_chunks::<4>().0)
                {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            } else {
                to.copy_from_slice(from);
            }
        }
        self.mark_tiles(x, y, w, h);
        self.has_frame = true;
        Ok(())
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
        let scale = (f64::from(max_w) / f64::from(sw))
            .min(f64::from(max_h) / f64::from(sh))
            .min(1.0);
        let (w, h) = (
            ((f64::from(sw) * scale).round() as u32).max(1),
            ((f64::from(sh) * scale).round() as u32).max(1),
        );
        let rgb: Vec<u8> = self
            .buf
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        let full = image::RgbImage::from_raw(sw, sh, rgb)?;
        let scaled = if (w, h) == (sw, sh) {
            full
        } else {
            image::imageops::resize(&full, w, h, image::imageops::FilterType::Triangle)
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode_image(&scaled)
            .ok()?;
        Some(format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        ))
    }
}

/// The tile images currently on screen for one remote picture: changed tiles are rebuilt from the surface each frame
/// and the images they replace are freed from the GPU atlas.
#[derive(Default)]
pub struct TileLayer {
    tiles: Vec<Option<Arc<RenderImage>>>,
    grid: (usize, usize),
}

impl TileLayer {
    /// Tiles that have an image (all of them once a frame has been drawn).
    pub fn count(&self) -> usize {
        self.tiles.iter().flatten().count()
    }

    pub fn clear(&mut self, window: &mut Window) {
        for old in self.tiles.drain(..).flatten() {
            let _ = window.drop_image(old);
        }
        self.grid = (0, 0);
    }

    pub fn refresh(&mut self, surface: &mut RemoteSurface, window: &mut Window) {
        let grid = surface.tile_grid();
        if grid != self.grid {
            self.clear(window);
            self.grid = grid;
            self.tiles = vec![None; grid.0 * grid.1];
        }
        for (index, image) in surface.take_dirty_tiles() {
            if let Some(old) = self.tiles[index].replace(image) {
                let _ = window.drop_image(old);
            }
        }
    }

    /// What a paint closure needs (cheap: the images are shared).
    pub fn snapshot(&self) -> TileSnapshot {
        TileSnapshot {
            tiles: self.tiles.clone(),
            grid: self.grid,
        }
    }
}

pub struct TileSnapshot {
    tiles: Vec<Option<Arc<RenderImage>>>,
    grid: (usize, usize),
}

impl TileSnapshot {
    /// Draws the picture with its top-left corner at `(ox, oy)`, `scale` window pixels per desktop pixel. Each tile image
    /// has a one pixel border (see [`RemoteSurface::take_dirty_tiles`]): it is drawn one desktop pixel larger on every
    /// side and clipped to the tile's own rectangle, so scaled tiles blend into their neighbours instead of showing seams.
    pub fn paint(&self, window: &mut Window, desktop: (u16, u16), ox: f32, oy: f32, scale: f32) {
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| {
            Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
        };
        for (index, tile) in self.tiles.iter().enumerate() {
            let Some(image) = tile else { continue };
            let (clip, padded) = tile_rects(index, self.grid.0, desktop, ox, oy, scale);
            let _ = window.paint_image(
                rect(clip),
                rect(padded),
                Corners::default(),
                image.clone(),
                0,
                false,
            );
        }
    }
}

/// The rectangle `(x, y, w, h)` an image of `desktop` size occupies inside `stage` with a "contain" fit.
pub fn contain_rect(stage: (f32, f32, f32, f32), desktop: (u16, u16)) -> Option<(f32, f32, f32)> {
    fit_rect(stage, desktop, true)
}

/// Like [`contain_rect`]; without `upscale` the picture is never shown larger than its own size (the RDP canvas has
/// `max-width/max-height: 100%` and natural size, so a desktop smaller than the stage is centred, not stretched).
pub fn fit_rect(
    stage: (f32, f32, f32, f32),
    desktop: (u16, u16),
    upscale: bool,
) -> Option<(f32, f32, f32)> {
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

/// Where tile `index` of a `grid`-wide tiling of `desktop` goes when the picture is drawn at `(ox, oy)` scaled by
/// `scale`: the tile's own rectangle `(x, y, w, h)` and the rectangle `(x, y, w, h)` its one-pixel-padded image covers.
/// Neighbouring tiles share their edge coordinates exactly, so they leave neither gaps nor overlaps.
pub fn tile_rects(
    index: usize,
    cols: usize,
    desktop: (u16, u16),
    ox: f32,
    oy: f32,
    scale: f32,
) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let (tx, ty) = (index % cols, index / cols);
    let (x0, y0) = (tx * TILE, ty * TILE);
    let (x1, y1) = (
        (x0 + TILE).min(usize::from(desktop.0)),
        (y0 + TILE).min(usize::from(desktop.1)),
    );
    let edge = |v: usize, o: f32| o + v as f32 * scale;
    let tile = (
        edge(x0, ox),
        edge(y0, oy),
        edge(x1, ox) - edge(x0, ox),
        edge(y1, oy) - edge(y0, oy),
    );
    let padded = (
        tile.0 - scale,
        tile.1 - scale,
        tile.2 + 2.0 * scale,
        tile.3 + 2.0 * scale,
    );
    (tile, padded)
}

/// Window position → desktop pixel, clamped to the desktop (like the original `getRemotePoint`).
pub fn map_point(
    stage: (f32, f32, f32, f32),
    desktop: (u16, u16),
    pos: (f32, f32),
) -> Option<(u16, u16)> {
    map_point_fit(stage, desktop, pos, true)
}

pub fn map_point_fit(
    stage: (f32, f32, f32, f32),
    desktop: (u16, u16),
    pos: (f32, f32),
    upscale: bool,
) -> Option<(u16, u16)> {
    let (ox, oy, scale) = fit_rect(stage, desktop, upscale)?;
    let x = ((pos.0 - ox) / scale)
        .floor()
        .clamp(0.0, f32::from(desktop.0) - 1.0);
    let y = ((pos.1 - oy) / scale)
        .floor()
        .clamp(0.0, f32::from(desktop.1) - 1.0);
    Some((x as u16, y as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_land_at_their_position_and_rgba_is_swapped() {
        let mut s = RemoteSurface::default();
        s.apply_rgba((4, 2), 1, 1, 2, 1, &[10, 20, 30, 0, 40, 50, 60, 0])
            .unwrap();
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
        assert!(
            s.apply_bgra((4, 2), 3, 0, 2, 1, &[0; 8]).is_err(),
            "outside"
        );
        assert!(
            s.apply_bgra((4, 2), 0, 0, 2, 1, &[0; 4]).is_err(),
            "wrong length"
        );
        s.apply_bgra((4, 2), 0, 0, 1, 1, &[9, 9, 9, 255]).unwrap();
        s.resize(8, 8);
        assert_eq!(s.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(s.pixel(8, 0), None);
    }

    #[test]
    fn only_the_tiles_a_region_touches_are_rebuilt() {
        let mut s = RemoteSurface::default();
        s.resize(600, 300);
        assert_eq!(s.tile_grid(), (3, 2));
        assert_eq!(
            s.take_dirty_tiles().len(),
            6,
            "a new surface starts with every tile to build"
        );
        assert!(s.take_dirty_tiles().is_empty());
        // a 2x2 patch inside the second tile
        s.apply_bgra((600, 300), 300, 10, 2, 2, &[1; 16]).unwrap();
        assert_eq!(
            s.take_dirty_tiles()
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![1]
        );
        // a region across a tile corner touches all four
        s.apply_bgra((600, 300), 255, 255, 2, 2, &[1; 16]).unwrap();
        assert_eq!(
            s.take_dirty_tiles()
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![0, 1, 3, 4]
        );
        // a region ending exactly on a tile edge does not spill into the next tile
        s.apply_bgra((600, 300), 0, 0, 256, 256, &vec![2; 256 * 256 * 4])
            .unwrap();
        assert_eq!(
            s.take_dirty_tiles()
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn a_tile_carries_a_border_from_its_neighbours() {
        let (w, h) = (300u16, 260u16);
        let mut s = RemoteSurface::default();
        s.resize(w, h);
        // every pixel holds its own x and y (mod 256) so the border can be checked
        let mut all = Vec::new();
        for y in 0..h {
            for x in 0..w {
                all.extend_from_slice(&[x as u8, y as u8, (x >> 8) as u8, 255]);
            }
        }
        s.apply_bgra((w, h), 0, 0, w, h, &all).unwrap();
        // tile (1, 0): x 256..300, y 0..256 -> padded 46 x 258
        let (bytes, pw, ph) = s.tile_pixels(1, 0).unwrap();
        assert_eq!((pw, ph), (46, 258));
        let at = |x: usize, y: usize| &bytes[(y * pw + x) * 4..(y * pw + x) * 4 + 2];
        assert_eq!(at(1, 1), &[0, 0], "first real pixel is desktop (256, 0)");
        assert_eq!(
            at(0, 1),
            &[255, 0],
            "left border is desktop x = 255, the neighbour tile's last column"
        );
        assert_eq!(
            at(45, 1),
            &[43, 0],
            "right border repeats the last desktop column (299)"
        );
        assert_eq!(at(1, 0), &[0, 0], "top border repeats row 0");
        assert_eq!(
            at(1, 257),
            &[0, 0],
            "bottom border is desktop y = 256, the next tile row"
        );
        let (bytes, _, ph) = s.tile_pixels(1, 1).unwrap();
        assert_eq!(ph, 4 + 2, "last tile row is 4 pixels high");
        assert_eq!(&bytes[4 + 1], &255, "its top border is desktop row 255");
    }

    #[test]
    fn neighbouring_tiles_share_their_edges() {
        let desktop = (700, 300);
        let (ox, oy, scale) = (13.5, 7.25, 0.8);
        let rect = |i| tile_rects(i, 3, desktop, ox, oy, scale).0;
        for i in [0usize, 1, 3, 4] {
            let (a, right) = (rect(i), rect(i + 1));
            if i % 3 != 2 {
                assert_eq!(
                    a.0 + a.2,
                    right.0,
                    "tile {i} ends where its right neighbour starts"
                );
            }
        }
        for i in 0..3 {
            let (a, below) = (rect(i), rect(i + 3));
            assert_eq!(
                a.1 + a.3,
                below.1,
                "tile {i} ends where the tile below starts"
            );
        }
        // the whole tiling covers exactly the scaled desktop
        let last = rect(5);
        assert!((last.0 + last.2 - (ox + 700.0 * scale)).abs() < 1e-3);
        assert!((last.1 + last.3 - (oy + 300.0 * scale)).abs() < 1e-3);
        // the padded image reaches one desktop pixel past the tile on every side
        let (tile, padded) = tile_rects(4, 3, desktop, ox, oy, scale);
        assert!(
            (padded.0 - (tile.0 - scale)).abs() < 1e-6
                && (padded.2 - (tile.2 + 2.0 * scale)).abs() < 1e-6
        );
    }

    #[test]
    fn a_small_desktop_is_centred_not_stretched() {
        let stage = (10.0, 20.0, 1000.0, 800.0);
        let (x, y, scale) = fit_rect(stage, (640, 480), false).unwrap();
        assert_eq!((x, y, scale), (10.0 + 180.0, 20.0 + 160.0, 1.0));
        assert_eq!(
            map_point_fit(stage, (640, 480), (190.0, 180.0), false),
            Some((0, 0))
        );
        assert_eq!(
            map_point_fit(stage, (640, 480), (190.0 + 100.0, 180.0 + 50.0), false),
            Some((100, 50))
        );
        // too big a desktop still shrinks to fit
        assert!(
            fit_rect((0.0, 0.0, 640.0, 360.0), (1280, 720), false)
                .unwrap()
                .2
                < 0.51
        );
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
        assert_eq!(
            map_point((0.0, 0.0, 100.0, 50.0), (200, 100), (50.0, 25.0)),
            Some((100, 50))
        );
        assert_eq!(
            map_point((0.0, 0.0, 0.0, 50.0), (200, 100), (1.0, 1.0)),
            None
        );
    }

    #[test]
    fn screenshots_are_scaled_jpegs() {
        let mut s = RemoteSurface::default();
        assert!(s.screenshot_data_url(100, 100, 70).is_none());
        s.apply_bgra((400, 200), 0, 0, 400, 200, &vec![200; 400 * 200 * 4])
            .unwrap();
        let url = s.screenshot_data_url(100, 100, 70).unwrap();
        let b64 = url.strip_prefix("data:image/jpeg;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (100, 50));
    }
}
