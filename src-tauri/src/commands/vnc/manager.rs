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
    StartVncSessionRequest, VncAuthMethod, VncBridgeSession, VncKeyEventRequest,
    VncPointerEventRequest, VncSessionStarted, VncSessionStatus, VncSimpleRequest,
    DEFAULT_VNC_PORT,
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
        let auth_method = request.auth_method;
        let options = request.options.unwrap_or_default();
        super::append_vnc_diagnostic_log(
            &app,
            &format!(
                "bridge start requested sessionId={} host={} port={} usernameSet={} passwordSet={} authMethod={} shared={} viewOnly={} proxySet={}",
                session_id,
                host,
                port,
                username.is_some(),
                password.as_ref().is_some_and(|value| !value.is_empty()),
                auth_method,
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
                auth_method,
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
            auth_method,
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
    auth_method: VncAuthMethod,
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
                            auth_method,
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
    auth_method: VncAuthMethod,
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
                    log_rfb_clipboard_diagnostics(
                        &app,
                        &session_id,
                        &inspect_rfb_client_messages(&bytes),
                    );
                    tcp_writer
                        .write_all(&bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    client_metrics.record_client_to_server(bytes.len());
                }
                Message::Text(text) => {
                    let bytes = text.as_bytes();
                    log_rfb_clipboard_diagnostics(
                        &app,
                        &session_id,
                        &inspect_rfb_client_messages(bytes),
                    );
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
        let mut rfb_tracker =
            RfbServerHandshakeTracker::new(app.clone(), session_id.clone(), auth_method);
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
            log_rfb_clipboard_diagnostics(
                &app,
                &session_id,
                &inspect_rfb_server_clipboard_candidates(&outbound),
            );
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

const RFB_EXTENDED_CLIPBOARD_ENCODING: u32 = 0xc0a1_e5ce;
const RFB_CLIPBOARD_FORMAT_MASK: u32 = 0x0000_ffff;
const RFB_CLIPBOARD_ACTION_MASK: u32 = 0xff00_0000;
const RFB_CLIPBOARD_ACTION_CAPS: u32 = 1 << 24;
const RFB_CLIPBOARD_ACTION_REQUEST: u32 = 1 << 25;
const RFB_CLIPBOARD_ACTION_PEEK: u32 = 1 << 26;
const RFB_CLIPBOARD_ACTION_NOTIFY: u32 = 1 << 27;
const RFB_CLIPBOARD_ACTION_PROVIDE: u32 = 1 << 28;

#[derive(Debug, PartialEq, Eq)]
enum RfbClipboardDiagnostic {
    SetEncodings {
        count: u16,
        extended_clipboard: bool,
    },
    ClientCutText {
        extended: bool,
        bytes: usize,
        action: Option<String>,
        formats: Option<u32>,
    },
    ServerCutText {
        extended: bool,
        bytes: usize,
        action: Option<String>,
        formats: Option<u32>,
    },
}

fn inspect_rfb_client_messages(bytes: &[u8]) -> Vec<RfbClipboardDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            0 => offset += 20,
            2 => {
                if bytes.len() < offset + 4 {
                    break;
                }
                let count = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]);
                let message_len = 4 + usize::from(count) * 4;
                if bytes.len() < offset + message_len {
                    break;
                }
                let encodings = &bytes[offset + 4..offset + message_len];
                let extended_clipboard = encodings.chunks_exact(4).any(|chunk| {
                    u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                        == RFB_EXTENDED_CLIPBOARD_ENCODING
                });
                diagnostics.push(RfbClipboardDiagnostic::SetEncodings {
                    count,
                    extended_clipboard,
                });
                offset += message_len;
            }
            3 => offset += 10,
            4 => offset += 8,
            5 => offset += 6,
            6 => {
                let Some((diagnostic, message_len)) = inspect_cut_text_message(bytes, offset, true)
                else {
                    break;
                };
                diagnostics.push(diagnostic);
                offset += message_len;
            }
            _ => break,
        }
    }
    diagnostics
}

fn inspect_rfb_server_clipboard_candidates(bytes: &[u8]) -> Vec<RfbClipboardDiagnostic> {
    if bytes.first() != Some(&3) {
        return Vec::new();
    }
    inspect_cut_text_message(bytes, 0, false)
        .map(|(diagnostic, _)| vec![diagnostic])
        .unwrap_or_default()
}

fn inspect_cut_text_message(
    bytes: &[u8],
    offset: usize,
    client_to_server: bool,
) -> Option<(RfbClipboardDiagnostic, usize)> {
    if bytes.len() < offset + 8 {
        return None;
    }
    let length = i32::from_be_bytes([
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]);
    let data_len = length.unsigned_abs() as usize;
    let message_len = 8 + data_len;
    if bytes.len() < offset + message_len {
        return None;
    }
    let extended = length < 0;
    let (action, formats) = if extended && data_len >= 4 {
        let flags = u32::from_be_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]);
        (
            describe_extended_clipboard_actions(flags & RFB_CLIPBOARD_ACTION_MASK),
            Some(flags & RFB_CLIPBOARD_FORMAT_MASK),
        )
    } else {
        (None, None)
    };
    let diagnostic = if client_to_server {
        RfbClipboardDiagnostic::ClientCutText {
            extended,
            bytes: data_len,
            action,
            formats,
        }
    } else {
        RfbClipboardDiagnostic::ServerCutText {
            extended,
            bytes: data_len,
            action,
            formats,
        }
    };
    Some((diagnostic, message_len))
}

fn describe_extended_clipboard_actions(actions: u32) -> Option<String> {
    if actions == 0 {
        return None;
    }
    let mut labels = Vec::new();
    if actions & RFB_CLIPBOARD_ACTION_CAPS != 0 {
        labels.push("caps");
    }
    if actions & RFB_CLIPBOARD_ACTION_REQUEST != 0 {
        labels.push("request");
    }
    if actions & RFB_CLIPBOARD_ACTION_PEEK != 0 {
        labels.push("peek");
    }
    if actions & RFB_CLIPBOARD_ACTION_NOTIFY != 0 {
        labels.push("notify");
    }
    if actions & RFB_CLIPBOARD_ACTION_PROVIDE != 0 {
        labels.push("provide");
    }
    if labels.is_empty() {
        Some(format!("unknown({actions:#x})"))
    } else {
        Some(labels.join(","))
    }
}

fn log_rfb_clipboard_diagnostics(
    app: &AppHandle,
    session_id: &str,
    diagnostics: &[RfbClipboardDiagnostic],
) {
    for diagnostic in diagnostics {
        let message = match diagnostic {
            RfbClipboardDiagnostic::SetEncodings {
                count,
                extended_clipboard,
            } => format!(
                "rfb clipboard clientSetEncodings sessionId={} count={} extendedClipboard={}",
                session_id, count, extended_clipboard
            ),
            RfbClipboardDiagnostic::ClientCutText {
                extended,
                bytes,
                action,
                formats,
            } => format!(
                "rfb clipboard clientCutText sessionId={} extended={} bytes={} action={} formats={}",
                session_id,
                extended,
                bytes,
                action.as_deref().unwrap_or("standard"),
                formats
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "standard".to_string())
            ),
            RfbClipboardDiagnostic::ServerCutText {
                extended,
                bytes,
                action,
                formats,
            } => format!(
                "rfb clipboard serverCutTextCandidate sessionId={} extended={} bytes={} action={} formats={}",
                session_id,
                extended,
                bytes,
                action.as_deref().unwrap_or("standard"),
                formats
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "standard".to_string())
            ),
        };
        super::append_vnc_diagnostic_log(app, &message);
    }
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
    auth_method: VncAuthMethod,
    state: RfbHandshakeState,
    buffer: Vec<u8>,
}

impl RfbServerHandshakeTracker {
    fn new(app: AppHandle, session_id: String, auth_method: VncAuthMethod) -> Self {
        Self {
            app,
            session_id,
            auth_method,
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
                    let preferred_types = order_vnc_security_types(&server_types, self.auth_method);
                    let reordered = preferred_types != server_types;
                    message[1..].copy_from_slice(&preferred_types);
                    super::append_vnc_diagnostic_log(
                        &self.app,
                        &format!(
                            "rfb security types sessionId={} serverTypes={:?} forwardedTypes={:?} authMethod={} reordered={}",
                            self.session_id,
                            server_types,
                            preferred_types,
                            self.auth_method,
                            reordered
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

fn order_vnc_security_types(types: &[u8], auth_method: VncAuthMethod) -> Vec<u8> {
    match auth_method {
        VncAuthMethod::VncAuth => order_security_type_before(types, 2, 30),
        VncAuthMethod::AppleRemoteDesktop => order_security_type_before(types, 30, 2),
    }
}

fn order_security_type_before(types: &[u8], preferred_type: u8, secondary_type: u8) -> Vec<u8> {
    let mut preferred = types.to_vec();
    let Some(preferred_index) = preferred.iter().position(|value| *value == preferred_type) else {
        return preferred;
    };
    let Some(secondary_index) = preferred.iter().position(|value| *value == secondary_type) else {
        return preferred;
    };
    if preferred_index < secondary_index {
        return preferred;
    }
    let security_type = preferred.remove(preferred_index);
    preferred.insert(secondary_index, security_type);
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
    fn prefers_standard_vnc_auth_before_apple_remote_desktop_by_default() {
        assert_eq!(
            order_vnc_security_types(&[30, 2], VncAuthMethod::VncAuth),
            vec![2, 30]
        );
        assert_eq!(
            order_vnc_security_types(&[1, 30, 2], VncAuthMethod::VncAuth),
            vec![1, 2, 30]
        );
        assert_eq!(
            order_vnc_security_types(&[2, 30], VncAuthMethod::VncAuth),
            vec![2, 30]
        );
        assert_eq!(
            order_vnc_security_types(&[30], VncAuthMethod::VncAuth),
            vec![30]
        );
    }

    #[test]
    fn can_prefer_apple_remote_desktop_when_selected() {
        assert_eq!(
            order_vnc_security_types(&[2, 30], VncAuthMethod::AppleRemoteDesktop),
            vec![30, 2]
        );
        assert_eq!(
            order_vnc_security_types(&[1, 2, 30], VncAuthMethod::AppleRemoteDesktop),
            vec![1, 30, 2]
        );
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

    #[test]
    fn detects_client_clipboard_messages() {
        let standard = [6, 0, 0, 0, 0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o'];
        let extended = [6, 0, 0, 0, 255, 255, 255, 252, 8, 0, 0, 1];

        assert_eq!(
            inspect_rfb_client_messages(&standard),
            vec![RfbClipboardDiagnostic::ClientCutText {
                extended: false,
                bytes: 5,
                action: None,
                formats: None,
            }]
        );
        assert_eq!(
            inspect_rfb_client_messages(&extended),
            vec![RfbClipboardDiagnostic::ClientCutText {
                extended: true,
                bytes: 4,
                action: Some("notify".to_string()),
                formats: Some(1),
            }]
        );
    }

    #[test]
    fn detects_extended_clipboard_encoding_offer() {
        let message = [2, 0, 0, 2, 0, 0, 0, 16, 0xc0, 0xa1, 0xe5, 0xce];

        assert_eq!(
            inspect_rfb_client_messages(&message),
            vec![RfbClipboardDiagnostic::SetEncodings {
                count: 2,
                extended_clipboard: true,
            }]
        );
    }

    #[test]
    fn detects_server_clipboard_frame_candidates() {
        let standard = [3, 0, 0, 0, 0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o'];
        let extended = [3, 0, 0, 0, 255, 255, 255, 252, 1, 0, 0, 1];

        assert_eq!(
            inspect_rfb_server_clipboard_candidates(&standard),
            vec![RfbClipboardDiagnostic::ServerCutText {
                extended: false,
                bytes: 5,
                action: None,
                formats: None,
            }]
        );
        assert_eq!(
            inspect_rfb_server_clipboard_candidates(&extended),
            vec![RfbClipboardDiagnostic::ServerCutText {
                extended: true,
                bytes: 4,
                action: Some("caps".to_string()),
                formats: Some(1),
            }]
        );
    }
}
