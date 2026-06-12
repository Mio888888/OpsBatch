use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use tauri::AppHandle;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::Runtime,
    sync::oneshot,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use super::types::{
    StartVncSessionRequest, VncBridgeSession, VncKeyEventRequest, VncPointerEventRequest,
    VncSessionStarted, VncSessionStatus, VncSimpleRequest, DEFAULT_VNC_PORT,
};
use crate::ssh;

const BRIDGE_METRICS_INTERVAL: Duration = Duration::from_secs(2);
const VNC_BRIDGE_READ_BUFFER_BYTES: usize = 128 * 1024;

pub struct VncSessionManager {
    runtime: Runtime,
    pub(super) sessions: Arc<Mutex<HashMap<String, VncBridgeSession>>>,
}

impl VncSessionManager {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new().expect("VNC bridge runtime initializes"),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_session(
        &self,
        app: AppHandle,
        request: StartVncSessionRequest,
        proxy: Option<ssh::ProxySettings>,
    ) -> Result<VncSessionStarted, String> {
        let session_id = required_id(request.session_id)?;
        let host = required_field("VNC host", request.host)?;
        let port = request.port.unwrap_or(DEFAULT_VNC_PORT);
        if port == 0 {
            return Err("VNC port must be between 1 and 65535".to_string());
        }
        {
            let sessions = self.lock_sessions()?;
            if sessions.contains_key(&session_id) {
                return Err(format!("VNC session '{session_id}' is already running"));
            }
        }

        let username = request
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let password = request.password.filter(|value| !value.is_empty());
        let options = request.options.unwrap_or_default();
        super::append_vnc_diagnostic_log(
            &app,
            &format!(
                "bridge start requested sessionId={} host={} port={} usernameSet={} passwordSet={} shared={} viewOnly={} proxySet={}",
                session_id,
                host,
                port,
                username.is_some(),
                password.as_ref().is_some_and(|value| !value.is_empty()),
                options.shared_session,
                options.view_only,
                proxy.is_some(),
            ),
        );

        let listener = self
            .runtime
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .map_err(|error| format!("failed to bind VNC WebSocket bridge: {error}"))?;
        let local_port = listener
            .local_addr()
            .map_err(|error| format!("failed to read VNC bridge address: {error}"))?
            .port();
        let websocket_url = bridge_websocket_url(local_port, &session_id);
        let (stop_tx, stop_rx) = oneshot::channel();
        let sessions = Arc::clone(&self.sessions);
        let task_session_id = session_id.clone();
        let task_host = host.clone();
        let task_app = app.clone();
        let task = self.runtime.spawn(async move {
            run_bridge_listener(
                task_app.clone(),
                task_session_id.clone(),
                task_host,
                port,
                proxy,
                listener,
                stop_rx,
            )
            .await;
            if let Ok(mut sessions) = sessions.lock() {
                sessions.remove(&task_session_id);
            }
        });

        let mut sessions = self.lock_sessions()?;
        sessions.insert(
            session_id.clone(),
            VncBridgeSession {
                stop: Some(stop_tx),
                task,
                connected: true,
                view_only: options.view_only,
            },
        );
        super::append_vnc_diagnostic_log(
            &app,
            &format!(
                "bridge listening sessionId={} host={} port={} websocketUrl={} activeSessions={}",
                session_id,
                host,
                port,
                websocket_url,
                sessions.len(),
            ),
        );

        Ok(VncSessionStarted {
            session_id,
            host,
            port,
            websocket_url,
            username,
            password,
            shared: options.shared_session,
            view_only: options.view_only,
        })
    }

    pub fn pointer_event(&self, request: VncPointerEventRequest) -> Result<(), String> {
        let _ = (request.x, request.y, request.button_mask);
        self.ensure_input_allowed(&request.session_id)
    }

    pub fn key_event(&self, request: VncKeyEventRequest) -> Result<(), String> {
        let _ = (request.key, request.down);
        self.ensure_input_allowed(&request.session_id)
    }

    pub fn send_ctrl_alt_delete(&self, request: VncSimpleRequest) -> Result<(), String> {
        self.ensure_input_allowed(&request.session_id)
    }

    pub fn refresh(&self, request: VncSimpleRequest) -> Result<(), String> {
        self.ensure_session_exists(&request.session_id).map(|_| ())
    }

    pub fn close_session(&self, request: VncSimpleRequest) -> Result<(), String> {
        let removed = {
            let mut sessions = self.lock_sessions()?;
            sessions.remove(&request.session_id)
        };
        if let Some(mut session) = removed {
            if let Some(stop) = session.stop.take() {
                let _ = stop.send(());
            }
            session.task.abort();
        }
        Ok(())
    }

    pub fn session_status(&self, request: VncSimpleRequest) -> Result<VncSessionStatus, String> {
        let sessions = self.lock_sessions()?;
        let connected = sessions
            .get(&request.session_id)
            .map(|session| session.connected)
            .unwrap_or(false);
        Ok(VncSessionStatus {
            session_id: request.session_id,
            connected,
        })
    }

    pub fn ensure_session_exists(&self, session_id: &str) -> Result<bool, String> {
        let sessions = self.lock_sessions()?;
        sessions
            .get(session_id)
            .map(|session| session.view_only)
            .ok_or_else(|| format!("VNC session '{session_id}' was not found"))
    }

    fn ensure_input_allowed(&self, session_id: &str) -> Result<(), String> {
        let view_only = self.ensure_session_exists(session_id)?;
        if view_only {
            return Err(format!("VNC session '{session_id}' is view-only"));
        }
        Ok(())
    }

    fn lock_sessions(&self) -> Result<MutexGuard<'_, HashMap<String, VncBridgeSession>>, String> {
        self.sessions
            .lock()
            .map_err(|_| "VNC session lock is poisoned".to_string())
    }
}

impl Default for VncSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn run_bridge_listener(
    app: AppHandle,
    session_id: String,
    host: String,
    port: u16,
    proxy: Option<ssh::ProxySettings>,
    listener: TcpListener,
    mut stop_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((websocket_stream, peer_addr)) => {
                        super::append_vnc_diagnostic_log(
                            &app,
                            &format!("bridge websocket accepted sessionId={session_id} peer={peer_addr}"),
                        );
                        if let Err(error) = bridge_connection(
                            app.clone(),
                            session_id.clone(),
                            host.clone(),
                            port,
                            proxy.clone(),
                            websocket_stream,
                        ).await {
                            super::append_vnc_diagnostic_log(
                                &app,
                                &format!("bridge connection ended sessionId={session_id} error={error}"),
                            );
                        }
                    }
                    Err(error) => {
                        super::append_vnc_diagnostic_log(
                            &app,
                            &format!("bridge accept failed sessionId={session_id} error={error}"),
                        );
                        break;
                    }
                }
            }
        }
    }
    super::append_vnc_diagnostic_log(&app, &format!("bridge closed sessionId={session_id}"));
}

async fn bridge_connection(
    app: AppHandle,
    session_id: String,
    host: String,
    port: u16,
    proxy: Option<ssh::ProxySettings>,
    websocket_stream: TcpStream,
) -> Result<(), String> {
    websocket_stream
        .set_nodelay(true)
        .map_err(|error| format!("failed to enable VNC WebSocket TCP_NODELAY: {error}"))?;
    let websocket = accept_async(websocket_stream)
        .await
        .map_err(|error| format!("websocket handshake failed: {error}"))?;
    super::append_vnc_diagnostic_log(
        &app,
        &format!("bridge tcp connect start sessionId={session_id} host={host} port={port}"),
    );
    let std_tcp = tokio::task::spawn_blocking({
        let host = host.clone();
        move || ssh::connect_tcp_stream(&host, port, proxy, 12)
    })
    .await
    .map_err(|error| format!("VNC TCP bridge thread failed: {error}"))?
    .map_err(|error| {
        super::append_vnc_diagnostic_log(
            &app,
            &format!(
                "bridge tcp connect failed sessionId={} host={} port={} error={}",
                session_id, host, port, error
            ),
        );
        format!("failed to connect to VNC server {host}:{port}: {error}")
    })?;
    std_tcp
        .set_nodelay(true)
        .map_err(|error| format!("failed to enable VNC TCP_NODELAY: {error}"))?;
    std_tcp
        .set_nonblocking(true)
        .map_err(|error| format!("failed to make VNC TCP stream nonblocking: {error}"))?;
    let tcp = TcpStream::from_std(std_tcp)
        .map_err(|error| format!("failed to adopt VNC TCP stream: {error}"))?;
    super::append_vnc_diagnostic_log(
        &app,
        &format!(
            "bridge tcp connected sessionId={session_id} host={host} port={port} tcpNoDelay=true"
        ),
    );

    let (mut ws_sink, mut ws_stream) = websocket.split();
    let (mut tcp_reader, mut tcp_writer) = tcp.into_split();
    let mut client_metrics = BridgeTransferMetrics::new(app.clone(), session_id.clone());
    let mut server_metrics = BridgeTransferMetrics::new(app.clone(), session_id.clone());
    let websocket_to_tcp = async {
        while let Some(message) = ws_stream.next().await {
            match message.map_err(|error| error.to_string())? {
                Message::Binary(bytes) => {
                    tcp_writer
                        .write_all(&bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    client_metrics.record_client_to_server(bytes.len());
                }
                Message::Text(text) => {
                    let bytes = text.as_bytes();
                    tcp_writer
                        .write_all(bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    client_metrics.record_client_to_server(bytes.len());
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok::<(), String>(())
    };
    let tcp_to_websocket = async {
        let mut rfb_tracker = RfbServerHandshakeTracker::new(app.clone(), session_id.clone());
        let mut buffer = vec![0_u8; VNC_BRIDGE_READ_BUFFER_BYTES];
        loop {
            let read = tcp_reader
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            let outbound = rfb_tracker.process(&buffer[..read]);
            if outbound.is_empty() {
                continue;
            }
            let outbound_len = outbound.len();
            ws_sink
                .send(Message::Binary(outbound))
                .await
                .map_err(|error| error.to_string())?;
            server_metrics.record_server_to_client(outbound_len);
        }
        Ok::<(), String>(())
    };

    tokio::select! {
        result = websocket_to_tcp => result?,
        result = tcp_to_websocket => result?,
    }
    Ok(())
}

fn bridge_websocket_url(port: u16, session_id: &str) -> String {
    format!("ws://127.0.0.1:{}/vnc/{}", port, session_id)
}

struct BridgeTransferMetrics {
    app: AppHandle,
    session_id: String,
    enabled: bool,
    last_log: Instant,
    client_to_server_bytes: usize,
    server_to_client_bytes: usize,
    client_to_server_messages: usize,
    server_to_client_messages: usize,
}

impl BridgeTransferMetrics {
    fn new(app: AppHandle, session_id: String) -> Self {
        Self {
            app,
            session_id,
            enabled: vnc_bridge_metrics_enabled(),
            last_log: Instant::now(),
            client_to_server_bytes: 0,
            server_to_client_bytes: 0,
            client_to_server_messages: 0,
            server_to_client_messages: 0,
        }
    }

    fn record_client_to_server(&mut self, bytes: usize) {
        if !self.enabled {
            return;
        }
        self.client_to_server_bytes += bytes;
        self.client_to_server_messages += 1;
        self.maybe_log();
    }

    fn record_server_to_client(&mut self, bytes: usize) {
        if !self.enabled {
            return;
        }
        self.server_to_client_bytes += bytes;
        self.server_to_client_messages += 1;
        self.maybe_log();
    }

    fn maybe_log(&mut self) {
        let elapsed = self.last_log.elapsed();
        if elapsed < BRIDGE_METRICS_INTERVAL {
            return;
        }
        super::append_vnc_diagnostic_log(
            &self.app,
            &format!(
                "bridge metrics sessionId={} windowMs={} clientToServerBytes={} serverToClientBytes={} clientToServerMessages={} serverToClientMessages={}",
                self.session_id,
                elapsed.as_millis(),
                self.client_to_server_bytes,
                self.server_to_client_bytes,
                self.client_to_server_messages,
                self.server_to_client_messages,
            ),
        );
        self.last_log = Instant::now();
        self.client_to_server_bytes = 0;
        self.server_to_client_bytes = 0;
        self.client_to_server_messages = 0;
        self.server_to_client_messages = 0;
    }
}

fn vnc_bridge_metrics_enabled() -> bool {
    std::env::var("OPSBATCH_VNC_BRIDGE_METRICS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

enum RfbHandshakeState {
    ServerVersion,
    SecurityTypes,
    SecurityScheme,
    Done,
}

struct RfbServerHandshakeTracker {
    app: AppHandle,
    session_id: String,
    state: RfbHandshakeState,
    buffer: Vec<u8>,
}

impl RfbServerHandshakeTracker {
    fn new(app: AppHandle, session_id: String) -> Self {
        Self {
            app,
            session_id,
            state: RfbHandshakeState::ServerVersion,
            buffer: Vec::new(),
        }
    }

    fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        if matches!(self.state, RfbHandshakeState::Done) && self.buffer.is_empty() {
            return bytes.to_vec();
        }
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::with_capacity(bytes.len());

        loop {
            match self.state {
                RfbHandshakeState::ServerVersion => {
                    if self.buffer.len() < 12 {
                        break;
                    }
                    let version = self.buffer.drain(..12).collect::<Vec<_>>();
                    let (server_version, uses_security_types) = parse_rfb_version(&version);
                    super::append_vnc_diagnostic_log(
                        &self.app,
                        &format!(
                            "rfb protocol version sessionId={} serverVersion={} securityList={}",
                            self.session_id, server_version, uses_security_types
                        ),
                    );
                    output.extend_from_slice(&version);
                    self.state = if uses_security_types {
                        RfbHandshakeState::SecurityTypes
                    } else {
                        RfbHandshakeState::SecurityScheme
                    };
                }
                RfbHandshakeState::SecurityTypes => {
                    if self.buffer.is_empty() {
                        break;
                    }
                    let count = usize::from(self.buffer[0]);
                    if count == 0 {
                        output.push(self.buffer.remove(0));
                        super::append_vnc_diagnostic_log(
                            &self.app,
                            &format!(
                                "rfb security types sessionId={} serverTypes=[]",
                                self.session_id
                            ),
                        );
                        self.state = RfbHandshakeState::Done;
                        continue;
                    }
                    if self.buffer.len() < count + 1 {
                        break;
                    }
                    let mut message = self.buffer.drain(..count + 1).collect::<Vec<_>>();
                    let server_types = message[1..].to_vec();
                    let preferred_types = prefer_vnc_auth_security_types(&server_types);
                    let reordered = preferred_types != server_types;
                    message[1..].copy_from_slice(&preferred_types);
                    super::append_vnc_diagnostic_log(
                        &self.app,
                        &format!(
                            "rfb security types sessionId={} serverTypes={:?} forwardedTypes={:?} preferVncAuth={}",
                            self.session_id, server_types, preferred_types, reordered
                        ),
                    );
                    output.extend_from_slice(&message);
                    self.state = RfbHandshakeState::Done;
                }
                RfbHandshakeState::SecurityScheme => {
                    if self.buffer.len() < 4 {
                        break;
                    }
                    let scheme = self.buffer.drain(..4).collect::<Vec<_>>();
                    let security_type =
                        u32::from_be_bytes([scheme[0], scheme[1], scheme[2], scheme[3]]);
                    super::append_vnc_diagnostic_log(
                        &self.app,
                        &format!(
                            "rfb security scheme sessionId={} serverType={}",
                            self.session_id, security_type
                        ),
                    );
                    output.extend_from_slice(&scheme);
                    self.state = RfbHandshakeState::Done;
                }
                RfbHandshakeState::Done => {
                    output.extend_from_slice(&self.buffer);
                    self.buffer.clear();
                    break;
                }
            }
        }

        output
    }
}

fn parse_rfb_version(bytes: &[u8]) -> (String, bool) {
    let banner = std::str::from_utf8(bytes).unwrap_or("").trim_end();
    let version = banner
        .strip_prefix("RFB ")
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let uses_security_types = matches!(
        version.as_str(),
        "003.007" | "003.008" | "003.889" | "004.000" | "004.001" | "005.000"
    );
    (version, uses_security_types)
}

fn prefer_vnc_auth_security_types(types: &[u8]) -> Vec<u8> {
    let mut preferred = types.to_vec();
    let Some(vnc_auth_index) = preferred.iter().position(|value| *value == 2) else {
        return preferred;
    };
    let Some(ard_index) = preferred.iter().position(|value| *value == 30) else {
        return preferred;
    };
    if vnc_auth_index < ard_index {
        return preferred;
    }
    let vnc_auth = preferred.remove(vnc_auth_index);
    preferred.insert(ard_index, vnc_auth);
    preferred
}

fn required_id(value: String) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("VNC session id is required".to_string());
    }
    if trimmed.len() > 96 {
        return Err("VNC session id must be 96 characters or fewer".to_string());
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("VNC session id may only contain letters, digits, '-' or '_'".to_string());
    }
    Ok(trimmed.to_string())
}

fn required_field(label: &str, value: String) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_vnc_session_ids() {
        assert_eq!(
            required_id("vnc-session_1".to_string()).as_deref(),
            Ok("vnc-session_1")
        );
        assert!(required_id("bad/session".to_string()).is_err());
    }

    #[test]
    fn bridge_url_is_bound_to_localhost() {
        assert_eq!(
            bridge_websocket_url(5901, "vnc-session_1"),
            "ws://127.0.0.1:5901/vnc/vnc-session_1"
        );
    }

    #[test]
    fn prefers_vnc_auth_before_apple_remote_desktop() {
        assert_eq!(prefer_vnc_auth_security_types(&[30, 2]), vec![2, 30]);
        assert_eq!(prefer_vnc_auth_security_types(&[1, 30, 2]), vec![1, 2, 30]);
        assert_eq!(prefer_vnc_auth_security_types(&[2, 30]), vec![2, 30]);
        assert_eq!(prefer_vnc_auth_security_types(&[30]), vec![30]);
    }

    #[test]
    fn parses_apple_rfb_version_as_security_type_list() {
        assert_eq!(
            parse_rfb_version(b"RFB 003.889\n"),
            ("003.889".to_string(), true)
        );
        assert_eq!(
            parse_rfb_version(b"RFB 003.003\n"),
            ("003.003".to_string(), false)
        );
    }
}
