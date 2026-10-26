//! OpsBatch RDP client (port of `commands/rdp/*`): ironrdp session, frames as RGBA regions, input, clipboard,
//! audio and file upload. The browser-specific WebRTC delivery of the original is replaced by a local H.264
//! decode (see `h264.rs`); frames reach the UI through a flume channel in the original 16-byte-header format.

#![allow(unexpected_cfgs)]

mod audio;
mod clipboard;
mod clipboard_files;
mod config;
mod dynamic_channels;
mod egfx;
mod file_transfer;
pub mod frame;
mod h264;
mod input;
pub mod keys;
mod protocol;
mod rdpevor;
#[cfg(test)]
mod tests;
mod types;

use dashmap::DashMap;
use rusqlite::params;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};

use ops_core::db::Database;

pub use types::{RdpConnectRequest, RdpConnectResponse, RdpInputEvent, RdpMetricsPayload, RdpStatusDetail, RdpStatusPayload, RdpTransportMode};
use types::{RdpConnectionOptions, RdpCredentials};

/// Frames are sent as `[width u16][height u16][x u16][y u16][region_w u16][region_h u16][len u32][RGBA…]` (LE).
pub type FrameChannel = flume::Sender<Vec<u8>>;

const DEFAULT_RDP_PORT: u16 = 3389;
const DEFAULT_DESKTOP_WIDTH: u16 = 1280;
const DEFAULT_DESKTOP_HEIGHT: u16 = 720;
const MIN_DESKTOP_WIDTH: u16 = 640;
const MIN_DESKTOP_HEIGHT: u16 = 480;
const MAX_DESKTOP_WIDTH: u16 = 3840;
const MAX_DESKTOP_HEIGHT: u16 = 2160;
pub(crate) const RDP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StoredRdpSettings {
    domain: Option<String>,
    desktop_width: Option<u16>,
    desktop_height: Option<u16>,
    enable_clipboard: Option<bool>,
    enable_audio: Option<bool>,
    map_disk: Option<bool>,
    disk_path: Option<String>,
}

struct HostRdpFields {
    host: String,
    port: i32,
    auth_type: String,
    username: String,
    password: Option<String>,
    os: String,
    settings: StoredRdpSettings,
    proxy: Option<ops_ssh::ssh::ProxySettings>,
}

pub(crate) enum RdpSessionCommand {
    Input(RdpInputEvent),
    Disconnect,
    UploadFiles(Vec<String>, Option<(u16, u16)>),
}

struct RdpSessionHandle {
    sender: mpsc::UnboundedSender<RdpSessionCommand>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RdpSessionHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub struct RdpManager {
    sessions: DashMap<String, RdpSessionHandle>,
}

impl RdpManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connect to the Windows host `request.host_id` using its stored credentials and RDP settings.
    pub async fn connect_host(&self, core: &ops_core::Core, request: RdpConnectRequest, frame_channel: FrameChannel) -> Result<RdpConnectResponse, String> {
        let db = core.db.clone();
        let host_id = request.host_id.clone();
        let host_fields = ops_core::rt::spawn_blocking(move || load_host_rdp_fields(&db, &host_id)).await.map_err(|e| e.to_string())??;
        if !host_fields.os.trim().eq_ignore_ascii_case("windows") {
            return Err("RDP 连接仅支持 Windows 主机".to_string());
        }
        if host_fields.auth_type != "password" {
            return Err("RDP 当前仅支持密码认证".to_string());
        }
        let password = ops_ssh::hosts::resolve_host_password(&request.host_id, host_fields.password)?;
        let credentials = normalize_credentials(&host_fields.username, password)?;
        let options = normalize_rdp_options(&request, &host_fields.host, Some(host_fields.port), &host_fields.settings, host_fields.proxy)?;
        self.connect(core.clone(), options, credentials, frame_channel).await
    }

    async fn connect(&self, app: ops_core::Core, options: RdpConnectionOptions, credentials: RdpCredentials, frame_channel: FrameChannel) -> Result<RdpConnectResponse, String> {
        if let Some((_, existing)) = self.sessions.remove(&options.session_id) {
            let _ = existing.sender.send(RdpSessionCommand::Disconnect);
            existing.task.abort();
        }

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let session_id = options.session_id.clone();
        let task = ops_core::rt::spawn(protocol::run_rdp_session(app, options, credentials, command_tx.clone(), command_rx, ready_tx, frame_channel));

        let ready = match tokio::time::timeout(RDP_CONNECT_TIMEOUT, ready_rx).await {
            Ok(Ok(Ok(ready))) => ready,
            Ok(Ok(Err(error))) => {
                task.abort();
                return Err(error);
            }
            Ok(Err(_)) => {
                task.abort();
                return Err("RDP 连接任务在握手完成前退出".to_string());
            }
            Err(_) => {
                task.abort();
                return Err("RDP 连接超时".to_string());
            }
        };

        self.sessions.insert(session_id, RdpSessionHandle { sender: command_tx, task });
        Ok(ready)
    }

    pub fn send_input(&self, core: &ops_core::Core, session_id: &str, event: RdpInputEvent) -> Result<(), String> {
        let trace = protocol::input_trace_enabled();
        let summary = input::input_event_summary(&event);
        let result = input::validate_input_event(&event).and_then(|()| {
            self.sessions
                .get(session_id)
                .map(|entry| entry.sender.clone())
                .ok_or_else(|| format!("RDP 会话不存在: {session_id}"))?
                .send(RdpSessionCommand::Input(event))
                .map_err(|_| format!("RDP 会话已断开: {session_id}"))
        });
        match &result {
            Ok(()) if trace => ops_core::log::emit_log(core, "info", "rdp.input", &format!("stage=queued sessionId={session_id} event={summary}"), "backend"),
            Ok(()) => {}
            Err(error) => ops_core::log::emit_log(core, "warn", "rdp.input", &format!("stage=queue_failed sessionId={session_id} event={summary} error={error}"), "backend"),
        }
        result
    }

    pub fn disconnect(&self, session_id: &str) -> Result<(), String> {
        if let Some((_, handle)) = self.sessions.remove(session_id) {
            let _ = handle.sender.send(RdpSessionCommand::Disconnect);
            handle.task.abort();
            Ok(())
        } else {
            Err(format!("RDP 会话不存在: {session_id}"))
        }
    }

    pub fn upload_files(&self, session_id: &str, paths: Vec<String>, position: Option<(u16, u16)>) -> Result<(), String> {
        self.sessions
            .get(session_id)
            .map(|entry| entry.sender.clone())
            .ok_or_else(|| format!("RDP 会话不存在: {session_id}"))?
            .send(RdpSessionCommand::UploadFiles(paths, position))
            .map_err(|_| format!("RDP 会话已断开: {session_id}"))
    }

    /// Test seam: a session without a server whose input events are handed back on a channel.
    #[doc(hidden)]
    pub fn register_test_session(&self, session_id: &str) -> flume::Receiver<RdpInputEvent> {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = flume::unbounded();
        let task = ops_core::rt::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                if let RdpSessionCommand::Input(event) = command {
                    let _ = event_tx.send(event);
                }
            }
        });
        self.sessions.insert(session_id.to_string(), RdpSessionHandle { sender: command_tx, task });
        event_rx
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }
}

fn load_host_rdp_fields(db: &Database, host_id: &str) -> Result<HostRdpFields, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (host, port, auth_type, username, password, os, settings_json, proxy_json): (String, i32, String, String, Option<String>, String, String, Option<String>) = conn
        .query_row(
            "SELECT ip, port, auth_type, username, password, os, COALESCE(rdp_settings, '{}'), COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            params![host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "{}".to_string()),
                    row.get(7)?,
                ))
            },
        )
        .map_err(|e| format!("host not found: {e}"))?;
    let settings = serde_json::from_str(&settings_json).unwrap_or_default();
    Ok(HostRdpFields { host, port, auth_type, username, password, os, settings, proxy: ops_ssh::hosts::resolve_host_proxy_settings(host_id, proxy_json)? })
}

fn normalize_rdp_options(
    request: &RdpConnectRequest,
    host: &str,
    stored_port: Option<i32>,
    settings: &StoredRdpSettings,
    proxy: Option<ops_ssh::ssh::ProxySettings>,
) -> Result<RdpConnectionOptions, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("RDP 主机地址不能为空".to_string());
    }
    let port = match stored_port {
        Some(value) if value > 0 => u16::try_from(value).map_err(|_| format!("RDP 端口超出范围: {value}"))?,
        _ => DEFAULT_RDP_PORT,
    };

    let width = settings.desktop_width.or(request.width).unwrap_or(DEFAULT_DESKTOP_WIDTH).clamp(MIN_DESKTOP_WIDTH, MAX_DESKTOP_WIDTH);
    let height = settings.desktop_height.or(request.height).unwrap_or(DEFAULT_DESKTOP_HEIGHT).clamp(MIN_DESKTOP_HEIGHT, MAX_DESKTOP_HEIGHT);
    let domain = settings.domain.clone().or_else(|| request.domain.clone()).map(|value| value.trim().to_string()).filter(|value| !value.is_empty());

    let transport_mode = request.transport_mode.unwrap_or_default();
    let default_enable_audio = transport_mode == RdpTransportMode::H264Direct;

    Ok(RdpConnectionOptions {
        host_id: request.host_id.clone(),
        session_id: request.session_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        host: host.to_string(),
        port,
        width,
        height,
        domain,
        enable_clipboard: settings.enable_clipboard.unwrap_or(true),
        enable_audio: settings.enable_audio.unwrap_or(default_enable_audio),
        transport_mode,
        proxy,
    })
}

fn normalize_credentials(username: &str, password: Option<String>) -> Result<RdpCredentials, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("RDP 用户名不能为空".to_string());
    }
    let password = password.filter(|value| !value.is_empty()).ok_or_else(|| "RDP 密码不能为空".to_string())?;
    Ok(RdpCredentials { username: username.to_string(), password })
}
