use std::time::Duration;

use tokio::time::Instant;

#[cfg(test)]
use super::types::RdpFramePayload;
use super::types::{RdpBitmapFrame, RectRegion};

pub(super) const FRAME_HEADER_LEN: usize = 16;
const BYTES_PER_PIXEL: usize = 4;
const MAX_PENDING_REGIONS: usize = 24;
const MERGE_AREA_MULTIPLIER: u64 = 2;

#[cfg(test)]
pub(super) fn build_frame_payload(
    width: u16,
    height: u16,
    image: &[u8],
    region: RectRegion,
) -> Result<RdpFramePayload, String> {
    validate_frame_region(width, height, image, region)?;

    let mut rgba = Vec::with_capacity(
        usize::from(region.width) * usize::from(region.height) * BYTES_PER_PIXEL,
    );
    let stride = usize::from(width) * BYTES_PER_PIXEL;
    let row_len = usize::from(region.width) * BYTES_PER_PIXEL;
    let bottom = region.y + region.height;
    for row in region.y..bottom {
        let start = usize::from(row) * stride + usize::from(region.x) * BYTES_PER_PIXEL;
        rgba.extend_from_slice(&image[start..start + row_len]);
    }

    Ok(RdpFramePayload {
        width,
        height,
        x: region.x,
        y: region.y,
        region_width: region.width,
        region_height: region.height,
        rgba,
    })
}

pub(super) fn build_frame_message(
    width: u16,
    height: u16,
    image: &[u8],
    region: RectRegion,
) -> Result<Vec<u8>, String> {
    validate_frame_region(width, height, image, region)?;

    let rgba_len =
        u32::try_from(usize::from(region.width) * usize::from(region.height) * BYTES_PER_PIXEL)
            .map_err(|_| "RDP framebuffer 更新区域过大".to_string())?;
    let mut message = Vec::with_capacity(FRAME_HEADER_LEN + rgba_len as usize);

    message.extend_from_slice(&width.to_le_bytes());
    message.extend_from_slice(&height.to_le_bytes());
    message.extend_from_slice(&region.x.to_le_bytes());
    message.extend_from_slice(&region.y.to_le_bytes());
    message.extend_from_slice(&region.width.to_le_bytes());
    message.extend_from_slice(&region.height.to_le_bytes());
    message.extend_from_slice(&rgba_len.to_le_bytes());

    let stride = usize::from(width) * BYTES_PER_PIXEL;
    let row_len = usize::from(region.width) * BYTES_PER_PIXEL;
    let bottom = region.y + region.height;
    for row in region.y..bottom {
        let start = usize::from(row) * stride + usize::from(region.x) * BYTES_PER_PIXEL;
        message.extend_from_slice(&image[start..start + row_len]);
    }

    Ok(message)
}

pub(super) fn build_region_frame_message(frame: &RdpBitmapFrame) -> Result<Vec<u8>, String> {
    validate_region_frame(frame)?;

    let rgba_len =
        u32::try_from(frame.rgba.len()).map_err(|_| "RDP framebuffer 更新区域过大".to_string())?;
    let mut message = Vec::with_capacity(FRAME_HEADER_LEN + frame.rgba.len());

    message.extend_from_slice(&frame.width.to_le_bytes());
    message.extend_from_slice(&frame.height.to_le_bytes());
    message.extend_from_slice(&frame.x.to_le_bytes());
    message.extend_from_slice(&frame.y.to_le_bytes());
    message.extend_from_slice(&frame.region_width.to_le_bytes());
    message.extend_from_slice(&frame.region_height.to_le_bytes());
    message.extend_from_slice(&rgba_len.to_le_bytes());
    message.extend_from_slice(&frame.rgba);

    Ok(message)
}

fn validate_region_frame(frame: &RdpBitmapFrame) -> Result<(), String> {
    if frame.region_width == 0 || frame.region_height == 0 {
        return Err("RDP framebuffer 更新区域为空".to_string());
    }
    let expected_len =
        usize::from(frame.region_width) * usize::from(frame.region_height) * BYTES_PER_PIXEL;
    if frame.rgba.len() != expected_len {
        return Err(format!(
            "RDP framebuffer 区域大小不匹配: expected={}, actual={}",
            expected_len,
            frame.rgba.len()
        ));
    }
    let right = frame
        .x
        .checked_add(frame.region_width)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    let bottom = frame
        .y
        .checked_add(frame.region_height)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    if right > frame.width || bottom > frame.height {
        return Err("RDP framebuffer 更新区域超出画面尺寸".to_string());
    }

    Ok(())
}

fn validate_frame_region(
    width: u16,
    height: u16,
    image: &[u8],
    region: RectRegion,
) -> Result<(), String> {
    let expected_len = usize::from(width) * usize::from(height) * BYTES_PER_PIXEL;
    if image.len() != expected_len {
        return Err(format!(
            "RDP framebuffer 大小不匹配: expected={}, actual={}",
            expected_len,
            image.len()
        ));
    }
    if region.width == 0 || region.height == 0 {
        return Err("RDP framebuffer 更新区域为空".to_string());
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    if right > width || bottom > height {
        return Err("RDP framebuffer 更新区域超出画面尺寸".to_string());
    }

    Ok(())
}

pub(super) struct FramePacer {
    frame_interval: Duration,
    last_sent_at: Option<Instant>,
    pending_regions: Vec<RectRegion>,
    pending_update_count: u32,
    next_deadline: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueuedFrame {
    pub regions: Vec<RectRegion>,
    pub coalesced_updates: u32,
}

impl FramePacer {
    pub(super) fn new(frame_interval: Duration) -> Self {
        Self {
            frame_interval,
            last_sent_at: None,
            pending_regions: Vec::new(),
            pending_update_count: 0,
            next_deadline: None,
        }
    }

    pub(super) fn queue(&mut self, region: RectRegion, now: Instant) -> Option<QueuedFrame> {
        let Some(last_sent_at) = self.last_sent_at else {
            self.last_sent_at = Some(now);
            return Some(QueuedFrame {
                regions: vec![region],
                coalesced_updates: 0,
            });
        };

        let deadline = last_sent_at + self.frame_interval;
        if now >= deadline {
            let coalesced_updates = self.pending_update_count;
            let mut regions = std::mem::take(&mut self.pending_regions);
            push_dirty_region(&mut regions, region);
            self.pending_update_count = 0;
            self.last_sent_at = Some(now);
            self.next_deadline = None;
            return Some(QueuedFrame {
                regions,
                coalesced_updates,
            });
        }

        push_dirty_region(&mut self.pending_regions, region);
        self.pending_update_count = self.pending_update_count.saturating_add(1);
        self.next_deadline = Some(deadline);
        None
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.next_deadline
    }

    pub(super) fn flush_due(&mut self, now: Instant) -> Option<QueuedFrame> {
        let deadline = self.next_deadline?;
        if now < deadline {
            return None;
        }

        self.flush_pending(now)
    }

    pub(super) fn flush_pending(&mut self, now: Instant) -> Option<QueuedFrame> {
        if self.pending_regions.is_empty() {
            return None;
        }

        let regions = std::mem::take(&mut self.pending_regions);
        let coalesced_updates = self.pending_update_count;
        self.pending_update_count = 0;
        self.last_sent_at = Some(now);
        self.next_deadline = None;
        Some(QueuedFrame {
            regions,
            coalesced_updates,
        })
    }
}

fn push_dirty_region(regions: &mut Vec<RectRegion>, region: RectRegion) {
    if regions.is_empty() {
        regions.push(region);
        return;
    }

    for existing in regions.iter_mut() {
        if should_merge_regions(*existing, region) {
            *existing = existing.union(region);
            compact_regions(regions);
            return;
        }
    }

    if regions.len() < MAX_PENDING_REGIONS {
        regions.push(region);
        return;
    }

    merge_into_lowest_cost_region(regions, region);
    compact_regions(regions);
}

fn compact_regions(regions: &mut Vec<RectRegion>) {
    let mut i = 0;
    while i < regions.len() {
        let mut j = i + 1;
        while j < regions.len() {
            if should_merge_regions(regions[i], regions[j]) {
                regions[i] = regions[i].union(regions[j]);
                regions.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

fn merge_into_lowest_cost_region(regions: &mut [RectRegion], region: RectRegion) {
    let Some((best_index, _)) = regions
        .iter()
        .enumerate()
        .map(|(index, existing)| (index, merge_cost(*existing, region)))
        .min_by_key(|(_, cost)| *cost)
    else {
        return;
    };

    regions[best_index] = regions[best_index].union(region);
}

fn should_merge_regions(a: RectRegion, b: RectRegion) -> bool {
    let combined_area = region_area(a).saturating_add(region_area(b));
    let union_area = region_area(a.union(b));
    union_area <= combined_area.saturating_mul(MERGE_AREA_MULTIPLIER)
}

fn merge_cost(a: RectRegion, b: RectRegion) -> u64 {
    region_area(a.union(b)).saturating_sub(region_area(a))
}

fn region_area(region: RectRegion) -> u64 {
    u64::from(region.width) * u64::from(region.height)
}

/// A decoded frame message: a region of the desktop plus the full desktop size. The pixels stay in the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame<'a> {
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
    pub region_width: u16,
    pub region_height: u16,
    pub rgba: &'a [u8],
}

/// Port of `decodeRdpFramePayload`.
pub fn decode_frame_payload(message: &[u8]) -> Result<DecodedFrame<'_>, String> {
    if message.len() < FRAME_HEADER_LEN {
        return Err("RDP frame header is incomplete".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([message[i], message[i + 1]]);
    let rgba_len =
        u32::from_le_bytes([message[12], message[13], message[14], message[15]]) as usize;
    let (region_width, region_height) = (u16_at(8), u16_at(10));
    if rgba_len != usize::from(region_width) * usize::from(region_height) * 4
        || message.len() != FRAME_HEADER_LEN + rgba_len
    {
        return Err("RDP frame payload size is invalid".into());
    }
    Ok(DecodedFrame {
        width: u16_at(0),
        height: u16_at(2),
        x: u16_at(4),
        y: u16_at(6),
        region_width,
        region_height,
        rgba: &message[FRAME_HEADER_LEN..],
    })
}

const BASE_FRAME_PAINT_BUDGET: usize = 8;
const MAX_FRAME_PAINT_BUDGET: usize = 24;
const LARGE_BACKLOG_FRAME_COUNT: usize = 24;

/// How many queued frames to paint in one UI frame (port of `getRdpFramePaintBudget`).
pub fn frame_paint_budget(queue_len: usize) -> usize {
    if queue_len == 0 {
        0
    } else if queue_len <= BASE_FRAME_PAINT_BUDGET {
        queue_len
    } else if queue_len <= LARGE_BACKLOG_FRAME_COUNT {
        BASE_FRAME_PAINT_BUDGET
    } else {
        MAX_FRAME_PAINT_BUDGET.min(queue_len.div_ceil(2))
    }
}

/// Take up to `budget` items from the front, preserving order (port of `drainRdpFrameBatch`).
pub fn drain_frame_batch<T>(queue: &mut std::collections::VecDeque<T>, budget: usize) -> Vec<T> {
    let n = budget.min(queue.len());
    queue.drain(..n).collect()
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn message(w: u16, h: u16, x: u16, y: u16, rw: u16, rh: u16, body: &[u8]) -> Vec<u8> {
        let mut m = Vec::new();
        for v in [w, h, x, y, rw, rh] {
            m.extend_from_slice(&v.to_le_bytes());
        }
        m.extend_from_slice(&(body.len() as u32).to_le_bytes());
        m.extend_from_slice(body);
        m
    }

    #[test]
    fn frames_round_trip_and_bad_sizes_are_rejected() {
        let body: Vec<u8> = (0..16).collect();
        let msg = message(100, 50, 3, 4, 2, 2, &body);
        let f = decode_frame_payload(&msg).unwrap();
        assert_eq!(
            (f.width, f.height, f.x, f.y, f.region_width, f.region_height),
            (100, 50, 3, 4, 2, 2)
        );
        assert_eq!(f.rgba, &body[..]);
        assert!(decode_frame_payload(&message(100, 50, 0, 0, 2, 2, &body[..12])).is_err());
        assert!(decode_frame_payload(&[0; 8]).is_err());
        let mut long = message(100, 50, 0, 0, 2, 2, &body);
        long.push(0);
        assert!(decode_frame_payload(&long).is_err());
    }

    #[test]
    fn paint_budget_and_batches() {
        assert_eq!(
            [0, 3, 8, 9, 24, 25, 40, 100].map(frame_paint_budget),
            [0, 3, 8, 8, 8, 13, 20, 24]
        );
        let mut q: std::collections::VecDeque<i32> = (1..=5).collect();
        assert_eq!(drain_frame_batch(&mut q, 2), vec![1, 2]);
        assert_eq!(q, [3, 4, 5]);
        assert_eq!(drain_frame_batch(&mut q, 0), Vec::<i32>::new());
        assert_eq!(drain_frame_batch(&mut q, 9), vec![3, 4, 5]);
    }
}
