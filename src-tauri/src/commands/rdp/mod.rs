mod config;
mod frame;
mod input;
mod protocol;
#[cfg(test)]
mod tests;
mod types;

use dashmap::DashMap;
use rusqlite::params;
use tauri::Manager;
use tokio::sync::{mpsc, oneshot};

use crate::db::Database;

pub use types::{RdpConnectRequest, RdpConnectResponse, RdpInputEvent};
use types::{RdpConnectionOptions, RdpCredentials};

const DEFAULT_RDP_PORT: u16 = 3389;
const DEFAULT_DESKTOP_WIDTH: u16 = 1280;
const DEFAULT_DESKTOP_HEIGHT: u16 = 720;
pub(super) const RDP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) enum RdpSessionCommand {
    Input(RdpInputEvent),
    Disconnect,
}

struct RdpSessionHandle {
    sender: mpsc::UnboundedSender<RdpSessionCommand>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for RdpSessionHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct RdpManager {
    sessions: DashMap<String, RdpSessionHandle>,
}

impl RdpManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    async fn connect(
        &self,
        app: tauri::AppHandle,
        options: RdpConnectionOptions,
        credentials: RdpCredentials,
    ) -> Result<RdpConnectResponse, String> {
        if let Some((_, existing)) = self.sessions.remove(&options.session_id) {
            let _ = existing.sender.send(RdpSessionCommand::Disconnect);
            existing.task.abort();
        }

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let session_id = options.session_id.clone();
        let task = tauri::async_runtime::spawn(protocol::run_rdp_session(
            app,
            options,
            credentials,
            command_rx,
            ready_tx,
        ));

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

        self.sessions.insert(
            session_id,
            RdpSessionHandle {
                sender: command_tx,
                task,
            },
        );

        Ok(ready)
    }

    async fn send_input(&self, session_id: &str, event: RdpInputEvent) -> Result<(), String> {
        input::validate_input_event(&event)?;
        self.sessions
            .get(session_id)
            .map(|entry| entry.sender.clone())
            .ok_or_else(|| format!("RDP 会话不存在: {}", session_id))?
            .send(RdpSessionCommand::Input(event))
            .map_err(|_| format!("RDP 会话已断开: {}", session_id))
    }

    async fn disconnect(&self, session_id: &str) -> Result<(), String> {
        if let Some((_, handle)) = self.sessions.remove(session_id) {
            let _ = handle.sender.send(RdpSessionCommand::Disconnect);
            handle.task.abort();
            Ok(())
        } else {
            Err(format!("RDP 会话不存在: {}", session_id))
        }
    }
}

#[tauri::command]
pub async fn rdp_connect(
    app: tauri::AppHandle,
    request: RdpConnectRequest,
) -> Result<RdpConnectResponse, String> {
    let (host, port, auth_type, username, password, os) =
        load_host_rdp_fields(&app, &request.host_id)?;
    if !os.trim().eq_ignore_ascii_case("windows") {
        return Err("RDP 连接仅支持 Windows 主机".to_string());
    }
    if auth_type != "password" {
        return Err("RDP 当前仅支持密码认证".to_string());
    }

    let password = crate::commands::hosts::resolve_host_password(&request.host_id, password)?;
    let credentials = normalize_credentials(&username, password)?;
    let options = normalize_rdp_options(&request, &host, Some(port))?;

    app.state::<RdpManager>()
        .connect(app.clone(), options, credentials)
        .await
}

#[tauri::command]
pub async fn rdp_send_input(
    manager: tauri::State<'_, RdpManager>,
    session_id: String,
    event: RdpInputEvent,
) -> Result<(), String> {
    manager.send_input(&session_id, event).await
}

#[tauri::command]
pub async fn rdp_disconnect(
    manager: tauri::State<'_, RdpManager>,
    session_id: String,
) -> Result<(), String> {
    manager.disconnect(&session_id).await
}

fn load_host_rdp_fields(
    app: &tauri::AppHandle,
    host_id: &str,
) -> Result<(String, i32, String, String, Option<String>, String), String> {
    let db = app.state::<Database>();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT ip, port, auth_type, username, password, os FROM hosts WHERE id=?1",
        params![host_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )
    .map_err(|e| format!("host not found: {}", e))
}

fn normalize_rdp_options(
    request: &RdpConnectRequest,
    host: &str,
    stored_port: Option<i32>,
) -> Result<RdpConnectionOptions, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("RDP 主机地址不能为空".to_string());
    }
    let port = match stored_port {
        Some(value) if value > 0 => {
            u16::try_from(value).map_err(|_| format!("RDP 端口超出范围: {}", value))?
        }
        _ => DEFAULT_RDP_PORT,
    };

    Ok(RdpConnectionOptions {
        host_id: request.host_id.clone(),
        session_id: request
            .session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        host: host.to_string(),
        port,
        width: request.width.unwrap_or(DEFAULT_DESKTOP_WIDTH).max(1),
        height: request.height.unwrap_or(DEFAULT_DESKTOP_HEIGHT).max(1),
        domain: request
            .domain
            .clone()
            .filter(|value| !value.trim().is_empty()),
    })
}

fn normalize_credentials(
    username: &str,
    password: Option<String>,
) -> Result<RdpCredentials, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("RDP 用户名不能为空".to_string());
    }
    let password = password
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "RDP 密码不能为空".to_string())?;

    Ok(RdpCredentials {
        username: username.to_string(),
        password,
    })
}
