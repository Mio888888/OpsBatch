use rusqlite::params;

use super::types::{VncHostConfig, VncSessionOptions, VncSettings, DEFAULT_VNC_PORT};
use crate::db::Database;

pub fn parse_vnc_settings(value: &str) -> VncSettings {
    serde_json::from_str(value).unwrap_or(VncSettings {
        protocol: None,
        vnc_port: None,
        vnc_username: None,
        vnc_password: None,
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
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
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

    Ok(VncHostConfig {
        host,
        port: vnc_port_from_settings(&rdp_settings, port),
        username: settings.vnc_username.and_then(non_empty_trimmed),
        password: settings.vnc_password.and_then(non_empty_trimmed),
        options: VncSessionOptions {
            shared_session: settings.vnc_shared.unwrap_or(true),
            view_only: settings.vnc_view_only.unwrap_or(false),
        },
        proxy: crate::commands::hosts::parse_host_proxy_settings(proxy_settings),
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
