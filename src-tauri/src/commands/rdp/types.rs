use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use serde::{Deserialize, Serialize};

use crate::ssh::ProxySettings;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpConnectRequest {
    pub host_id: String,
    pub session_id: Option<String>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub domain: Option<String>,
    pub transport_mode: Option<RdpTransportMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpConnectResponse {
    pub session_id: String,
    pub host_id: String,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub(super) struct RdpConnectionOptions {
    pub host_id: String,
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub width: u16,
    pub height: u16,
    pub domain: Option<String>,
    pub enable_clipboard: bool,
    pub enable_audio: bool,
    pub transport_mode: RdpTransportMode,
    pub proxy: Option<ProxySettings>,
}

#[derive(Debug, Clone)]
pub(super) struct RdpCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpStatusPayload {
    pub session_id: String,
    pub state: String,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<RdpStatusDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RdpStatusDetail {
    H264DirectUnavailable { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpMetricsPayload {
    pub session_id: String,
    pub server_updates_per_second: u32,
    pub sent_frames_per_second: u32,
    pub coalesced_updates_per_second: u32,
    pub sent_mbytes_per_second: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RdpTransportMode {
    LegacyBitmap,
    H264Direct,
}

impl Default for RdpTransportMode {
    fn default() -> Self {
        Self::LegacyBitmap
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(super) struct RdpFramePayload {
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
    pub region_width: u16,
    pub region_height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RdpEncodedVideoFrame {
    pub session_id: String,
    pub codec: RdpEncodedVideoCodec,
    pub frame_id: u32,
    pub timestamp_ms: u64,
    pub is_keyframe: bool,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RdpBitmapFrame {
    pub session_id: String,
    pub surface_id: u16,
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
    pub region_width: u16,
    pub region_height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RdpEncodedVideoCodec {
    /// AVC format carried by RDPGFX: each H.264 NAL unit has a 4-byte big-endian length prefix.
    H264AvcLengthPrefixed,
    /// Annex B H.264 sample carried by RDPEVOR TSMM_VIDEO_DATA.
    H264AnnexB,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RdpEncodedAudioFrame {
    pub session_id: String,
    pub codec: RdpEncodedAudioCodec,
    pub timestamp_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RdpEncodedAudioCodec {
    /// G.711 mu-law. Browser WebRTC stacks commonly negotiate this as audio/PCMU.
    Pcmu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RectRegion {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl RectRegion {
    #[cfg(test)]
    pub(super) fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(super) fn from_inclusive(rect: InclusiveRectangle) -> Self {
        Self {
            x: rect.left,
            y: rect.top,
            width: rect.width(),
            height: rect.height(),
        }
    }

    pub(super) fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = u32::from(self.x)
            .saturating_add(u32::from(self.width))
            .max(u32::from(other.x).saturating_add(u32::from(other.width)));
        let bottom = u32::from(self.y)
            .saturating_add(u32::from(self.height))
            .max(u32::from(other.y).saturating_add(u32::from(other.height)));

        Self {
            x: left,
            y: top,
            width: u16::try_from(right.saturating_sub(u32::from(left))).unwrap_or(u16::MAX),
            height: u16::try_from(bottom.saturating_sub(u32::from(top))).unwrap_or(u16::MAX),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RdpMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RdpInputEvent {
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        x: u16,
        y: u16,
        button: u8,
        down: bool,
    },
    Wheel {
        x: u16,
        y: u16,
        delta: i16,
        #[serde(default = "default_vertical_wheel")]
        vertical: bool,
    },
    KeyScancode {
        code: u8,
        #[serde(default)]
        extended: bool,
        down: bool,
    },
    Unicode {
        character: String,
        down: bool,
    },
}

fn default_vertical_wheel() -> bool {
    true
}
