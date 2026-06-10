use std::net::SocketAddr;
use std::time::Duration;

use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::{ClientConnector, Config, ServerName};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::Database as InputDatabase;
use ironrdp::rdpsnd::client::Rdpsnd;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite as _, MovableTokioFramed, TokioFramed};
use tauri::ipc::{Channel, Response};
use tauri::Emitter;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use super::audio::PcmAudioHandler;
use super::clipboard::{
    text_clipboard_formats, ClipboardAction, ClipboardBridge, TextClipboardBackend,
};
use super::config::build_ironrdp_config;
use super::frame::{build_frame_message, FramePacer, QueuedFrame};
use super::input::input_operations;
use super::types::{
    RdpConnectResponse, RdpConnectionOptions, RdpCredentials, RdpMetricsPayload, RdpStatusPayload,
    RectRegion,
};
use super::{RdpSessionCommand, RDP_CONNECT_TIMEOUT};

type RdpFramed = MovableTokioFramed<ironrdp_tls::TlsStream<TcpStream>>;
const RDP_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const RDP_METRICS_INTERVAL: Duration = Duration::from_secs(1);
const RDP_CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(300);

struct ConnectedRdpSession {
    framed: RdpFramed,
    active_stage: ActiveStage,
    image: DecodedImage,
    input: InputDatabase,
    clipboard: Option<ClipboardBridge>,
    width: u16,
    height: u16,
}

#[derive(Debug)]
struct RdpMetrics {
    window_started_at: tokio::time::Instant,
    server_updates: u32,
    sent_frames: u32,
    coalesced_updates: u32,
    sent_bytes: u64,
}

impl RdpMetrics {
    fn new(now: tokio::time::Instant) -> Self {
        Self {
            window_started_at: now,
            server_updates: 0,
            sent_frames: 0,
            coalesced_updates: 0,
            sent_bytes: 0,
        }
    }

    fn record_server_update(&mut self) {
        self.server_updates = self.server_updates.saturating_add(1);
    }

    fn record_sent_frame(&mut self, bytes: usize, coalesced_updates: u32) {
        self.sent_frames = self.sent_frames.saturating_add(1);
        self.coalesced_updates = self.coalesced_updates.saturating_add(coalesced_updates);
        self.sent_bytes = self.sent_bytes.saturating_add(bytes as u64);
    }

    fn emit_if_due(&mut self, app: &tauri::AppHandle, session_id: &str, now: tokio::time::Instant) {
        let elapsed = now.duration_since(self.window_started_at);
        if elapsed < RDP_METRICS_INTERVAL {
            return;
        }

        let elapsed_secs = elapsed.as_secs_f64().max(0.001);
        let payload = RdpMetricsPayload {
            session_id: session_id.to_string(),
            server_updates_per_second: (f64::from(self.server_updates) / elapsed_secs).round()
                as u32,
            sent_frames_per_second: (f64::from(self.sent_frames) / elapsed_secs).round() as u32,
            coalesced_updates_per_second: (f64::from(self.coalesced_updates) / elapsed_secs).round()
                as u32,
            sent_mbytes_per_second: self.sent_bytes as f64 / elapsed_secs / 1_048_576.0,
        };
        let _ = app.emit(&format!("rdp-metrics-{}", session_id), payload);

        self.window_started_at = now;
        self.server_updates = 0;
        self.sent_frames = 0;
        self.coalesced_updates = 0;
        self.sent_bytes = 0;
    }
}

pub(super) async fn run_rdp_session(
    app: tauri::AppHandle,
    options: RdpConnectionOptions,
    credentials: RdpCredentials,
    command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
    ready: oneshot::Sender<Result<RdpConnectResponse, String>>,
    frame_channel: Channel<Response>,
) {
    let session_id = options.session_id.clone();
    let mut ready = Some(ready);
    emit_status(&app, &session_id, "connecting", None);

    let mut session = match connect_rdp_session(&options, &credentials).await {
        Ok(session) => session,
        Err(error) => {
            emit_status(&app, &session_id, "error", Some(error.clone()));
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(error));
            }
            return;
        }
    };

    let response = RdpConnectResponse {
        session_id: session_id.clone(),
        host_id: options.host_id,
        width: session.width,
        height: session.height,
    };
    if let Some(tx) = ready.take() {
        let _ = tx.send(Ok(response));
    }
    emit_status(&app, &session_id, "connected", None);

    if let Err(error) =
        run_active_session(&app, &session_id, &mut session, command_rx, frame_channel).await
    {
        emit_status(&app, &session_id, "error", Some(error));
        return;
    }

    emit_status(&app, &session_id, "disconnected", None);
}

async fn connect_rdp_session(
    options: &RdpConnectionOptions,
    credentials: &RdpCredentials,
) -> Result<ConnectedRdpSession, String> {
    let addr = format!("{}:{}", options.host, options.port);
    let tcp = tokio::time::timeout(RDP_CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| format!("RDP TCP 连接超时: {addr}"))?
        .map_err(|e| format!("RDP TCP 连接失败 {addr}: {e}"))?;
    tcp.set_nodelay(true)
        .map_err(|e| format!("设置 RDP TCP_NODELAY 失败: {e}"))?;

    let client_addr = tcp
        .local_addr()
        .map_err(|e| format!("获取本地 RDP 地址失败: {e}"))?;
    let config = build_ironrdp_config(options, credentials)?;
    let clipboard = options.enable_clipboard.then(ClipboardBridge::new);
    let mut framed = TokioFramed::new(tcp);
    let mut connector =
        build_client_connector_with_clipboard(config, client_addr, options, clipboard.clone())?;

    let should_upgrade = tokio::time::timeout(
        RDP_CONNECT_TIMEOUT,
        ironrdp_tokio::connect_begin(&mut framed, &mut connector),
    )
    .await
    .map_err(|_| "RDP 协议协商超时".to_string())?
    .map_err(|e| format!("RDP 协议协商失败: {}", e.report()))?;

    let stream = framed.into_inner_no_leftover();
    let (tls_stream, tls_cert) = tokio::time::timeout(
        RDP_CONNECT_TIMEOUT,
        ironrdp_tls::upgrade(stream, &options.host),
    )
    .await
    .map_err(|_| "RDP TLS 握手超时".to_string())?
    .map_err(|e| format!("RDP TLS 握手失败: {e}"))?;

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
        .ok_or_else(|| "RDP TLS 证书缺少 server public key".to_string())?
        .to_vec();
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = MovableTokioFramed::new(tls_stream);
    let mut network_client = ReqwestNetworkClient::new();
    let connection_result = tokio::time::timeout(
        RDP_CONNECT_TIMEOUT,
        ironrdp_tokio::connect_finalize(
            upgraded,
            connector,
            &mut framed,
            &mut network_client,
            ServerName::new(options.host.clone()),
            server_public_key,
            None,
        ),
    )
    .await
    .map_err(|_| "RDP NLA/CredSSP 握手超时".to_string())?
    .map_err(|e| format!("RDP NLA/CredSSP 握手失败: {}", e.report()))?;

    let width = connection_result.desktop_size.width;
    let height = connection_result.desktop_size.height;
    let active_stage = ActiveStage::new(connection_result);
    let image = DecodedImage::new(PixelFormat::RgbA32, width, height);

    Ok(ConnectedRdpSession {
        framed,
        active_stage,
        image,
        input: InputDatabase::new(),
        clipboard,
        width,
        height,
    })
}

#[cfg(test)]
pub(super) fn build_client_connector(
    config: Config,
    client_addr: SocketAddr,
    options: &RdpConnectionOptions,
) -> Result<ClientConnector, String> {
    build_client_connector_with_clipboard(config, client_addr, options, None)
}

fn build_client_connector_with_clipboard(
    config: Config,
    client_addr: SocketAddr,
    options: &RdpConnectionOptions,
    clipboard: Option<ClipboardBridge>,
) -> Result<ClientConnector, String> {
    let mut connector = ClientConnector::new(config, client_addr);
    if options.enable_clipboard {
        let bridge = clipboard.unwrap_or_else(ClipboardBridge::new);
        connector.attach_static_channel(CliprdrClient::new(Box::new(TextClipboardBackend::new(
            bridge,
        ))));
    }
    if options.enable_audio {
        connector.attach_static_channel(Rdpsnd::new(Box::new(PcmAudioHandler::new())));
    }
    Ok(connector)
}

#[cfg(test)]
pub(super) fn rdp_static_channel_names(connector: &ClientConnector) -> Vec<String> {
    let mut names = connector
        .static_channels
        .values()
        .filter_map(|channel| channel.channel_name().as_str().map(str::to_string))
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn run_active_session(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    mut command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
    frame_channel: Channel<Response>,
) -> Result<(), String> {
    let mut frame_pacer = FramePacer::new(RDP_FRAME_INTERVAL);
    let mut metrics = RdpMetrics::new(tokio::time::Instant::now());
    let mut clipboard_interval = tokio::time::interval(RDP_CLIPBOARD_POLL_INTERVAL);
    clipboard_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let frame_deadline = frame_pacer.next_deadline();
        tokio::select! {
            command = command_rx.recv() => {
                match command {
                    Some(RdpSessionCommand::Input(event)) => {
                        if !process_input_event(app, session_id, session, &frame_channel, &mut frame_pacer, &mut metrics, event).await? {
                            return Ok(());
                        }
                    }
                    Some(RdpSessionCommand::Disconnect) | None => {
                        flush_pending_graphics_update(&frame_channel, session, &mut frame_pacer, &mut metrics)?;
                        return Ok(());
                    }
                }
            }
            frame = session.framed.read_pdu() => {
                let (action, payload) = frame.map_err(|e| format!("读取 RDP 帧失败: {e}"))?;
                let outputs = session
                    .active_stage
                    .process(&mut session.image, action, &payload)
                    .map_err(|e| format!("处理 RDP 帧失败: {e}"))?;
                if !handle_stage_outputs(app, session_id, session, &frame_channel, &mut frame_pacer, &mut metrics, outputs).await? {
                    return Ok(());
                }
                flush_clipboard_actions(session).await?;
            }
            _ = sleep_until_deadline(frame_deadline), if frame_deadline.is_some() => {
                flush_due_graphics_update(&frame_channel, session, &mut frame_pacer, &mut metrics)?;
            }
            _ = clipboard_interval.tick(), if session.clipboard.is_some() => {
                poll_local_clipboard(session).await?;
            }
        }
        metrics.emit_if_due(app, session_id, tokio::time::Instant::now());
    }
}

async fn sleep_until_deadline(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    }
}

async fn poll_local_clipboard(session: &mut ConnectedRdpSession) -> Result<(), String> {
    if let Some(clipboard) = session.clipboard.clone() {
        clipboard.poll_local_clipboard();
    }
    flush_clipboard_actions(session).await
}

async fn flush_clipboard_actions(session: &mut ConnectedRdpSession) -> Result<(), String> {
    let Some(clipboard) = session.clipboard.as_ref() else {
        return Ok(());
    };

    for action in clipboard.drain_actions() {
        let messages = {
            let cliprdr = session
                .active_stage
                .get_svc_processor_mut::<CliprdrClient>()
                .ok_or_else(|| "RDP 剪贴板通道不可用".to_string())?;
            match action {
                ClipboardAction::AdvertiseText(enabled) => cliprdr
                    .initiate_copy(&text_clipboard_formats(enabled))
                    .map_err(|e| format!("生成 RDP 剪贴板格式列表失败: {e}"))?,
                ClipboardAction::RequestRemoteText(format) => cliprdr
                    .initiate_paste(format)
                    .map_err(|e| format!("生成 RDP 剪贴板粘贴请求失败: {e}"))?,
                ClipboardAction::SendTextResponse(response) => cliprdr
                    .submit_format_data(response)
                    .map_err(|e| format!("生成 RDP 剪贴板文本响应失败: {e}"))?,
            }
        };

        let frame = session
            .active_stage
            .process_svc_processor_messages(messages)
            .map_err(|e| format!("编码 RDP 剪贴板帧失败: {e}"))?;
        session
            .framed
            .write_all(&frame)
            .await
            .map_err(|e| format!("发送 RDP 剪贴板帧失败: {e}"))?;
    }

    Ok(())
}

async fn process_input_event(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    frame_channel: &Channel<Response>,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
    event: super::types::RdpInputEvent,
) -> Result<bool, String> {
    let operations = input_operations(event, session.width, session.height)?;
    let events = session.input.apply(operations);
    let outputs = session
        .active_stage
        .process_fastpath_input(&mut session.image, events.as_slice())
        .map_err(|e| format!("编码 RDP 输入失败: {e}"))?;
    handle_stage_outputs(
        app,
        session_id,
        session,
        frame_channel,
        frame_pacer,
        metrics,
        outputs,
    )
    .await
}

async fn handle_stage_outputs(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    frame_channel: &Channel<Response>,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
    outputs: Vec<ActiveStageOutput>,
) -> Result<bool, String> {
    let mut pending_region: Option<RectRegion> = None;

    for output in outputs {
        match output {
            ActiveStageOutput::ResponseFrame(frame) => {
                queue_graphics_update(
                    frame_channel,
                    session,
                    frame_pacer,
                    metrics,
                    pending_region.take(),
                )?;
                session
                    .framed
                    .write_all(&frame)
                    .await
                    .map_err(|e| format!("发送 RDP 响应帧失败: {e}"))?;
            }
            ActiveStageOutput::GraphicsUpdate(rect) => {
                metrics.record_server_update();
                let region = RectRegion::from_inclusive(rect);
                pending_region = Some(match pending_region {
                    Some(existing) => existing.union(region),
                    None => region,
                });
            }
            ActiveStageOutput::Terminate(reason) => {
                queue_graphics_update(
                    frame_channel,
                    session,
                    frame_pacer,
                    metrics,
                    pending_region.take(),
                )?;
                flush_pending_graphics_update(frame_channel, session, frame_pacer, metrics)?;
                emit_status(
                    app,
                    session_id,
                    "terminated",
                    Some(disconnect_reason(reason)),
                );
                return Ok(false);
            }
            ActiveStageOutput::DeactivateAll(_)
            | ActiveStageOutput::PointerDefault
            | ActiveStageOutput::PointerHidden
            | ActiveStageOutput::PointerPosition { .. }
            | ActiveStageOutput::PointerBitmap(_)
            | ActiveStageOutput::MultitransportRequest(_)
            | ActiveStageOutput::AutoDetect(_) => {}
        }
    }

    queue_graphics_update(frame_channel, session, frame_pacer, metrics, pending_region)?;

    Ok(true)
}

fn queue_graphics_update(
    frame_channel: &Channel<Response>,
    session: &ConnectedRdpSession,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
    region: Option<RectRegion>,
) -> Result<(), String> {
    let Some(region) = region else {
        return Ok(());
    };
    if let Some(frame) = frame_pacer.queue(region, tokio::time::Instant::now()) {
        send_graphics_update(frame_channel, session, metrics, frame)?;
    }

    Ok(())
}

fn flush_due_graphics_update(
    frame_channel: &Channel<Response>,
    session: &ConnectedRdpSession,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
) -> Result<(), String> {
    if let Some(frame) = frame_pacer.flush_due(tokio::time::Instant::now()) {
        send_graphics_update(frame_channel, session, metrics, frame)?;
    }

    Ok(())
}

fn flush_pending_graphics_update(
    frame_channel: &Channel<Response>,
    session: &ConnectedRdpSession,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
) -> Result<(), String> {
    if let Some(frame) = frame_pacer.flush_pending(tokio::time::Instant::now()) {
        send_graphics_update(frame_channel, session, metrics, frame)?;
    }

    Ok(())
}

fn send_graphics_update(
    frame_channel: &Channel<Response>,
    session: &ConnectedRdpSession,
    metrics: &mut RdpMetrics,
    frame: QueuedFrame,
) -> Result<(), String> {
    let mut coalesced_updates = frame.coalesced_updates;

    for region in frame.regions {
        let message =
            build_frame_message(session.width, session.height, session.image.data(), region)?;
        metrics.record_sent_frame(message.len(), coalesced_updates);
        coalesced_updates = 0;
        frame_channel
            .send(Response::new(message))
            .map_err(|e| format!("发送 RDP 图像帧失败: {e}"))?;
    }

    Ok(())
}

fn emit_status(app: &tauri::AppHandle, session_id: &str, state: &str, message: Option<String>) {
    let payload = RdpStatusPayload {
        session_id: session_id.to_string(),
        state: state.to_string(),
        message,
    };
    let _ = app.emit(&format!("rdp-status-{}", session_id), payload);
}

fn disconnect_reason(reason: GracefulDisconnectReason) -> String {
    match reason {
        GracefulDisconnectReason::UserInitiated => "用户主动断开".to_string(),
        GracefulDisconnectReason::ServerInitiated => "服务器主动断开".to_string(),
        GracefulDisconnectReason::Other(message) => message,
    }
}
