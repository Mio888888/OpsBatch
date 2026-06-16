use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use tokio::sync::oneshot;

pub const DEFAULT_VNC_PORT: u16 = 5900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VncAuthMethod {
    #[serde(rename = "vnc")]
    VncAuth,
    #[serde(rename = "ard")]
    AppleRemoteDesktop,
}

impl VncAuthMethod {
    pub fn from_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ard" | "appleRemoteDesktop" | "apple_remote_desktop" => Self::AppleRemoteDesktop,
            _ => Self::VncAuth,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::VncAuth => "vnc",
            Self::AppleRemoteDesktop => "ard",
        }
    }
}

impl Default for VncAuthMethod {
    fn default() -> Self {
        Self::VncAuth
    }
}

impl<'de> Deserialize<'de> for VncAuthMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        Ok(value.as_deref().map(Self::from_value).unwrap_or_default())
    }
}

impl fmt::Display for VncAuthMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub struct VncBridgeSession {
    pub stop: Option<oneshot::Sender<()>>,
    pub task: tokio::task::JoinHandle<()>,
    pub connected: bool,
    pub view_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VncSettings {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default, rename = "vncPort")]
    pub vnc_port: Option<u16>,
    #[serde(default, rename = "vncUsername")]
    pub vnc_username: Option<String>,
    #[serde(default, rename = "vncPassword")]
    pub vnc_password: Option<String>,
    #[serde(default, rename = "vncAuthMethod")]
    pub vnc_auth_method: VncAuthMethod,
    #[serde(default, rename = "vncShared")]
    pub vnc_shared: Option<bool>,
    #[serde(default, rename = "vncViewOnly")]
    pub vnc_view_only: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct VncHostConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_method: VncAuthMethod,
    pub options: VncSessionOptions,
    pub proxy: Option<crate::ssh::ProxySettings>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartVncSessionRequest {
    pub session_id: String,
    pub host: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub auth_method: VncAuthMethod,
    pub options: Option<VncSessionOptions>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncSessionOptions {
    #[serde(default = "default_true")]
    pub shared_session: bool,
    #[serde(default)]
    pub view_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncConnectResponse {
    pub session_id: String,
    pub host_id: String,
    pub websocket_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_method: VncAuthMethod,
    pub shared: bool,
    pub view_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncSessionStarted {
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub websocket_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_method: VncAuthMethod,
    pub shared: bool,
    pub view_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncSessionStatus {
    pub session_id: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum VncInputEvent {
    #[serde(rename = "mouse")]
    Mouse { x: u16, y: u16, buttons: u8 },
    #[serde(rename = "key")]
    Key { keycode: u32, down: bool },
    #[serde(rename = "refresh")]
    Refresh,
    #[serde(rename = "clipboard")]
    Clipboard { text: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncPointerEventRequest {
    pub session_id: String,
    pub x: u16,
    pub y: u16,
    pub button_mask: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncKeyEventRequest {
    pub session_id: String,
    pub key: u32,
    pub down: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncSimpleRequest {
    pub session_id: String,
}

impl StartVncSessionRequest {
    pub fn from_host_config(session_id: String, config: VncHostConfig) -> Self {
        Self {
            session_id,
            host: config.host,
            port: Some(config.port),
            username: config.username,
            password: config.password,
            auth_method: config.auth_method,
            options: Some(config.options),
        }
    }
}

impl Default for VncSessionOptions {
    fn default() -> Self {
        Self {
            shared_session: true,
            view_only: false,
        }
    }
}

pub fn default_true() -> bool {
    true
}
