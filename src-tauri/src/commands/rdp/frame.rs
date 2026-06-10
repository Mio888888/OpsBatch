use std::time::Duration;

use tokio::time::Instant;

use super::types::{RdpFramePayload, RectRegion};

pub(super) const FRAME_HEADER_LEN: usize = 16;

pub(super) fn build_frame_payload(
    width: u16,
    height: u16,
    image: &[u8],
    region: RectRegion,
) -> Result<RdpFramePayload, String> {
    let bytes_per_pixel = 4usize;
    let expected_len = usize::from(width) * usize::from(height) * bytes_per_pixel;
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

    let mut rgba = Vec::with_capacity(usize::from(region.width) * usize::from(region.height) * 4);
    let stride = usize::from(width) * bytes_per_pixel;
    let row_len = usize::from(region.width) * bytes_per_pixel;
    for row in region.y..bottom {
        let start = usize::from(row) * stride + usize::from(region.x) * bytes_per_pixel;
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
    let payload = build_frame_payload(width, height, image, region)?;
    let rgba_len = u32::try_from(payload.rgba.len())
        .map_err(|_| "RDP framebuffer 更新区域过大".to_string())?;
    let mut message = Vec::with_capacity(FRAME_HEADER_LEN + payload.rgba.len());

    message.extend_from_slice(&payload.width.to_le_bytes());
    message.extend_from_slice(&payload.height.to_le_bytes());
    message.extend_from_slice(&payload.x.to_le_bytes());
    message.extend_from_slice(&payload.y.to_le_bytes());
    message.extend_from_slice(&payload.region_width.to_le_bytes());
    message.extend_from_slice(&payload.region_height.to_le_bytes());
    message.extend_from_slice(&rgba_len.to_le_bytes());
    message.extend_from_slice(&payload.rgba);

    Ok(message)
}

pub(super) struct FramePacer {
    frame_interval: Duration,
    last_sent_at: Option<Instant>,
    pending_region: Option<RectRegion>,
    pending_update_count: u32,
    next_deadline: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QueuedFrame {
    pub region: RectRegion,
    pub coalesced_updates: u32,
}

impl FramePacer {
    pub(super) fn new(frame_interval: Duration) -> Self {
        Self {
            frame_interval,
            last_sent_at: None,
            pending_region: None,
            pending_update_count: 0,
            next_deadline: None,
        }
    }

    pub(super) fn queue(&mut self, region: RectRegion, now: Instant) -> Option<QueuedFrame> {
        let Some(last_sent_at) = self.last_sent_at else {
            self.last_sent_at = Some(now);
            return Some(QueuedFrame {
                region,
                coalesced_updates: 0,
            });
        };

        let deadline = last_sent_at + self.frame_interval;
        if now >= deadline {
            let coalesced_updates = self.pending_update_count;
            let region = self
                .pending_region
                .take()
                .map(|pending| pending.union(region))
                .unwrap_or(region);
            self.pending_update_count = 0;
            self.last_sent_at = Some(now);
            self.next_deadline = None;
            return Some(QueuedFrame {
                region,
                coalesced_updates,
            });
        }

        self.pending_region = Some(match self.pending_region {
            Some(pending) => pending.union(region),
            None => region,
        });
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
        let region = self.pending_region.take()?;
        let coalesced_updates = self.pending_update_count;
        self.pending_update_count = 0;
        self.last_sent_at = Some(now);
        self.next_deadline = None;
        Some(QueuedFrame {
            region,
            coalesced_updates,
        })
    }
}
