use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub const DEFAULT_VNC_PORT: u16 = 5900;

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
    pub secret_owner_id: Option<String>,
    pub password: Option<String>,
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
            secret_owner_id: None,
            password: config.password,
            options: Some(config.options),
        }
    }

    pub(crate) fn secret_owner_id(&self) -> Option<&str> {
        self.secret_owner_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn password(&self) -> Option<&str> {
        self.password.as_deref().filter(|value| !value.is_empty())
    }

    pub(crate) fn set_password(&mut self, password: Option<String>) {
        self.password = password;
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
