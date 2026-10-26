//! VNC host settings stored in `hosts.rdp_settings` (shared by ops-ssh host handling and ops-vnc).

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const DEFAULT_VNC_PORT: u16 = 5900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum VncAuthMethod {
    #[serde(rename = "vnc")]
    #[default]
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

pub fn parse_vnc_settings(value: &str) -> VncSettings {
    serde_json::from_str(value).unwrap_or(VncSettings {
        protocol: None,
        vnc_port: None,
        vnc_username: None,
        vnc_password: None,
        vnc_auth_method: VncAuthMethod::default(),
        vnc_shared: None,
        vnc_view_only: None,
    })
}

pub fn vnc_port_from_settings(value: &str, _fallback: i32) -> u16 {
    let settings = parse_vnc_settings(value);
    settings
        .vnc_port
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_VNC_PORT)
}

