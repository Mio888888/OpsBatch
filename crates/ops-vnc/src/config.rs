//! VNC host configuration (port of `commands/vnc/{types,config}.rs`): stored in the host's `rdp_settings` JSON.

use ops_core::db::Database;
use ops_core::keychain::SecretError;
use ops_core::security::SECRET_PLACEHOLDER;
use rusqlite::params;
use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_VNC_PORT: u16 = 5900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
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
            "ard" | "appleremotedesktop" | "apple_remote_desktop" => Self::AppleRemoteDesktop,
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
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        Ok(value.as_deref().map(Self::from_value).unwrap_or_default())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
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
    pub shared: bool,
    pub view_only: bool,
    pub proxy: Option<ops_ssh::ssh::ProxySettings>,
}

pub fn parse_vnc_settings(value: &str) -> VncSettings {
    serde_json::from_str(value).unwrap_or_default()
}

pub fn vnc_port_from_settings(value: &str) -> u16 {
    parse_vnc_settings(value).vnc_port.filter(|port| *port > 0).unwrap_or(DEFAULT_VNC_PORT)
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Load a host's VNC settings; the stored password may be the keychain placeholder.
pub fn load_vnc_host_config(db: &Database, host_id: &str) -> Result<VncHostConfig, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (host, settings_json, proxy_settings): (String, String, Option<String>) = conn
        .query_row(
            "SELECT ip, COALESCE(rdp_settings, '{}'), COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            params![host_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| format!("host not found: {e}"))?;

    let settings = parse_vnc_settings(&settings_json);
    if settings.protocol.as_deref() != Some("vnc") {
        return Err("主机未配置为 VNC 远程桌面".to_string());
    }

    let password = settings
        .vnc_password
        .clone()
        .and_then(non_empty_trimmed)
        .map(|value| {
            if value == SECRET_PLACEHOLDER {
                ops_core::keychain::get_host_vnc_password(host_id).map_err(|error| match error {
                    SecretError::Missing => format!("主机 {host_id} 的 VNC 密码未在本地加密存储中找到，请重新编辑主机并保存 VNC 密码。"),
                    other => other.to_string(),
                })
            } else {
                Ok(value)
            }
        })
        .transpose()?;

    Ok(VncHostConfig {
        host,
        port: vnc_port_from_settings(&settings_json),
        username: settings.vnc_username.and_then(non_empty_trimmed),
        password,
        auth_method: settings.vnc_auth_method,
        shared: settings.vnc_shared.unwrap_or(true),
        view_only: settings.vnc_view_only.unwrap_or(false),
        proxy: ops_ssh::hosts::resolve_host_proxy_settings(host_id, proxy_settings)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_auth_method_aliases() {
        let s = parse_vnc_settings(r#"{"protocol":"vnc","vncPort":5901,"vncAuthMethod":"ard","vncViewOnly":true}"#);
        assert_eq!((s.vnc_port, s.vnc_auth_method, s.vnc_view_only), (Some(5901), VncAuthMethod::AppleRemoteDesktop, Some(true)));
        assert_eq!(parse_vnc_settings("{}").vnc_auth_method, VncAuthMethod::VncAuth);
        assert_eq!(parse_vnc_settings("not json").vnc_port, None);
        assert_eq!(vnc_port_from_settings(r#"{"vncPort":0}"#), 5900);
        assert_eq!(vnc_port_from_settings("{}"), DEFAULT_VNC_PORT);
        assert_eq!(VncAuthMethod::from_value("Apple_Remote_Desktop"), VncAuthMethod::AppleRemoteDesktop);
        assert_eq!(VncAuthMethod::from_value("whatever").as_str(), "vnc");
    }
}
