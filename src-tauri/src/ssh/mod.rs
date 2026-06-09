mod host_key;

use dashmap::DashMap;
use russh::keys::ssh_key;
use russh::{
    cipher, client, compression, kex, mac, Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf,
    Preferred,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fs;
use std::future::Future;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;
use tokio::runtime::Runtime;

use host_key::HostKeyVerifier;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub success: bool,
    pub output: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResult {
    pub host: String,
    pub success: bool,
    pub error: Option<String>,
    pub file_size: u64,
    pub duration_ms: u64,
}

pub struct ClientHandler {
    verifier: HostKeyVerifier,
}

impl ClientHandler {
    fn new(host_id: impl Into<String>) -> Self {
        Self {
            verifier: HostKeyVerifier::new(host_id),
        }
    }
}

fn build_ssh_client_config() -> client::Config {
    client::Config {
        preferred: Preferred {
            kex: Cow::Borrowed(&[
                kex::CURVE25519,
                kex::CURVE25519_PRE_RFC_8731,
                kex::ECDH_SHA2_NISTP256,
                kex::ECDH_SHA2_NISTP384,
                kex::ECDH_SHA2_NISTP521,
                kex::DH_GEX_SHA256,
                kex::DH_G18_SHA512,
                kex::DH_G17_SHA512,
                kex::DH_G16_SHA512,
                kex::DH_G15_SHA512,
                kex::DH_G14_SHA256,
                kex::DH_G14_SHA1,
                kex::DH_GEX_SHA1,
                kex::EXTENSION_SUPPORT_AS_CLIENT,
                kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
            ]),
            key: Cow::Borrowed(&[
                ssh_key::Algorithm::Ed25519,
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP256,
                },
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP384,
                },
                ssh_key::Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP521,
                },
                ssh_key::Algorithm::Rsa {
                    hash: Some(ssh_key::HashAlg::Sha512),
                },
                ssh_key::Algorithm::Rsa {
                    hash: Some(ssh_key::HashAlg::Sha256),
                },
                ssh_key::Algorithm::Rsa { hash: None },
            ]),
            cipher: Cow::Borrowed(&[
                cipher::CHACHA20_POLY1305,
                cipher::AES_256_GCM,
                cipher::AES_128_GCM,
                cipher::AES_256_CTR,
                cipher::AES_192_CTR,
                cipher::AES_128_CTR,
                cipher::AES_256_CBC,
                cipher::AES_192_CBC,
                cipher::AES_128_CBC,
            ]),
            mac: Cow::Borrowed(&[
                mac::HMAC_SHA512_ETM,
                mac::HMAC_SHA256_ETM,
                mac::HMAC_SHA512,
                mac::HMAC_SHA256,
                mac::HMAC_SHA1_ETM,
                mac::HMAC_SHA1,
            ]),
            compression: Cow::Borrowed(&[compression::NONE]),
        },
        keepalive_interval: Some(std::time::Duration::from_secs(15)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &ssh_key::PublicKey) -> Result<bool, Self::Error> {
        let fingerprint = key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
        self.verifier.verify_fingerprint(&fingerprint).map_err(|e| {
            eprintln!("[SSH] Host key verification failed: {}", e);
            russh::Error::Disconnect
        })
    }
}

pub struct SshConnection {
    handle: client::Handle<ClientHandler>,
    pub(crate) runtime: Runtime,
}

impl SshConnection {
    pub async fn open_channel_split_async(
        &self,
    ) -> Result<(ChannelReadHalf, ChannelWriteHalf<client::Msg>), String> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| format!("channel failed: {}", e))?;

        Ok(channel.split())
    }

    pub async fn open_channel_async(&self) -> Result<Channel<client::Msg>, String> {
        self.handle
            .channel_open_session()
            .await
            .map_err(|e| format!("channel failed: {}", e))
    }

    pub fn into_shared(self) -> SharedSshConnection {
        SharedSshConnection::from_connection(self)
    }
}

struct SharedSshConnectionInner {
    runtime: Runtime,
    handle: tokio::sync::Mutex<client::Handle<ClientHandler>>,
}

/// Keeps one shared SSH transport alive for the lifetime of an opened channel.
#[derive(Clone)]
pub struct SharedSshChannelLease {
    connection: SharedSshConnection,
}

impl SharedSshChannelLease {
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.connection.block_on(future)
    }

    pub fn is_closed(&self) -> bool {
        self.connection.is_closed()
    }
}

/// Cloneable reference to one authenticated SSH transport.
///
/// The russh client handle itself is not cloneable because it owns reply state,
/// so the pool stores it behind an async mutex and shares this wrapper. Each
/// terminal/stat/SFTP operation opens its own channel on the same transport.
#[derive(Clone)]
pub struct SharedSshConnection {
    inner: Arc<SharedSshConnectionInner>,
}

impl SharedSshConnection {
    fn from_connection(conn: SshConnection) -> Self {
        Self {
            inner: Arc::new(SharedSshConnectionInner {
                runtime: conn.runtime,
                handle: tokio::sync::Mutex::new(conn.handle),
            }),
        }
    }

    fn same_transport(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn is_closed(&self) -> bool {
        match self.inner.handle.try_lock() {
            Ok(handle) => handle.is_closed(),
            Err(_) => false,
        }
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.inner.runtime.block_on(future)
    }

    pub async fn open_channel_split_async(
        &self,
    ) -> Result<(ChannelReadHalf, ChannelWriteHalf<client::Msg>), String> {
        let handle = self.inner.handle.lock().await;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("channel failed: {}", e))?;
        Ok(channel.split())
    }

    pub async fn open_channel_async(&self) -> Result<Channel<client::Msg>, String> {
        let handle = self.inner.handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|e| format!("channel failed: {}", e))
    }

    pub async fn open_direct_tcpip_channel_async(
        &self,
        host: &str,
        port: u32,
    ) -> Result<Channel<client::Msg>, String> {
        let handle = self.inner.handle.lock().await;
        handle
            .channel_open_direct_tcpip(host, port, "127.0.0.1", 0)
            .await
            .map_err(|e| format!("direct-tcpip channel failed: {}", e))
    }

    pub async fn tcpip_forward_async(&self, host: &str, port: u32) -> Result<(), String> {
        let handle = self.inner.handle.lock().await;
        handle
            .tcpip_forward(host, port)
            .await
            .map_err(|e| format!("tcpip_forward failed: {}", e))?;
        Ok(())
    }

    pub async fn cancel_tcpip_forward_async(&self, host: &str, port: u32) -> Result<(), String> {
        let handle = self.inner.handle.lock().await;
        handle
            .cancel_tcpip_forward(host, port)
            .await
            .map_err(|e| format!("cancel_tcpip_forward failed: {}", e))?;
        Ok(())
    }

    pub async fn open_sftp_session(&self) -> Result<russh_sftp::client::SftpSession, String> {
        let channel = self
            .open_channel_async()
            .await
            .map_err(|e| format!("open channel: {}", e))?;
        channel
            .request_subsystem(false, "sftp")
            .await
            .map_err(|e| format!("sftp subsystem: {}", e))?;
        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| format!("sftp init: {}", e))?;
        Ok(sftp)
    }
}

pub fn connect_internal(config: &SshConfig, timeout_secs: u64) -> Result<SshConnection, String> {
    connect_internal_with_host_id(
        &format!("{}:{}", config.host, config.port),
        config,
        timeout_secs,
    )
}

fn connect_internal_with_host_id(
    host_id: &str,
    config: &SshConfig,
    timeout_secs: u64,
) -> Result<SshConnection, String> {
    let runtime = Runtime::new().map_err(|e| format!("runtime init failed: {}", e))?;

    let config_clone = config.clone();

    let addr = format!("{}:{}", config_clone.host, config_clone.port);
    let socket_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("DNS解析失败 {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("未找到地址: {}", addr))?;

    let tcp = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(timeout_secs))
        .map_err(|e| format!("TCP连接失败: {}", e))?;
    tcp.set_nonblocking(true)
        .map_err(|e| format!("设置非阻塞失败: {}", e))?;

    let config_clone = config.clone();
    let verifier_id = host_id.to_string();

    let handle = runtime.block_on(async move {
        let tcp = tokio::net::TcpStream::from_std(tcp)
            .map_err(|e| format!("TcpStream转换失败: {}", e))?;

        let ssh_config = build_ssh_client_config();

        let handler = ClientHandler::new(verifier_id);

        let mut handle = client::connect_stream(Arc::new(ssh_config), tcp, handler)
            .await
            .map_err(|e| format!("SSH握手失败: {}", e))?;

        let auth_result = match config_clone.auth_type.as_str() {
            "password" => handle
                .authenticate_password(
                    &config_clone.username,
                    config_clone.password.as_deref().unwrap_or(""),
                )
                .await
                .map_err(|e| format!("认证失败: {}", e))?,
            "key" => {
                let key_data = config_clone.private_key.as_deref().unwrap_or("");
                if key_data.is_empty() {
                    return Err("私钥内容为空".to_string());
                }
                let key_pair = russh::keys::decode_secret_key(key_data, None)
                    .map_err(|e| format!("私钥解析失败: {}", e))?;
                let hash_alg = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|e| format!("hash alg error: {}", e))?;
                let key_with_alg =
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg.flatten());
                handle
                    .authenticate_publickey(&config_clone.username, key_with_alg)
                    .await
                    .map_err(|e| format!("密钥认证失败: {}", e))?
            }
            _ => return Err(format!("不支持的认证类型: {}", config_clone.auth_type)),
        };

        match auth_result {
            russh::client::AuthResult::Success => {}
            russh::client::AuthResult::Failure { .. } => {
                return Err("认证被拒绝".to_string());
            }
        }

        Ok(handle)
    })?;

    Ok(SshConnection { handle, runtime })
}

/// Connect to a target host through a chain of jump hosts using recursive direct-tcpip channels.
///
/// Connection flow: Client → Jump1 → Jump2 → … → Target
/// Each jump opens a `direct-tcpip` channel on the previous host's SSH session,
/// then converts it to a stream via `channel.into_stream()` for the next SSH handshake.
pub fn connect_via_jump_chain(
    target_config: &SshConfig,
    jump_configs: &[SshConfig],
    timeout_secs: u64,
) -> Result<SshConnection, String> {
    if jump_configs.is_empty() {
        return connect_internal(target_config, timeout_secs);
    }

    let runtime = Runtime::new().map_err(|e| format!("runtime init failed: {}", e))?;

    // Step 1: Direct TCP connect to first jump host
    let first_config = &jump_configs[0];
    let addr = format!("{}:{}", first_config.host, first_config.port);
    let socket_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("DNS解析失败 {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("未找到地址: {}", addr))?;
    let tcp = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(timeout_secs))
        .map_err(|e| format!("跳板机 {} TCP连接失败: {}", first_config.host, e))?;
    tcp.set_nonblocking(true)
        .map_err(|e| format!("设置非阻塞失败: {}", e))?;

    // Step 2: SSH handshake + auth on first jump
    let first_config_clone = first_config.clone();
    let mut current_handle = runtime.block_on(async move {
        let tcp = tokio::net::TcpStream::from_std(tcp)
            .map_err(|e| format!("TcpStream转换失败: {}", e))?;
        let ssh_config = build_ssh_client_config();
        let mut handle = client::connect_stream(
            Arc::new(ssh_config),
            tcp,
            ClientHandler::new(format!(
                "{}:{}",
                first_config_clone.host, first_config_clone.port
            )),
        )
        .await
        .map_err(|e| format!("跳板机 {} SSH握手失败: {}", first_config_clone.host, e))?;
        authenticate_handle(&mut handle, &first_config_clone).await?;
        Ok::<_, String>(handle)
    })?;

    // Step 3: For each subsequent hop, open direct-tcpip channel → SSH handshake on that stream
    let hops: Vec<&SshConfig> = jump_configs[1..]
        .iter()
        .chain(std::iter::once(target_config))
        .collect();

    for (i, hop_config) in hops.iter().enumerate() {
        let is_target = i == hops.len() - 1;
        let label = if is_target {
            "目标主机"
        } else {
            "跳板机"
        };
        let hop = (*hop_config).clone();

        current_handle = runtime.block_on(async move {
            // Open a direct-tcpip channel to the next hop
            let channel = current_handle
                .channel_open_direct_tcpip(&hop.host, hop.port as u32, "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    format!(
                        "{} {}:{} direct-tcpip 通道打开失败: {}",
                        label, hop.host, hop.port, e
                    )
                })?;

            // Convert channel to a stream (implements AsyncRead + AsyncWrite)
            let stream = channel.into_stream();

            // Perform SSH handshake on the tunneled stream
            let ssh_config = build_ssh_client_config();
            let mut handle = client::connect_stream(
                Arc::new(ssh_config),
                stream,
                ClientHandler::new(format!("{}:{}", hop.host, hop.port)),
            )
            .await
            .map_err(|e| format!("{} {}:{} SSH握手失败: {}", label, hop.host, hop.port, e))?;

            authenticate_handle(&mut handle, &hop).await?;

            Ok::<_, String>(handle)
        })?;
    }

    Ok(SshConnection {
        handle: current_handle,
        runtime,
    })
}

/// Authenticate an SSH handle using the given config.
async fn authenticate_handle(
    handle: &mut client::Handle<ClientHandler>,
    config: &SshConfig,
) -> Result<(), String> {
    let auth_result = match config.auth_type.as_str() {
        "password" => handle
            .authenticate_password(&config.username, config.password.as_deref().unwrap_or(""))
            .await
            .map_err(|e| format!("认证失败: {}", e))?,
        "key" => {
            let key_data = config.private_key.as_deref().unwrap_or("");
            if key_data.is_empty() {
                return Err("私钥内容为空".to_string());
            }
            let key_pair = russh::keys::decode_secret_key(key_data, None)
                .map_err(|e| format!("私钥解析失败: {}", e))?;
            let hash_alg = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| format!("hash alg error: {}", e))?;
            let key_with_alg =
                russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg.flatten());
            handle
                .authenticate_publickey(&config.username, key_with_alg)
                .await
                .map_err(|e| format!("密钥认证失败: {}", e))?
        }
        _ => return Err(format!("不支持的认证类型: {}", config.auth_type)),
    };

    match auth_result {
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure { .. } => Err("认证被拒绝".to_string()),
    }
}

pub async fn execute_command_on_connection(
    conn: &SshConnection,
    command: &str,
    start: std::time::Instant,
) -> Result<ExecResult, String> {
    let (mut read_half, write_half) = conn.open_channel_split_async().await?;

    write_half
        .exec(true, command)
        .await
        .map_err(|e| format!("exec failed: {}", e))?;

    read_command_output(&mut read_half, &write_half, start).await
}

pub async fn execute_command_on_shared_connection(
    conn: &SharedSshConnection,
    command: &str,
    start: std::time::Instant,
) -> Result<ExecResult, String> {
    let (mut read_half, write_half) = conn.open_channel_split_async().await?;

    write_half
        .exec(true, command)
        .await
        .map_err(|e| format!("exec failed: {}", e))?;

    read_command_output(&mut read_half, &write_half, start).await
}

async fn read_command_output(
    read_half: &mut ChannelReadHalf,
    write_half: &ChannelWriteHalf<client::Msg>,
    start: std::time::Instant,
) -> Result<ExecResult, String> {
    let mut output = Vec::new();
    let mut exit_code = -1i32;
    let mut exec_accepted = false;

    while let Some(msg) = read_half.wait().await {
        match msg {
            ChannelMsg::Data { data } => output.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => {
                exit_code = exit_status as i32;
            }
            ChannelMsg::ExitSignal { signal_name, .. } => {
                eprintln!("[SSH Exec] ExitSignal={:?}", signal_name);
            }
            ChannelMsg::Success => {
                exec_accepted = true;
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    let _ = write_half.eof().await;

    Ok(ExecResult {
        success: exit_code == 0 || (exit_code == -1 && (exec_accepted || !output.is_empty())),
        output: String::from_utf8_lossy(&output).to_string(),
        exit_code,
        duration_ms,
    })
}

pub fn execute_command(
    config: &SshConfig,
    command: &str,
    timeout_secs: u64,
) -> Result<ExecResult, String> {
    let start = std::time::Instant::now();
    let conn = connect_internal(config, timeout_secs)?;

    conn.runtime
        .block_on(execute_command_on_connection(&conn, command, start))
}

pub fn upload_file(
    config: &SshConfig,
    local_path: &str,
    remote_path: &str,
    timeout_secs: u64,
) -> Result<TransferResult, String> {
    let start = std::time::Instant::now();

    let local = Path::new(local_path);
    if !local.exists() {
        return Err(format!("local file not found: {}", local_path));
    }

    let file_size = local.metadata().map(|m| m.len()).unwrap_or(0);
    let data = fs::read(local).map_err(|e| format!("read local file failed: {}", e))?;

    let conn = connect_internal(config, timeout_secs)?;

    conn.runtime.block_on(async {
        let channel = conn.open_channel_async().await?;
        channel
            .request_subsystem(false, "sftp")
            .await
            .map_err(|e| format!("SFTP subsystem request failed: {}", e))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| format!("SFTP session init failed: {}", e))?;

        sftp.write(remote_path, &data)
            .await
            .map_err(|e| format!("write failed: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(TransferResult {
            host: config.host.clone(),
            success: true,
            error: None,
            file_size,
            duration_ms,
        })
    })
}

pub fn download_file(
    config: &SshConfig,
    remote_path: &str,
    local_dir: &str,
    timeout_secs: u64,
) -> Result<TransferResult, String> {
    let start = std::time::Instant::now();

    let conn = connect_internal(config, timeout_secs)?;

    conn.runtime.block_on(async {
        let channel = conn.open_channel_async().await?;
        channel
            .request_subsystem(false, "sftp")
            .await
            .map_err(|e| format!("SFTP subsystem request failed: {}", e))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| format!("SFTP session init failed: {}", e))?;

        let metadata = sftp
            .metadata(remote_path)
            .await
            .map_err(|e| format!("stat remote file failed: {}", e))?;
        let file_size = metadata.len();

        let data = sftp
            .read(remote_path)
            .await
            .map_err(|e| format!("read failed: {}", e))?;

        let dir = PathBuf::from(local_dir);
        fs::create_dir_all(&dir).ok();
        let file_name = Path::new(remote_path)
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("downloaded"));
        let local_file_path = dir.join(file_name);
        fs::write(&local_file_path, &data)
            .map_err(|e| format!("write local file failed: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(TransferResult {
            host: config.host.clone(),
            success: true,
            error: None,
            file_size,
            duration_ms,
        })
    })
}

pub struct PooledSftpSession {
    pub sftp: russh_sftp::client::SftpSession,
    pub lease: SharedSshChannelLease,
}

pub struct PooledTerminalChannel {
    pub read_half: ChannelReadHalf,
    pub write_half: ChannelWriteHalf<client::Msg>,
    pub lease: SharedSshChannelLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    Connecting,
    Active,
    Idle,
    LinkDown,
    Reconnecting,
}

struct ConnectionEntry {
    connection: SharedSshConnection,
    state: Mutex<ConnectionState>,
    last_activity: Mutex<Instant>,
}

impl ConnectionEntry {
    fn new(connection: SharedSshConnection) -> Self {
        Self {
            connection,
            state: Mutex::new(ConnectionState::Active),
            last_activity: Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        *self.last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }
}

pub struct SshConnectionRegistry {
    connections: DashMap<String, ConnectionEntry>,
    connection_locks: DashMap<String, Arc<Mutex<()>>>,
    app_handle: Mutex<Option<tauri::AppHandle>>,
    idle_timeout: Duration,
}

impl SshConnectionRegistry {
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            connection_locks: DashMap::new(),
            app_handle: Mutex::new(None),
            idle_timeout: Duration::from_secs(15 * 60),
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut guard) = self.app_handle.lock() {
            *guard = Some(handle);
        }
    }

    fn emit_connection_status(&self, host_id: &str, state: ConnectionState) {
        if let Ok(guard) = self.app_handle.lock() {
            if let Some(app) = guard.as_ref() {
                use tauri::Emitter;
                let status = match state {
                    ConnectionState::Connecting => "connecting",
                    ConnectionState::Active => "active",
                    ConnectionState::Idle => "idle",
                    ConnectionState::LinkDown => "link_down",
                    ConnectionState::Reconnecting => "reconnecting",
                };
                let _ = app.emit(
                    "connection_status_changed",
                    serde_json::json!({ "hostId": host_id, "status": status }),
                );

                // Push to global log
                let level = match state {
                    ConnectionState::LinkDown => "error",
                    ConnectionState::Reconnecting => "warn",
                    _ => "info",
                };
                let msg = match state {
                    ConnectionState::Connecting => format!("主机 {} 正在连接...", host_id),
                    ConnectionState::Active => format!("主机 {} 已连接", host_id),
                    ConnectionState::Idle => format!("主机 {} 连接空闲", host_id),
                    ConnectionState::LinkDown => format!("主机 {} 连接断开", host_id),
                    ConnectionState::Reconnecting => format!("主机 {} 正在重连...", host_id),
                };
                crate::commands::app_log::emit_log(app, level, "ssh", &msg, "backend");
            }
        }
    }

    fn log(&self, level: &str, message: &str) {
        if let Ok(guard) = self.app_handle.lock() {
            if let Some(app) = guard.as_ref() {
                crate::commands::app_log::emit_log(app, level, "ssh", message, "backend");
            }
        }
    }

    pub fn start_idle_reaper(app_handle: tauri::AppHandle) {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60));
            let registry = app_handle.state::<SshConnectionRegistry>();
            registry.reap_idle();
        });
    }

    fn reap_idle(&self) {
        let now = Instant::now();
        let keys: Vec<String> = self.connections.iter().map(|r| r.key().clone()).collect();
        for key in keys {
            if let Some(entry) = self.connections.get(&key) {
                let last = *entry
                    .last_activity
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if now.duration_since(last) > self.idle_timeout {
                    let should_remove = {
                        let s = entry.state.lock().unwrap_or_else(|e| e.into_inner());
                        *s == ConnectionState::Active
                    };
                    drop(entry);
                    if should_remove {
                        self.connections.remove(&key);
                        eprintln!("[SSH Registry] Idle timeout for {}", key);
                        self.emit_connection_status(&key, ConnectionState::Idle);
                    }
                }
            }
        }
    }

    pub fn execute(
        &self,
        host_id: &str,
        config: &SshConfig,
        command: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        let conn = self.get_or_connect(host_id, config, timeout_secs)?;
        match Self::execute_on_shared(&conn, command) {
            Ok(result) => Ok(result.output),
            Err(err) => {
                eprintln!(
                    "[SSH Registry] Shared connection failed for {} during exec: {}; reconnecting once",
                    host_id, err
                );
                self.remove_if_same(host_id, &conn);
                let conn = self.reconnect_and_store(host_id, config, timeout_secs)?;
                let result = Self::execute_on_shared(&conn, command)?;
                Ok(result.output)
            }
        }
    }

    pub fn open_sftp_session(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
    ) -> Result<PooledSftpSession, String> {
        let conn = self.get_or_connect(host_id, config, timeout_secs)?;
        match Self::open_sftp_on_shared(conn.clone()) {
            Ok(session) => Ok(session),
            Err(err) => {
                eprintln!(
                    "[SSH Registry] Failed to open SFTP on shared connection for {}: {}; reconnecting once",
                    host_id, err
                );
                self.remove_if_same(host_id, &conn);
                let conn = self.reconnect_and_store(host_id, config, timeout_secs)?;
                Self::open_sftp_on_shared(conn)
            }
        }
    }

    pub fn open_terminal_channel(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
        cols: u16,
        rows: u16,
    ) -> Result<PooledTerminalChannel, String> {
        let conn = self.get_or_connect(host_id, config, timeout_secs)?;
        match Self::open_terminal_on_shared(conn.clone(), cols, rows) {
            Ok(channel) => Ok(channel),
            Err(err) => {
                eprintln!(
                    "[SSH Registry] Failed to open terminal on shared connection for {}: {}; reconnecting once",
                    host_id, err
                );
                self.remove_if_same(host_id, &conn);
                let conn = self.reconnect_and_store(host_id, config, timeout_secs)?;
                Self::open_terminal_on_shared(conn, cols, rows)
            }
        }
    }

    fn execute_on_shared(conn: &SharedSshConnection, command: &str) -> Result<ExecResult, String> {
        let start = std::time::Instant::now();
        let opener = conn.clone();
        conn.block_on(
            async move { execute_command_on_shared_connection(&opener, command, start).await },
        )
    }

    fn open_sftp_on_shared(conn: SharedSshConnection) -> Result<PooledSftpSession, String> {
        let opener = conn.clone();
        let sftp = conn.block_on(async move { opener.open_sftp_session().await })?;
        Ok(PooledSftpSession {
            sftp,
            lease: SharedSshChannelLease { connection: conn },
        })
    }

    fn open_terminal_on_shared(
        conn: SharedSshConnection,
        cols: u16,
        rows: u16,
    ) -> Result<PooledTerminalChannel, String> {
        let opener = conn.clone();
        let (read_half, write_half) = conn.block_on(async move {
            let (read_half, write_half) = opener.open_channel_split_async().await?;
            write_half
                .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
                .await
                .map_err(|e| {
                    eprintln!("[Terminal] PTY request failed: {}", e);
                    format!("pty request failed: {}", e)
                })?;
            write_half.request_shell(false).await.map_err(|e| {
                eprintln!("[Terminal] Shell request failed: {}", e);
                format!("shell failed: {}", e)
            })?;
            Ok::<_, String>((read_half, write_half))
        })?;

        Ok(PooledTerminalChannel {
            read_half,
            write_half,
            lease: SharedSshChannelLease { connection: conn },
        })
    }

    pub fn get_shared_connection(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
    ) -> Result<SharedSshConnection, String> {
        self.get_or_connect(host_id, config, timeout_secs)
    }

    fn host_lock(&self, host_id: &str) -> Arc<Mutex<()>> {
        self.connection_locks
            .entry(host_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn get_or_connect(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
    ) -> Result<SharedSshConnection, String> {
        if let Some(conn) = self.get_existing(host_id) {
            return Ok(conn);
        }

        let host_lock = self.host_lock(host_id);
        let _host_guard = host_lock.lock().map_err(|e| e.to_string())?;

        if let Some(conn) = self.get_existing(host_id) {
            return Ok(conn);
        }

        self.connect_and_store_locked(host_id, config, timeout_secs)
    }

    fn get_existing(&self, host_id: &str) -> Option<SharedSshConnection> {
        let entry = self.connections.get(host_id)?;
        if entry.connection.is_closed() {
            drop(entry);
            self.connections
                .remove_if(host_id, |_, e| e.connection.is_closed());
            eprintln!(
                "[SSH Registry] Dropping closed shared connection for {}",
                host_id
            );
            return None;
        }
        entry.touch();
        Some(entry.connection.clone())
    }

    fn reconnect_and_store(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
    ) -> Result<SharedSshConnection, String> {
        let host_lock = self.host_lock(host_id);
        let _host_guard = host_lock.lock().map_err(|e| e.to_string())?;

        if let Some(conn) = self.get_existing(host_id) {
            return Ok(conn);
        }

        self.connect_and_store_locked(host_id, config, timeout_secs)
    }

    fn connect_and_store_locked(
        &self,
        host_id: &str,
        config: &SshConfig,
        timeout_secs: u64,
    ) -> Result<SharedSshConnection, String> {
        self.log(
            "info",
            &format!(
                "主机 {} 正在连接 {}:{}...",
                host_id, config.host, config.port
            ),
        );

        match connect_internal_with_host_id(host_id, config, timeout_secs) {
            Ok(conn) => {
                let shared = conn.into_shared();

                if let Some(existing) = self.connections.get(host_id) {
                    if !existing.connection.is_closed() {
                        return Ok(existing.connection.clone());
                    }
                }

                self.connections
                    .insert(host_id.to_string(), ConnectionEntry::new(shared.clone()));

                self.log(
                    "success",
                    &format!("主机 {} ({}) 已连接", host_id, config.host),
                );

                Ok(shared)
            }
            Err(e) => {
                self.log("error", &format!("主机 {} 连接失败: {}", host_id, e));
                Err(e)
            }
        }
    }

    fn remove_if_same(&self, host_id: &str, conn: &SharedSshConnection) {
        self.connections
            .remove_if(host_id, |_, entry| entry.connection.same_transport(conn));
    }

    pub fn remove_connection(&self, host_id: &str) -> bool {
        self.connections.remove(host_id).is_some()
    }

    pub fn get_shared_connection_via_jump(
        &self,
        host_id: &str,
        target_config: &SshConfig,
        jump_configs: &[SshConfig],
        timeout_secs: u64,
    ) -> Result<SharedSshConnection, String> {
        if jump_configs.is_empty() {
            return self.get_or_connect(host_id, target_config, timeout_secs);
        }

        let pool_key = format!(
            "{}:jump:{}",
            host_id,
            jump_configs
                .iter()
                .map(|c| c.host.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        if let Some(conn) = self.get_existing(&pool_key) {
            return Ok(conn);
        }

        let host_lock = self.host_lock(&pool_key);
        let _host_guard = host_lock.lock().map_err(|e| e.to_string())?;

        if let Some(conn) = self.get_existing(&pool_key) {
            return Ok(conn);
        }

        let conn = connect_via_jump_chain(target_config, jump_configs, timeout_secs)?;
        let shared = conn.into_shared();

        self.connections
            .insert(pool_key, ConnectionEntry::new(shared.clone()));
        Ok(shared)
    }

    pub fn open_terminal_channel_via_jump(
        &self,
        host_id: &str,
        target_config: &SshConfig,
        jump_configs: &[SshConfig],
        timeout_secs: u64,
        cols: u16,
        rows: u16,
    ) -> Result<PooledTerminalChannel, String> {
        let conn = self.get_shared_connection_via_jump(
            host_id,
            target_config,
            jump_configs,
            timeout_secs,
        )?;
        match Self::open_terminal_on_shared(conn.clone(), cols, rows) {
            Ok(channel) => Ok(channel),
            Err(err) => {
                eprintln!(
                    "[SSH Registry] Failed to open terminal via jump for {}: {}; reconnecting once",
                    host_id, err
                );
                let pool_key = format!(
                    "{}:jump:{}",
                    host_id,
                    jump_configs
                        .iter()
                        .map(|c| c.host.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                self.connections.remove(&pool_key);
                let conn = self.get_shared_connection_via_jump(
                    host_id,
                    target_config,
                    jump_configs,
                    timeout_secs,
                )?;
                Self::open_terminal_on_shared(conn, cols, rows)
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_connection_state(&self, host_id: &str) -> Option<ConnectionState> {
        self.connections
            .get(host_id)
            .map(|e| *e.state.lock().unwrap_or_else(|e| e.into_inner()))
    }

    #[allow(dead_code)]
    pub fn mark_link_down(&self, host_id: &str) {
        if let Some(entry) = self.connections.get(host_id) {
            *entry.state.lock().unwrap_or_else(|e| e.into_inner()) = ConnectionState::LinkDown;
            self.emit_connection_status(host_id, ConnectionState::LinkDown);
        }
        // Cascade: mark all downstream connections that route through this host as LinkDown
        let downstream_keys: Vec<String> = self
            .connections
            .iter()
            .filter_map(|r| {
                let key = r.key();
                // Jump connection keys are formatted as "{host_id}:jump:{jump1,jump2,...}"
                if let Some(idx) = key.find(":jump:") {
                    let jump_part = &key[idx + 6..];
                    if jump_part.split(',').any(|j| j == host_id) {
                        return Some(key.clone());
                    }
                }
                None
            })
            .collect();
        for key in downstream_keys {
            if let Some(entry) = self.connections.get(&key) {
                *entry.state.lock().unwrap_or_else(|e| e.into_inner()) = ConnectionState::LinkDown;
                self.emit_connection_status(&key, ConnectionState::LinkDown);
            }
        }
    }
}
