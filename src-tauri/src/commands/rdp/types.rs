use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpConnectRequest {
    pub host_id: String,
    pub session_id: Option<String>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub domain: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpFramePayload {
    pub session_id: String,
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
    pub region_width: u16,
    pub region_height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
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
