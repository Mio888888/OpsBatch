use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use russh::client;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc as async_mpsc;

use crate::commands::terminal_shell::{current_shell_platform, select_local_shell};
use crate::db::Database;
use crate::ssh;

/// hosts 表查询的终端连接字段：
/// (ip, port, auth_type, username, password, private_key, jump_chain_json, proxy_settings)
type TerminalHostRow = (
    String,
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

const OUTPUT_BATCH_MAX_BYTES: usize = 128 * 1024;
const OUTPUT_BATCH_WINDOW: Duration = Duration::from_millis(16);
const READ_BUFFER_SIZE: usize = 16 * 1024;
const IDLE_SLEEP: Duration = Duration::from_millis(5);
const ACTOR_CHANNEL_CAPACITY: usize = 512;
const SSH_DISCONNECT_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(serde::Serialize)]
pub struct BackgroundProcessInfo {
    pid: u32,
    log_file: String,
}

// ---------------------------------------------------------------------------
// Actor command protocol
// ---------------------------------------------------------------------------

enum TerminalCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Disconnect,
}

// ---------------------------------------------------------------------------
// Per-session resources stored outside the DashMap (owned by actor threads)
// ---------------------------------------------------------------------------

/// Resources owned by the SSH actor thread.
struct SshActorResources {
    lease: ssh::SharedSshChannelLease,
    write_half: ChannelWriteHalf<client::Msg>,
}

/// Resources owned by the local PTY actor thread.
struct LocalActorResources {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

// ---------------------------------------------------------------------------
// Session handle: cheaply cloneable handle stored in DashMap
// ---------------------------------------------------------------------------

struct SessionHandle {
    sender: async_mpsc::Sender<TerminalCommand>,
    actor_thread: std::thread::JoinHandle<()>,
    /// For local sessions, the master PTY is needed for resize operations.
    /// For SSH sessions, resize goes through the actor.
    local_master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    /// For local sessions, the child process handle for kill/wait on disconnect.
    local_child: Option<Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>>,
}

// ---------------------------------------------------------------------------
// Terminal manager -- DashMap-backed concurrent session registry
// ---------------------------------------------------------------------------

pub struct TerminalManager {
    sessions: DashMap<String, SessionHandle>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        let session_ids: Vec<String> = self.sessions.iter().map(|r| r.key().clone()).collect();
        let mut actor_threads = Vec::new();
        let mut local_children = Vec::new();
        for id in session_ids {
            if let Some((_, handle)) = self.sessions.remove(&id) {
                let _ = handle.sender.try_send(TerminalCommand::Disconnect);
                let is_local_session = handle.local_child.is_some();
                if let Some(child) = handle.local_child {
                    local_children.push(child);
                }
                actor_threads.push((id, handle.actor_thread, is_local_session));
            }
        }
        // Kill and reap local child processes.
        for child in local_children {
            if let Ok(mut c) = child.lock() {
                if c.kill().is_ok() {
                    let _ = c.wait();
                } else {
                    let _ = c.try_wait();
                }
            }
        }
        for (id, thread, is_local_session) in actor_threads {
            if is_local_session {
                let _ = thread.join();
            } else {
                join_actor_thread_with_timeout(&id, thread, SSH_DISCONNECT_TIMEOUT);
            }
        }
    }
}

fn join_actor_thread_with_timeout(
    session_id: &str,
    thread: std::thread::JoinHandle<()>,
    timeout: Duration,
) {
    let (tx, rx) = mpsc::channel();
    let session_id = session_id.to_string();
    std::thread::spawn(move || {
        let _ = thread.join();
        let _ = tx.send(());
    });

    if rx.recv_timeout(timeout).is_err() {
        eprintln!(
            "[Terminal] Session {} actor did not exit within {:?}; cleanup continues in background",
            session_id, timeout
        );
    }
}

fn cleanup_terminal_session_in_background(session_id: String, handle: SessionHandle) {
    std::thread::spawn(move || {
        let is_local_session = handle.local_child.is_some();

        if let Some(child) = handle.local_child {
            match child.lock() {
                Ok(mut c) => {
                    if c.kill().is_ok() {
                        let _ = c.wait();
                    } else {
                        let _ = c.try_wait();
                    }
                }
                Err(e) => {
                    eprintln!("[Terminal] Failed to lock child for kill: {}", e);
                }
            }
        }

        if is_local_session {
            let _ = handle.actor_thread.join();
        } else {
            join_actor_thread_with_timeout(
                &session_id,
                handle.actor_thread,
                SSH_DISCONNECT_TIMEOUT,
            );
        }
    });
}

fn default_background_log_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("background-processes");
    fs::create_dir_all(&dir).map_err(|e| format!("创建后台日志目录失败: {}", e))?;
    Ok(dir.join(format!(
        "terminal-{}.log",
        chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
    )))
}

fn resolve_background_log_file(
    app: &tauri::AppHandle,
    log_file: Option<String>,
) -> Result<PathBuf, String> {
    match log_file
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(path) => {
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建后台日志目录失败: {}", e))?;
            }
            Ok(path)
        }
        None => default_background_log_file(app),
    }
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn spawn_detached_background_process(
    command: &str,
    cwd: Option<&str>,
    log_file: &Path,
) -> Result<u32, String> {
    let wrapper = format!(
        "nohup sh -lc {} >> {} 2>&1 < /dev/null & printf '%s' \"$!\"",
        shell_quote(command),
        shell_quote(&log_file.to_string_lossy())
    );
    let mut shell = Command::new("/bin/sh");
    shell
        .arg("-lc")
        .arg(wrapper)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        shell.current_dir(cwd);
    }

    let output = shell
        .output()
        .map_err(|e| format!("启动后台进程失败: {}", e))?;
    if !output.status.success() {
        return Err(format!("启动后台进程失败: exit {}", output.status));
    }

    let pid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "后台进程已启动，但无法读取 PID".to_string())?;
    Ok(pid)
}

#[cfg(windows)]
fn spawn_detached_background_process(
    command: &str,
    cwd: Option<&str>,
    log_file: &Path,
) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .map_err(|e| format!("打开后台日志失败: {}", e))?;
    let stderr = stdout
        .try_clone()
        .map_err(|e| format!("打开后台日志失败: {}", e))?;

    let mut process = Command::new("cmd.exe");
    process
        .arg("/C")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        process.current_dir(cwd);
    }

    let child = process
        .spawn()
        .map_err(|e| format!("启动后台进程失败: {}", e))?;
    Ok(child.id())
}

// ---------------------------------------------------------------------------
// Output batching helpers
// ---------------------------------------------------------------------------

const EMITTER_CHANNEL_CAPACITY: usize = 64;
const EMITTER_BATCH_SIZE: usize = 8;
const EMITTER_BATCH_WINDOW: Duration = Duration::from_millis(4);

struct TerminalOutputEmitter {
    sender: async_mpsc::Sender<String>,
}

impl TerminalOutputEmitter {
    fn emit(&self, output: String) {
        let _ = self.sender.try_send(output);
    }
}

fn create_terminal_output_emitter(
    app_handle: tauri::AppHandle,
    event_name: String,
) -> TerminalOutputEmitter {
    let (sender, mut receiver) = async_mpsc::channel::<String>(EMITTER_CHANNEL_CAPACITY);
    tauri::async_runtime::spawn(async move {
        let mut buffer = String::with_capacity(OUTPUT_BATCH_MAX_BYTES);
        loop {
            let first = match receiver.recv().await {
                Some(data) => data,
                None => break,
            };

            if first.is_empty() {
                let _ = app_handle.emit(&event_name, String::new());
                continue;
            }

            buffer.push_str(&first);

            let deadline = tokio::time::sleep(EMITTER_BATCH_WINDOW);
            tokio::pin!(deadline);

            loop {
                tokio::select! {
                    data = receiver.recv() => match data {
                        Some(data) if data.is_empty() => {
                            if !buffer.is_empty() {
                                let _ = app_handle.emit(&event_name, std::mem::take(&mut buffer));
                            }
                            let _ = app_handle.emit(&event_name, String::new());
                            break;
                        }
                        Some(data) => {
                            buffer.push_str(&data);
                            if buffer.len() >= OUTPUT_BATCH_MAX_BYTES || buffer.len() / 1024 >= EMITTER_BATCH_SIZE {
                                let _ = app_handle.emit(&event_name, std::mem::take(&mut buffer));
                                break;
                            }
                        }
                        None => {
                            if !buffer.is_empty() {
                                let _ = app_handle.emit(&event_name, std::mem::take(&mut buffer));
                            }
                            return;
                        }
                    },
                    _ = &mut deadline => {
                        if !buffer.is_empty() {
                            let _ = app_handle.emit(&event_name, std::mem::take(&mut buffer));
                        }
                        break;
                    }
                }
            }
        }
    });
    TerminalOutputEmitter { sender }
}

fn flush_terminal_batch(
    emitter: &TerminalOutputEmitter,
    batch: &mut Vec<u8>,
    started_at: &mut Option<Instant>,
) {
    if batch.is_empty() {
        return;
    }
    let output = String::from_utf8_lossy(batch).to_string();
    emitter.emit(output);
    batch.clear();
    *started_at = None;
}

// ---------------------------------------------------------------------------
// SSH reader thread
// ---------------------------------------------------------------------------

fn spawn_ssh_reader(
    app_handle: tauri::AppHandle,
    session_id: String,
    lease: ssh::SharedSshChannelLease,
    mut read_half: ChannelReadHalf,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let event_name = format!("terminal-output-{}", session_id);
        let emitter = create_terminal_output_emitter(app_handle, event_name);
        let mut batch = Vec::with_capacity(OUTPUT_BATCH_MAX_BYTES);
        let mut batch_started_at: Option<Instant> = None;
        let mut closed = false;

        while !closed {
            let msg = lease.block_on(async {
                tokio::select! {
                    msg = read_half.wait() => Some(msg),
                    _ = tokio::time::sleep(OUTPUT_BATCH_WINDOW) => None,
                }
            });

            match msg {
                Some(Some(ChannelMsg::Data { data }))
                | Some(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    if batch_started_at.is_none() {
                        batch_started_at = Some(Instant::now());
                    }
                    batch.extend_from_slice(&data);
                    if batch.len() >= OUTPUT_BATCH_MAX_BYTES
                        || batch_started_at
                            .map(|s| s.elapsed() >= OUTPUT_BATCH_WINDOW)
                            .unwrap_or(false)
                    {
                        flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
                    }
                }
                Some(Some(ChannelMsg::Eof)) | Some(Some(ChannelMsg::Close)) | Some(None) => {
                    closed = true;
                }
                None => {
                    flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
                }
                _ => {}
            }
        }

        flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
        emitter.emit(String::new());
    })
}

// ---------------------------------------------------------------------------
// Local PTY reader thread
// ---------------------------------------------------------------------------

fn spawn_local_reader(
    app_handle: tauri::AppHandle,
    session_id: String,
    mut reader: Box<dyn Read + Send>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let event_name = format!("terminal-output-{}", session_id);
        let emitter = create_terminal_output_emitter(app_handle, event_name);
        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();

        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; READ_BUFFER_SIZE];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(None);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(Some(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(IDLE_SLEEP);
                    }
                    Err(_) => {
                        let _ = tx.send(None);
                        break;
                    }
                }
            }
        });

        let mut batch = Vec::with_capacity(OUTPUT_BATCH_MAX_BYTES);
        let mut batch_started_at: Option<Instant> = None;
        let mut closed = false;

        while !closed {
            match rx.recv_timeout(OUTPUT_BATCH_WINDOW) {
                Ok(Some(chunk)) => {
                    if batch_started_at.is_none() {
                        batch_started_at = Some(Instant::now());
                    }
                    batch.extend_from_slice(&chunk);
                    if batch.len() >= OUTPUT_BATCH_MAX_BYTES {
                        flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
                    }
                }
                Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    closed = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
                }
            }
        }

        flush_terminal_batch(&emitter, &mut batch, &mut batch_started_at);
        emitter.emit(String::new());
        let _ = reader_handle.join();
    })
}

// ---------------------------------------------------------------------------
// SSH actor: owns a shared SSH connection reference + one terminal channel write half.
// Dropping the actor only closes this terminal channel; the shared SSH transport
// can remain alive for monitor/SFTP channels held by SshConnectionRegistry/SftpManager.
fn spawn_ssh_actor(
    resources: SshActorResources,
) -> (
    async_mpsc::Sender<TerminalCommand>,
    std::thread::JoinHandle<()>,
) {
    let (sender, mut receiver) = async_mpsc::channel::<TerminalCommand>(ACTOR_CHANNEL_CAPACITY);

    let actor_thread = std::thread::spawn(move || {
        let SshActorResources { lease, write_half } = resources;
        lease.block_on(async move {
            while let Some(cmd) = receiver.recv().await {
                match cmd {
                    TerminalCommand::Write(bytes) => {
                        if write_half.data_bytes(bytes).await.is_err() {
                            break;
                        }
                    }
                    TerminalCommand::Resize { cols, rows } => {
                        if write_half
                            .window_change(cols as u32, rows as u32, 0, 0)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    TerminalCommand::Disconnect => {
                        let _ = write_half.eof().await;
                        let _ = write_half.close().await;
                        break;
                    }
                }
            }
        });
    });

    (sender, actor_thread)
}

// ---------------------------------------------------------------------------
// Local PTY actor: owns writer, processes write commands (blocking I/O)
// ---------------------------------------------------------------------------

fn spawn_local_actor(
    resources: LocalActorResources,
) -> (
    async_mpsc::Sender<TerminalCommand>,
    std::thread::JoinHandle<()>,
) {
    let (sender, mut receiver) = async_mpsc::channel::<TerminalCommand>(ACTOR_CHANNEL_CAPACITY);

    let actor_thread = std::thread::spawn(move || {
        let LocalActorResources { writer } = resources;
        // Use a minimal single-threaded runtime for the mpsc receiver.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build local actor runtime");

        rt.block_on(async move {
            while let Some(cmd) = receiver.recv().await {
                match cmd {
                    TerminalCommand::Write(bytes) => {
                        let mut w = match writer.lock() {
                            Ok(w) => w,
                            Err(_) => break,
                        };
                        if w.write_all(&bytes).is_err() || w.flush().is_err() {
                            break;
                        }
                    }
                    TerminalCommand::Resize { .. } => {
                        // Local PTY resize uses the MasterPty directly,
                        // handled in terminal_resize. This path is not used.
                    }
                    TerminalCommand::Disconnect => {
                        break;
                    }
                }
            }
            // writer drops here, PTY writer fd closes.
        });
    });

    (sender, actor_thread)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn terminal_connect(
    app: tauri::AppHandle,
    host_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
    session_id: Option<String>,
) -> Result<String, String> {
    let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let (ip, port, auth_type, username, password, private_key, jump_chain_json, proxy_settings): TerminalHostRow = {
        let db = app.state::<Database>();
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT ip, port, auth_type, username, password, private_key, COALESCE(jump_chain, '[]'), COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            rusqlite::params![host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, String>(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|e| format!("host not found: {}", e))?
    };

    let password =
        crate::commands::hosts::resolve_host_password(&host_id, password).map_err(|e| {
            crate::commands::app_log::emit_log(
                &app,
                "error",
                "host-secret",
                &format!(
                    "terminal_connect password resolve failed host={} {} error={}",
                    host_id,
                    crate::keychain::host_password_debug_label(&host_id),
                    e
                ),
                "backend",
            );
            e
        })?;
    let private_key = crate::commands::hosts::resolve_host_private_key(&host_id, private_key)
        .map_err(|e| {
            crate::commands::app_log::emit_log(
                &app,
                "error",
                "host-secret",
                &format!(
                    "terminal_connect private_key resolve failed host={} error={}",
                    host_id, e
                ),
                "backend",
            );
            e
        })?;

    let config = ssh::SshConfig {
        host: ip.clone(),
        port: port as u16,
        username: username.clone(),
        auth_type: auth_type.clone(),
        password,
        private_key,
        proxy: crate::commands::hosts::resolve_host_proxy_settings(&host_id, proxy_settings)?,
    };
    let pool = app.state::<ssh::SshConnectionRegistry>();
    pool.remember_config(&host_id, &config);

    let cols = cols.unwrap_or(80);
    let rows = rows.unwrap_or(24);

    let jump_chain: Vec<String> = serde_json::from_str(&jump_chain_json).unwrap_or_default();
    let jump_configs = if jump_chain.is_empty() {
        vec![]
    } else {
        let db = app.state::<Database>();
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut configs = Vec::with_capacity(jump_chain.len());
        for jump_id in &jump_chain {
            let (jip, jport, jauth, juname, jpass, jkey, jproxy): (
                String, i32, String, String, Option<String>, Option<String>, Option<String>,
            ) = conn
                .query_row(
                    "SELECT ip, port, auth_type, username, password, private_key, COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
                    rusqlite::params![jump_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
                )
                .map_err(|e| format!("跳板机 {} 未找到: {}", jump_id, e))?;
            let jpass =
                crate::commands::hosts::resolve_host_password(jump_id, jpass).map_err(|e| {
                    crate::commands::app_log::emit_log(
                        &app,
                        "error",
                        "host-secret",
                        &format!(
                            "terminal_connect jump password resolve failed host={} error={}",
                            jump_id, e
                        ),
                        "backend",
                    );
                    e
                })?;
            let jkey =
                crate::commands::hosts::resolve_host_private_key(jump_id, jkey).map_err(|e| {
                    crate::commands::app_log::emit_log(
                        &app,
                        "error",
                        "host-secret",
                        &format!(
                            "terminal_connect jump private_key resolve failed host={} error={}",
                            jump_id, e
                        ),
                        "backend",
                    );
                    e
                })?;
            let jump_config = ssh::SshConfig {
                host: jip,
                port: jport as u16,
                username: juname,
                auth_type: jauth,
                password: jpass,
                private_key: jkey,
                proxy: crate::commands::hosts::resolve_host_proxy_settings(jump_id, jproxy)?,
            };
            pool.remember_config(jump_id, &jump_config);
            configs.push(jump_config);
        }
        configs
    };

    // Run blocking SSH work on a dedicated thread to avoid blocking the async runtime.
    // Use std::thread::spawn directly instead of tokio::spawn_blocking to avoid
    // queueing on the limited blocking thread pool when opening many terminals.
    let app_clone = app.clone();
    let host_id_clone = host_id.clone();
    let config_clone = config.clone();
    let jump_configs_clone = jump_configs.clone();

    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let pool = app_clone.state::<ssh::SshConnectionRegistry>();
        let result = if jump_configs_clone.is_empty() {
            pool.open_terminal_channel(&host_id_clone, &config_clone, 10, cols, rows)
        } else {
            pool.open_terminal_channel_via_jump(
                &host_id_clone,
                &config_clone,
                &jump_configs_clone,
                10,
                cols,
                rows,
            )
        };
        let _ = tx.send(result);
    });

    let channel_result = rx
        .await
        .map_err(|e| format!("连接线程异常: {}", e))?
        .map_err(|e| format!("连接失败: {}", e))?;

    let ssh::PooledTerminalChannel {
        read_half,
        write_half,
        lease,
    } = channel_result;

    let reader_lease = lease.clone();
    let _reader_handle = spawn_ssh_reader(app.clone(), session_id.clone(), reader_lease, read_half);

    let (actor_sender, actor_thread) = spawn_ssh_actor(SshActorResources { lease, write_half });

    let manager = app.state::<TerminalManager>();
    manager.sessions.insert(
        session_id.clone(),
        SessionHandle {
            sender: actor_sender,
            actor_thread,
            local_master: None,
            local_child: None,
        },
    );

    // Pre-initialize SFTP session lazily via frontend after terminal is interactive,
    // to avoid blocking SSH transport during terminal startup.
    // See: auxiliaryReady delay in TerminalPage.tsx

    Ok(session_id)
}

#[tauri::command]
pub fn terminal_connect_local(
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<String, String> {
    let shell = select_local_shell(current_shell_platform(), std::env::vars(), |path| {
        Path::new(path).exists()
    });
    let cols = cols.unwrap_or(80);
    let rows = rows.unwrap_or(24);

    eprintln!(
        "[Terminal] Starting local PTY shell={} cols={} rows={}",
        shell, cols, rows
    );

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("创建本机 PTY 失败: {}", e))?;

    let mut command = CommandBuilder::new(shell.clone());
    command.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| format!("本机 shell 启动失败（{}）: {}", shell, e))?;

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(e) => {
            let _ = child.kill();
            return Err(format!("初始化本机终端输出失败: {}", e));
        }
    };

    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(e) => {
            let _ = child.kill();
            return Err(format!("初始化本机终端输入失败: {}", e));
        }
    };

    drop(pair.slave);

    let session_id = uuid::Uuid::new_v4().to_string();
    let _reader_handle = spawn_local_reader(app.clone(), session_id.clone(), reader);

    let writer = Arc::new(Mutex::new(writer));
    let master = Arc::new(Mutex::new(pair.master));

    let (actor_sender, actor_thread) = spawn_local_actor(LocalActorResources {
        writer: writer.clone(),
    });

    let child = Arc::new(Mutex::new(child));

    // Insert into DashMap.
    manager.sessions.insert(
        session_id.clone(),
        SessionHandle {
            sender: actor_sender,
            actor_thread,
            local_master: Some(master),
            local_child: Some(child),
        },
    );

    Ok(session_id)
}

#[tauri::command]
pub async fn terminal_spawn_background_process(
    app: tauri::AppHandle,
    command: String,
    cwd: Option<String>,
    log_file: Option<String>,
) -> Result<BackgroundProcessInfo, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("后台命令不能为空".to_string());
    }

    let log_file = resolve_background_log_file(&app, log_file)?;
    let pid = spawn_detached_background_process(command, cwd.as_deref(), &log_file)?;
    crate::commands::app_log::emit_log(
        &app,
        "info",
        "terminal",
        &format!(
            "Detached background process pid={} log={}",
            pid,
            log_file.display()
        ),
        "backend",
    );

    Ok(BackgroundProcessInfo {
        pid,
        log_file: log_file.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn terminal_write(
    manager: tauri::State<'_, TerminalManager>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sender = {
        let handle = manager
            .sessions
            .get(&session_id)
            .ok_or("session not found")?;
        handle.sender.clone()
    };

    sender
        .try_send(TerminalCommand::Write(data.into_bytes()))
        .map_err(|err| match err {
            async_mpsc::error::TrySendError::Full(_) => "session write channel busy".to_string(),
            async_mpsc::error::TrySendError::Closed(_) => {
                let _ = manager.sessions.remove(&session_id);
                "session write channel closed".to_string()
            }
        })
}

#[tauri::command]
pub async fn terminal_batch_write(
    manager: tauri::State<'_, TerminalManager>,
    writes: Vec<(String, String)>,
) -> Result<(), String> {
    for (session_id, data) in writes {
        let sender = {
            let handle = match manager.sessions.get(&session_id) {
                Some(h) => h,
                None => continue,
            };
            handle.sender.clone()
        };
        let _ = sender.try_send(TerminalCommand::Write(data.into_bytes()));
    }
    Ok(())
}

#[tauri::command]
pub async fn terminal_resize(
    manager: tauri::State<'_, TerminalManager>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    // For each session kind, handle resize differently.
    // SSH: send resize command through actor.
    // Local: resize the MasterPty directly (no actor round-trip needed).
    let handle = manager
        .sessions
        .get(&session_id)
        .ok_or("session not found")?;

    if let Some(ref master) = handle.local_master {
        // Local PTY resize: use MasterPty directly.
        let master = master.clone();
        drop(handle); // Release DashMap ref before blocking I/O.
        let m = master.lock().map_err(|e| e.to_string())?;
        m.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
    } else {
        // SSH resize: send through actor channel.
        let sender = handle.sender.clone();
        drop(handle); // Release DashMap ref before blocking send.
        sender
            .try_send(TerminalCommand::Resize { cols, rows })
            .map_err(|err| match err {
                async_mpsc::error::TrySendError::Full(_) => {
                    "session resize channel busy".to_string()
                }
                async_mpsc::error::TrySendError::Closed(_) => {
                    "session resize channel closed".to_string()
                }
            })?;
    }

    Ok(())
}

#[tauri::command]
pub async fn terminal_disconnect(
    manager: tauri::State<'_, TerminalManager>,
    session_id: String,
    _host_id: Option<String>,
) -> Result<(), String> {
    if let Some((_, handle)) = manager.sessions.remove(&session_id) {
        let _ = handle.sender.try_send(TerminalCommand::Disconnect);
        cleanup_terminal_session_in_background(session_id, handle);
    }

    Ok(())
}

#[cfg(test)]
mod background_process_tests {
    #[cfg(unix)]
    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(super::shell_quote("mio's app"), "'mio'\\''s app'");
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_handles_empty_values() {
        assert_eq!(super::shell_quote(""), "''");
    }
}
