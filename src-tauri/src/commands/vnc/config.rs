use rusqlite::params;

use super::types::{
    VncAuthMethod, VncHostConfig, VncSessionOptions, VncSettings, DEFAULT_VNC_PORT,
};
use crate::db::Database;
use crate::security::SECRET_PLACEHOLDER;

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

pub fn load_vnc_host_config(db: &Database, host_id: &str) -> Result<VncHostConfig, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (host, port, rdp_settings, proxy_settings): (String, i32, String, Option<String>) = conn
        .query_row(
            "SELECT ip, port, COALESCE(rdp_settings, '{}'), COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            params![host_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|e| format!("host not found: {}", e))?;

    let settings = parse_vnc_settings(&rdp_settings);
    if settings.protocol.as_deref() != Some("vnc") {
        return Err("主机未配置为 VNC 远程桌面".to_string());
    }

    let password = settings
        .vnc_password
        .and_then(non_empty_trimmed)
        .map(|value| {
            if value == SECRET_PLACEHOLDER {
                crate::keychain::get_host_vnc_password(host_id).map_err(|error| match error {
                    crate::keychain::SecretError::Missing => format!(
                        "主机 {} 的 VNC 密码未在本地加密存储中找到，请重新编辑主机并保存 VNC 密码。",
                        host_id
                    ),
                    other => other.to_string(),
                })
            } else {
                Ok(value)
            }
        })
        .transpose()?;

    Ok(VncHostConfig {
        host,
        port: vnc_port_from_settings(&rdp_settings, port),
        username: settings.vnc_username.and_then(non_empty_trimmed),
        password,
        auth_method: settings.vnc_auth_method,
        options: VncSessionOptions {
            shared_session: settings.vnc_shared.unwrap_or(true),
            view_only: settings.vnc_view_only.unwrap_or(false),
        },
        proxy: crate::commands::hosts::resolve_host_proxy_settings(host_id, proxy_settings)?,
    })
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
