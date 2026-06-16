use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use russh::{client, ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::db::Database;
use crate::ssh;

const FORWARD_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const FORWARD_BUFFER_SIZE: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardType {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardConfig {
    pub forward_type: ForwardType,
    pub local_addr: String,
    pub remote_addr: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardStatus {
    Active,
    Suspended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardEntry {
    pub id: String,
    pub host_id: String,
    pub config: ForwardConfig,
    pub status: ForwardStatus,
    pub error: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub connected_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Actor protocol
// ---------------------------------------------------------------------------

enum ForwardCmd {
    Stop,
    Shutdown,
}

struct ForwardSession {
    cmd_tx: mpsc::Sender<ForwardCmd>,
    task_handle: std::thread::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// ForwardManager
// ---------------------------------------------------------------------------

pub struct ForwardManager {
    sessions: DashMap<String, ForwardSession>,
    host_forwards: Mutex<HashMap<String, Vec<String>>>,
}

impl ForwardManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            host_forwards: Mutex::new(HashMap::new()),
        }
    }
}

impl Drop for ForwardManager {
    fn drop(&mut self) {
        let ids: Vec<String> = self.sessions.iter().map(|r| r.key().clone()).collect();
        for id in ids {
            if let Some((_, session)) = self.sessions.remove(&id) {
                let _ = session.cmd_tx.blocking_send(ForwardCmd::Shutdown);
                let _ = session.task_handle.join();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_host_config(db: &Database, host_id: &str) -> Result<ssh::SshConfig, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (ip, port, auth_type, username, password, private_key, proxy_settings): (
        String,
        i32,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT ip, port, auth_type, username, password, private_key, COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            rusqlite::params![host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(|e| format!("host not found: {}", e))?;
    drop(conn);
    let password = crate::commands::hosts::resolve_host_password(host_id, password)?;
    let private_key = crate::commands::hosts::resolve_host_private_key(host_id, private_key)?;
    Ok(ssh::SshConfig {
        host: ip,
        port: port as u16,
        username,
        auth_type,
        password,
        private_key,
        proxy: crate::commands::hosts::resolve_host_proxy_settings(host_id, proxy_settings)?,
    })
}

fn parse_addr(addr: &str) -> (String, u16) {
    if let Some(idx) = addr.rfind(':') {
        let host = &addr[..idx];
        let port: u16 = addr[idx + 1..].parse().unwrap_or(22);
        (host.to_string(), port)
    } else {
        (addr.to_string(), 22)
    }
}

fn emit_status(app: &AppHandle, host_id: &str, entry: ForwardEntry) {
    let _ = app.emit(&format!("forward-status:{}", host_id), entry);
}

// ---------------------------------------------------------------------------
// Bi-directional relay between TCP and SSH channel
// ---------------------------------------------------------------------------

async fn relay_pair(
    mut tcp_read: tokio::net::tcp::OwnedReadHalf,
    mut tcp_write: tokio::net::tcp::OwnedWriteHalf,
    mut ch_read: ChannelReadHalf,
    ch_write: ChannelWriteHalf<client::Msg>,
    app: AppHandle,
    host_id: String,
    forward_id: String,
) {
    let mut buf_up = vec![0u8; FORWARD_BUFFER_SIZE];
    let mut sent: u64 = 0;
    let mut received: u64 = 0;

    loop {
        let up = tokio::io::AsyncReadExt::read(&mut tcp_read, &mut buf_up);
        let down = ch_read.wait();

        tokio::select! {
            read_result = up => match read_result {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    sent += n as u64;
                    if ch_write.data_bytes(buf_up[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
            },
            msg = down => match msg {
                Some(ChannelMsg::Data { data }) => {
                    received += data.len() as u64;
                    if tokio::io::AsyncWriteExt::write_all(&mut tcp_write, &data).await.is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
        }
    }

    let _ = app.emit(
        &format!("forward-stats:{}", host_id),
        serde_json::json!({
            "id": forward_id, "bytesSent": sent, "bytesReceived": received,
        }),
    );
    let _ = ch_write.eof().await;
}

// ---------------------------------------------------------------------------
// Local forwarding (-L)
// ---------------------------------------------------------------------------

fn spawn_local_forward(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
) -> (mpsc::Sender<ForwardCmd>, std::thread::JoinHandle<()>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
    let conn_for_block = shared_conn.clone();
    let handle = std::thread::spawn(move || {
        conn_for_block.block_on(local_forward_loop(
            app,
            forward_id,
            host_id,
            config,
            shared_conn,
            &mut cmd_rx,
        ));
    });
    (cmd_tx, handle)
}

async fn local_forward_loop(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
    cmd_rx: &mut mpsc::Receiver<ForwardCmd>,
) {
    let local_addr: SocketAddr = match config.local_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            emit_status(
                &app,
                &host_id,
                err_entry(&forward_id, &host_id, &config, &format!("addr: {}", e)),
            );
            return;
        }
    };

    let listener = match tokio::net::TcpListener::bind(local_addr).await {
        Ok(l) => l,
        Err(e) => {
            emit_status(
                &app,
                &host_id,
                err_entry(&forward_id, &host_id, &config, &format!("bind: {}", e)),
            );
            return;
        }
    };

    let connected_at = chrono::Utc::now().timestamp();
    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id.clone(),
            host_id: host_id.clone(),
            config: config.clone(),
            status: ForwardStatus::Active,
            error: None,
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );

    let (remote_host, remote_port) = parse_addr(&config.remote_addr);
    let mut _last_activity = Instant::now();

    loop {
        tokio::select! {
            result = listener.accept() => match result {
                Ok((tcp, _)) => {
                    _last_activity = Instant::now();
                    let channel = match shared_conn.open_direct_tcpip_channel_async(&remote_host, remote_port as u32).await {
                        Ok(ch) => ch,
                        Err(_) => continue,
                    };
                    let (ch_read, ch_write) = channel.split();
                    let (tcp_read, tcp_write) = tcp.into_split();
                    tokio::spawn(relay_pair(
                        tcp_read, tcp_write, ch_read, ch_write,
                        app.clone(), host_id.clone(), forward_id.clone(),
                    ));
                }
                Err(_) => break,
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(ForwardCmd::Stop) | Some(ForwardCmd::Shutdown) | None => break,
            },
        }

        if _last_activity.elapsed() > FORWARD_IDLE_TIMEOUT {
            break;
        }
    }

    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id,
            host_id: host_id.clone(),
            config,
            status: ForwardStatus::Suspended,
            error: Some("stopped".into()),
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );
}

// ---------------------------------------------------------------------------
// Dynamic SOCKS5 forwarding (-D)
// ---------------------------------------------------------------------------

fn spawn_dynamic_forward(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
) -> (mpsc::Sender<ForwardCmd>, std::thread::JoinHandle<()>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
    let conn_for_block = shared_conn.clone();
    let handle = std::thread::spawn(move || {
        conn_for_block.block_on(dynamic_forward_loop(
            app,
            forward_id,
            host_id,
            config,
            shared_conn,
            &mut cmd_rx,
        ));
    });
    (cmd_tx, handle)
}

async fn dynamic_forward_loop(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
    cmd_rx: &mut mpsc::Receiver<ForwardCmd>,
) {
    let local_addr: SocketAddr = match config.local_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            emit_status(
                &app,
                &host_id,
                err_entry(&forward_id, &host_id, &config, &format!("addr: {}", e)),
            );
            return;
        }
    };

    let listener = match tokio::net::TcpListener::bind(local_addr).await {
        Ok(l) => l,
        Err(e) => {
            emit_status(
                &app,
                &host_id,
                err_entry(&forward_id, &host_id, &config, &format!("bind: {}", e)),
            );
            return;
        }
    };

    let connected_at = chrono::Utc::now().timestamp();
    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id.clone(),
            host_id: host_id.clone(),
            config: config.clone(),
            status: ForwardStatus::Active,
            error: None,
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );

    loop {
        tokio::select! {
            result = listener.accept() => match result {
                Ok((tcp, _)) => {
                    let conn = shared_conn.clone();
                    tokio::spawn(socks5_connect_and_relay(
                        tcp, conn, app.clone(), forward_id.clone(), host_id.clone(),
                    ));
                }
                Err(_) => break,
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(ForwardCmd::Stop) | Some(ForwardCmd::Shutdown) | None => break,
            },
        }
    }

    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id,
            host_id: host_id.clone(),
            config,
            status: ForwardStatus::Suspended,
            error: Some("stopped".into()),
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );
}

async fn socks5_connect_and_relay(
    tcp: tokio::net::TcpStream,
    shared_conn: ssh::SharedSshConnection,
    app: AppHandle,
    forward_id: String,
    host_id: String,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tcp;
    let mut buf = [0u8; 512];

    let _ = match stream.read(&mut buf).await {
        Ok(n) if n >= 2 => n,
        _ => return,
    };
    if buf[0] != 0x05 {
        return;
    }
    let nmethods = buf[1] as usize;
    if !(2..2 + nmethods.min(34)).any(|i| buf[i] == 0x00) {
        let _ = stream.write_all(&[0x05, 0xFF]).await;
        return;
    }
    if stream.write_all(&[0x05, 0x00]).await.is_err() {
        return;
    }

    let n = match stream.read(&mut buf).await {
        Ok(n) if n >= 7 => n,
        _ => return,
    };
    if buf[0] != 0x05 || buf[1] != 0x01 {
        return;
    }

    let (target_host, target_port) = match buf[3] {
        0x01 if n >= 10 => (
            format!("{}.{}.{}.{}", buf[4], buf[5], buf[6], buf[7]),
            ((buf[8] as u16) << 8) | (buf[9] as u16),
        ),
        0x03 => {
            let dlen = buf[4] as usize;
            if n < 5 + dlen + 2 {
                return;
            }
            let host = String::from_utf8_lossy(&buf[5..5 + dlen]).to_string();
            let port = ((buf[5 + dlen] as u16) << 8) | (buf[5 + dlen + 1] as u16);
            (host, port)
        }
        _ => return,
    };

    let channel = match shared_conn
        .open_direct_tcpip_channel_async(&target_host, target_port as u32)
        .await
    {
        Ok(ch) => ch,
        Err(_) => {
            let _ = stream
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return;
        }
    };
    if stream
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .is_err()
    {
        return;
    }

    let (ch_read, ch_write) = channel.split();
    let (tcp_read, tcp_write) = stream.into_split();
    relay_pair(
        tcp_read, tcp_write, ch_read, ch_write, app, host_id, forward_id,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Remote forwarding (-R)
// ---------------------------------------------------------------------------

fn spawn_remote_forward(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
) -> (mpsc::Sender<ForwardCmd>, std::thread::JoinHandle<()>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
    let conn_for_block = shared_conn.clone();
    let handle = std::thread::spawn(move || {
        conn_for_block.block_on(remote_forward_loop(
            app,
            forward_id,
            host_id,
            config,
            shared_conn,
            &mut cmd_rx,
        ));
    });
    (cmd_tx, handle)
}

async fn remote_forward_loop(
    app: AppHandle,
    forward_id: String,
    host_id: String,
    config: ForwardConfig,
    shared_conn: ssh::SharedSshConnection,
    cmd_rx: &mut mpsc::Receiver<ForwardCmd>,
) {
    let (remote_host, remote_port) = parse_addr(&config.remote_addr);

    let result = shared_conn
        .tcpip_forward_async(&remote_host, remote_port as u32)
        .await;

    let connected_at = chrono::Utc::now().timestamp();

    if let Err(e) = result {
        emit_status(
            &app,
            &host_id,
            err_entry(
                &forward_id,
                &host_id,
                &config,
                &format!("tcpip_forward: {}", e),
            ),
        );
        return;
    }

    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id.clone(),
            host_id: host_id.clone(),
            config: config.clone(),
            status: ForwardStatus::Active,
            error: None,
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            ForwardCmd::Stop | ForwardCmd::Shutdown => break,
        }
    }

    let _ = shared_conn
        .cancel_tcpip_forward_async(&remote_host, remote_port as u32)
        .await;

    emit_status(
        &app,
        &host_id,
        ForwardEntry {
            id: forward_id,
            host_id: host_id.clone(),
            config,
            status: ForwardStatus::Suspended,
            error: Some("stopped".into()),
            bytes_sent: 0,
            bytes_received: 0,
            connected_at: Some(connected_at),
        },
    );
}

// ---------------------------------------------------------------------------
// Helper to create error entry
// ---------------------------------------------------------------------------

fn err_entry(id: &str, host_id: &str, config: &ForwardConfig, error: &str) -> ForwardEntry {
    ForwardEntry {
        id: id.to_string(),
        host_id: host_id.to_string(),
        config: config.clone(),
        status: ForwardStatus::Error,
        error: Some(error.to_string()),
        bytes_sent: 0,
        bytes_received: 0,
        connected_at: None,
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn forward_list(
    manager: tauri::State<'_, ForwardManager>,
    host_id: String,
) -> Result<Vec<ForwardEntry>, String> {
    let host_forwards = manager.host_forwards.lock().map_err(|e| e.to_string())?;
    let ids = host_forwards.get(&host_id).cloned().unwrap_or_default();
    drop(host_forwards);

    let mut entries = Vec::new();
    for id in ids {
        if manager.sessions.contains_key(&id) {
            entries.push(ForwardEntry {
                id: id.clone(),
                host_id: host_id.clone(),
                config: ForwardConfig {
                    forward_type: ForwardType::Local,
                    local_addr: String::new(),
                    remote_addr: String::new(),
                    label: None,
                },
                status: ForwardStatus::Active,
                error: None,
                bytes_sent: 0,
                bytes_received: 0,
                connected_at: None,
            });
        }
    }
    Ok(entries)
}

#[tauri::command]
pub fn forward_add(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    pool: tauri::State<'_, ssh::SshConnectionRegistry>,
    manager: tauri::State<'_, ForwardManager>,
    host_id: String,
    config: ForwardConfig,
) -> Result<ForwardEntry, String> {
    let forward_id = uuid::Uuid::new_v4().to_string();
    let ssh_config = load_host_config(&db, &host_id)?;
    let shared_conn = pool.get_shared_connection(&host_id, &ssh_config, 10)?;

    let (cmd_tx, task_handle) = match config.forward_type {
        ForwardType::Local => spawn_local_forward(
            app,
            forward_id.clone(),
            host_id.clone(),
            config.clone(),
            shared_conn,
        ),
        ForwardType::Remote => spawn_remote_forward(
            app,
            forward_id.clone(),
            host_id.clone(),
            config.clone(),
            shared_conn,
        ),
        ForwardType::Dynamic => spawn_dynamic_forward(
            app,
            forward_id.clone(),
            host_id.clone(),
            config.clone(),
            shared_conn,
        ),
    };

    manager.sessions.insert(
        forward_id.clone(),
        ForwardSession {
            cmd_tx,
            task_handle,
        },
    );

    {
        let mut host_forwards = manager.host_forwards.lock().map_err(|e| e.to_string())?;
        host_forwards
            .entry(host_id.clone())
            .or_default()
            .push(forward_id.clone());
    }

    Ok(ForwardEntry {
        id: forward_id,
        host_id,
        config,
        status: ForwardStatus::Active,
        error: None,
        bytes_sent: 0,
        bytes_received: 0,
        connected_at: Some(chrono::Utc::now().timestamp()),
    })
}

#[tauri::command]
pub fn forward_remove(
    manager: tauri::State<'_, ForwardManager>,
    host_id: String,
    forward_id: String,
) -> Result<(), String> {
    if let Some((_, session)) = manager.sessions.remove(&forward_id) {
        let _ = session.cmd_tx.blocking_send(ForwardCmd::Shutdown);
        let _ = session.task_handle.join();
    }
    {
        let mut host_forwards = manager.host_forwards.lock().map_err(|e| e.to_string())?;
        if let Some(ids) = host_forwards.get_mut(&host_id) {
            ids.retain(|id| id != &forward_id);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn forward_stop(
    manager: tauri::State<'_, ForwardManager>,
    _host_id: String,
    forward_id: String,
) -> Result<(), String> {
    if let Some(session) = manager.sessions.get(&forward_id) {
        session
            .cmd_tx
            .blocking_send(ForwardCmd::Stop)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
