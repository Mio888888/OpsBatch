//! Software H.264 decoding for the `H264Direct` transport (EGFX AVC420 / RDPEVOR). The original sent the encoded
//! samples to a browser through WebRTC; here they are decoded with openh264 and handed to the UI as RGBA frames.

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use super::frame::build_region_frame_message;
use super::types::{RdpBitmapFrame, RdpEncodedVideoCodec, RdpEncodedVideoFrame};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

pub(super) struct H264Decoder {
    decoder: Option<Decoder>,
}

impl H264Decoder {
    pub(super) fn new() -> Self {
        Self { decoder: None }
    }

    /// Decode one sample; `None` while the decoder is still waiting for a keyframe / parameter sets.
    pub(super) fn decode(&mut self, frame: &RdpEncodedVideoFrame) -> Result<Option<RdpBitmapFrame>, String> {
        let annex_b = match frame.codec {
            RdpEncodedVideoCodec::H264AnnexB => frame.bytes.clone(),
            RdpEncodedVideoCodec::H264AvcLengthPrefixed => avc_to_annex_b(&frame.bytes)?,
        };
        if self.decoder.is_none() {
            self.decoder = Some(Decoder::new().map_err(|e| format!("初始化 H.264 解码器失败: {e}"))?);
        }
        let decoder = self.decoder.as_mut().expect("just created");
        let Some(yuv) = decoder.decode(&annex_b).map_err(|e| format!("H.264 解码失败: {e}"))? else {
            return Ok(None);
        };
        let (w, h) = yuv.dimensions();
        let (width, height) = (u16::try_from(w).map_err(|_| "H.264 画面过宽")?, u16::try_from(h).map_err(|_| "H.264 画面过高")?);
        let mut rgba = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some(RdpBitmapFrame {
            session_id: frame.session_id.clone(),
            surface_id: 0,
            width,
            height,
            x: 0,
            y: 0,
            region_width: width,
            region_height: height,
            rgba,
        }))
    }
}

/// What changed between the last frame handed to the UI and a new decoded frame: only that rectangle is sent, so a
/// mostly static desktop no longer pushes (and the UI no longer copies) a full RGBA frame 30-60 times a second.
#[derive(Default)]
pub(super) struct DiffState {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

impl DiffState {
    /// The changed region of `frame` (the whole frame when the size changed or nothing was sent yet), or `None`
    /// when it is identical to the last one. The state then holds `frame`.
    pub(super) fn diff(&mut self, frame: RdpBitmapFrame) -> Option<RdpBitmapFrame> {
        const BPP: usize = 4;
        let (w, h) = (usize::from(frame.width), usize::from(frame.height));
        if self.width != frame.width || self.height != frame.height || self.rgba.len() != frame.rgba.len() || frame.rgba.len() != w * h * BPP {
            self.width = frame.width;
            self.height = frame.height;
            self.rgba = frame.rgba.clone();
            return Some(frame);
        }
        let stride = w * BPP;
        let row = |_: &[u8], y: usize| y * stride..(y + 1) * stride;
        let top = (0..h).find(|&y| self.rgba[row(&self.rgba, y)] != frame.rgba[row(&frame.rgba, y)])?;
        let bottom = (top..h).rev().find(|&y| self.rgba[row(&self.rgba, y)] != frame.rgba[row(&frame.rgba, y)]).unwrap_or(top);
        let (mut left, mut right) = (w, 0usize);
        for y in top..=bottom {
            let (old, new) = (&self.rgba[row(&self.rgba, y)], &frame.rgba[row(&frame.rgba, y)]);
            if old == new {
                continue;
            }
            let first = (0..w).find(|&x| old[x * BPP..(x + 1) * BPP] != new[x * BPP..(x + 1) * BPP]).unwrap_or(0);
            let last = (first..w).rev().find(|&x| old[x * BPP..(x + 1) * BPP] != new[x * BPP..(x + 1) * BPP]).unwrap_or(first);
            left = left.min(first);
            right = right.max(last);
        }
        let (rw, rh) = (right - left + 1, bottom - top + 1);
        let mut region = Vec::with_capacity(rw * rh * BPP);
        for y in top..=bottom {
            let range = y * stride + left * BPP..y * stride + (right + 1) * BPP;
            region.extend_from_slice(&frame.rgba[range.clone()]);
            self.rgba[range.clone()].copy_from_slice(&frame.rgba[range]);
        }
        Some(RdpBitmapFrame { session_id: frame.session_id, surface_id: frame.surface_id, width: frame.width, height: frame.height, x: left as u16, y: top as u16, region_width: rw as u16, region_height: rh as u16, rgba: region })
    }
}

#[derive(Default)]
pub(super) struct H264Counters {
    /// encoded samples handed to the decoder
    pub submitted: AtomicU64,
    /// regions sent to the UI and their bytes
    pub sent: AtomicU64,
    pub sent_bytes: AtomicU64,
}

/// Decoding happens on its own thread so a slow frame never delays reading the network or sending input. Samples are
/// decoded in order; when several are waiting only the newest picture is converted and sent.
pub(super) struct H264Pipeline {
    tx: mpsc::Sender<RdpEncodedVideoFrame>,
    pub counters: Arc<H264Counters>,
    error: Arc<Mutex<Option<String>>>,
}

impl H264Pipeline {
    pub(super) fn spawn(frames: super::FrameChannel) -> Self {
        let (tx, rx) = mpsc::channel::<RdpEncodedVideoFrame>();
        let counters = Arc::new(H264Counters::default());
        let error = Arc::new(Mutex::new(None));
        let (c, e) = (counters.clone(), error.clone());
        std::thread::Builder::new()
            .name("rdp-h264".into())
            .spawn(move || {
                let mut decoder = H264Decoder::new();
                let mut diff = DiffState::default();
                while let Ok(first) = rx.recv() {
                    let mut batch = vec![first];
                    while let Ok(next) = rx.try_recv() {
                        batch.push(next);
                    }
                    let mut latest = None;
                    for sample in batch {
                        match decoder.decode(&sample) {
                            Ok(Some(picture)) => latest = Some(picture),
                            Ok(None) => {}
                            Err(error) => {
                                *e.lock().unwrap() = Some(error);
                                return;
                            }
                        }
                    }
                    let Some(region) = latest.and_then(|picture| diff.diff(picture)) else { continue };
                    match build_region_frame_message(&region) {
                        Ok(message) => {
                            c.sent.fetch_add(1, Ordering::Relaxed);
                            c.sent_bytes.fetch_add(message.len() as u64, Ordering::Relaxed);
                            if frames.send(message).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            *e.lock().unwrap() = Some(error);
                            return;
                        }
                    }
                }
            })
            .expect("spawn the H.264 decoder thread");
        Self { tx, counters, error }
    }

    pub(super) fn submit(&self, frame: RdpEncodedVideoFrame) -> Result<(), String> {
        self.counters.submitted.fetch_add(1, Ordering::Relaxed);
        self.tx.send(frame).map_err(|_| self.take_error().unwrap_or_else(|| "H.264 解码线程已退出".to_string()))
    }

    pub(super) fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }
}

/// Replace 4-byte big-endian NAL length prefixes by Annex-B start codes.
pub(super) fn avc_to_annex_b(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(data.len() + 8);
    let mut pos = 0;
    while pos < data.len() {
        let header = data.get(pos..pos + 4).ok_or("AVC 样本长度前缀被截断")?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        pos += 4;
        let nal = data.get(pos..pos + len).ok_or("AVC NAL 长度超出样本")?;
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
        pos += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u16, h: u16, f: impl Fn(usize, usize) -> [u8; 4]) -> RdpBitmapFrame {
        let mut rgba = Vec::new();
        for y in 0..usize::from(h) {
            for x in 0..usize::from(w) {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        RdpBitmapFrame { session_id: "s".into(), surface_id: 0, width: w, height: h, x: 0, y: 0, region_width: w, region_height: h, rgba }
    }

    #[test]
    fn only_the_changed_rectangle_is_sent() {
        let mut diff = DiffState::default();
        let base = |x: usize, y: usize| [x as u8, y as u8, 7, 255];
        let first = diff.diff(picture(16, 12, base)).expect("the first picture is sent whole");
        assert_eq!((first.x, first.y, first.region_width, first.region_height), (0, 0, 16, 12));
        assert!(diff.diff(picture(16, 12, base)).is_none(), "an identical picture sends nothing");
        // two pixels differ: (3, 2) and (9, 6)
        let changed = |x: usize, y: usize| if (x, y) == (3, 2) || (x, y) == (9, 6) { [1, 2, 3, 255] } else { base(x, y) };
        let region = diff.diff(picture(16, 12, changed)).expect("changed");
        assert_eq!((region.x, region.y, region.region_width, region.region_height), (3, 2, 7, 5));
        assert_eq!(region.rgba.len(), 7 * 5 * 4);
        assert_eq!(&region.rgba[0..4], &[1, 2, 3, 255], "top-left of the region is the first changed pixel");
        assert_eq!(&region.rgba[(4 * 7 + 6) * 4..(4 * 7 + 6) * 4 + 4], &[1, 2, 3, 255], "bottom-right is the second");
        // the state followed: the same picture again is now identical
        assert!(diff.diff(picture(16, 12, changed)).is_none());
        // a different size is sent whole again
        let resized = diff.diff(picture(8, 8, base)).unwrap();
        assert_eq!((resized.region_width, resized.region_height), (8, 8));
    }

    #[test]
    fn length_prefixes_become_start_codes() {
        let data = [0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 1, 0x68];
        assert_eq!(avc_to_annex_b(&data).unwrap(), vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68]);
        assert!(avc_to_annex_b(&[0, 0, 0, 9, 1]).is_err());
        assert!(avc_to_annex_b(&[0, 0]).is_err());
        assert!(avc_to_annex_b(&[]).unwrap().is_empty());
    }
}
