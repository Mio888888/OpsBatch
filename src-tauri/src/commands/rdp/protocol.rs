use std::net::SocketAddr;
use std::time::Duration;

use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::{ClientConnector, Config, ServerName};
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::Database as InputDatabase;
#[cfg(test)]
use ironrdp::pdu::nego::ResponseFlags;
use ironrdp::rdpsnd::client::Rdpsnd;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite as _, MovableTokioFramed, TokioFramed};
use tauri::ipc::{Channel, Response};
use tauri::{Emitter, Manager};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use super::audio::{PcmAudioHandler, WebRtcAudioHandler};
use super::clipboard::{
    text_clipboard_formats, ClipboardAction, ClipboardBridge, TextClipboardBackend,
};
use super::config::build_ironrdp_config;
use super::dynamic_channels::{display_control_client, geometry_sink, input_sink};
use super::egfx::RdpEgfxBridge;
use super::frame::{build_frame_message, build_region_frame_message, FramePacer, QueuedFrame};
use super::input::input_operations;
use super::rdpevor::{video_control_channel, video_data_channel};
use super::types::{
    RdpBitmapFrame, RdpConnectResponse, RdpConnectionOptions, RdpCredentials, RdpEncodedAudioFrame,
    RdpEncodedVideoFrame, RdpMetricsPayload, RdpStatusDetail, RdpStatusPayload, RdpTransportMode,
    RectRegion,
};
use super::webrtc::RdpWebRtcManager;
use super::{RdpSessionCommand, RDP_CONNECT_TIMEOUT};
use crate::ssh;

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
    encoded_video_rx: Vec<mpsc::UnboundedReceiver<RdpEncodedVideoFrame>>,
    bitmap_rx: Vec<mpsc::UnboundedReceiver<RdpBitmapFrame>>,
    status_rx: Vec<mpsc::UnboundedReceiver<RdpStatusDetail>>,
    encoded_audio_rx: Option<mpsc::UnboundedReceiver<RdpEncodedAudioFrame>>,
    width: u16,
    height: u16,
}

struct ConnectorBundle {
    connector: ClientConnector,
    encoded_video_rx: Vec<mpsc::UnboundedReceiver<RdpEncodedVideoFrame>>,
    bitmap_rx: Vec<mpsc::UnboundedReceiver<RdpBitmapFrame>>,
    status_rx: Vec<mpsc::UnboundedReceiver<RdpStatusDetail>>,
    encoded_audio_rx: Option<mpsc::UnboundedReceiver<RdpEncodedAudioFrame>>,
}

#[derive(Debug)]
struct RdpMetrics {
    window_started_at: tokio::time::Instant,
    server_updates: u32,
    sent_frames: u32,
    coalesced_updates: u32,
    sent_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(super) struct NegotiationResponseFlagsDiagnostics {
    pub(super) bits: u8,
    pub(super) extended_client_data: bool,
    pub(super) dynvc_gfx: bool,
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

#[cfg(test)]
pub(super) fn negotiation_response_flags_diagnostics(
    flags: ResponseFlags,
) -> NegotiationResponseFlagsDiagnostics {
    NegotiationResponseFlagsDiagnostics {
        bits: flags.bits(),
        extended_client_data: flags.contains(ResponseFlags::EXTENDED_CLIENT_DATA_SUPPORTED),
        dynvc_gfx: flags.contains(ResponseFlags::DYNVC_GFX_PROTOCOL_SUPPORTED),
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
    let connecting_message = if options.transport_mode == RdpTransportMode::H264Direct {
        "正在进行 RDP TCP、TLS、NLA 与 EGFX 握手"
    } else {
        "正在进行 RDP TCP、TLS 与 NLA 握手"
    };
    emit_status(
        &app,
        &session_id,
        "connecting",
        Some(connecting_message.to_string()),
    );

    let mut session = match connect_rdp_session(&options, &credentials).await {
        Ok(session) => session,
        Err(error) => {
            eprintln!(
                "[RDP][backend][{}] session_connect_error error={}",
                session_id, error
            );
            emit_status(&app, &session_id, "error", Some(error.clone()));
            let _ = app.state::<RdpWebRtcManager>().close(&session_id).await;
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
        eprintln!(
            "[RDP][backend][{}] active_session_error error={}",
            session_id, error
        );
        let _ = app.state::<RdpWebRtcManager>().close(&session_id).await;
        emit_status(&app, &session_id, "error", Some(error));
        return;
    }

    let _ = app.state::<RdpWebRtcManager>().close(&session_id).await;
    emit_status(&app, &session_id, "disconnected", None);
}

async fn connect_rdp_session(
    options: &RdpConnectionOptions,
    credentials: &RdpCredentials,
) -> Result<ConnectedRdpSession, String> {
    let addr = format!("{}:{}", options.host, options.port);
    let std_tcp = tokio::task::spawn_blocking({
        let host = options.host.clone();
        let port = options.port;
        let proxy = options.proxy.clone();
        move || ssh::connect_tcp_stream(&host, port, proxy, RDP_CONNECT_TIMEOUT.as_secs())
    })
    .await
    .map_err(|e| format!("RDP TCP 连接线程异常 {addr}: {e}"))?
    .map_err(|e| format!("RDP TCP 连接失败 {addr}: {e}"))?;
    std_tcp
        .set_nonblocking(true)
        .map_err(|e| format!("设置 RDP 非阻塞失败: {e}"))?;
    let tcp = TcpStream::from_std(std_tcp).map_err(|e| format!("RDP TcpStream 转换失败: {e}"))?;
    tcp.set_nodelay(true)
        .map_err(|e| format!("设置 RDP TCP_NODELAY 失败: {e}"))?;

    let client_addr = tcp
        .local_addr()
        .map_err(|e| format!("获取本地 RDP 地址失败: {e}"))?;
    let config = build_ironrdp_config(options, credentials)?;
    let clipboard = options.enable_clipboard.then(ClipboardBridge::new);
    let mut framed = TokioFramed::new(tcp);
    let ConnectorBundle {
        mut connector,
        encoded_video_rx,
        bitmap_rx,
        status_rx,
        encoded_audio_rx,
    } = build_client_connector_with_clipboard(config, client_addr, options, clipboard.clone())?;
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
        encoded_video_rx,
        bitmap_rx,
        status_rx,
        encoded_audio_rx,
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
        .map(|bundle| bundle.connector)
}

#[cfg(test)]
pub(super) fn h264_direct_encoded_video_receiver_count_for_tests(
    config: Config,
    client_addr: SocketAddr,
    options: &RdpConnectionOptions,
) -> Result<usize, String> {
    build_client_connector_with_clipboard(config, client_addr, options, None)
        .map(|bundle| bundle.encoded_video_rx.len())
}

fn build_client_connector_with_clipboard(
    config: Config,
    client_addr: SocketAddr,
    options: &RdpConnectionOptions,
    clipboard: Option<ClipboardBridge>,
) -> Result<ConnectorBundle, String> {
    let mut connector = ClientConnector::new(config, client_addr);
    let mut encoded_video_rx = Vec::new();
    let mut bitmap_rx = Vec::new();
    let mut status_rx = Vec::new();
    let mut encoded_audio_rx = None;
    if options.enable_clipboard {
        let bridge = clipboard.unwrap_or_else(ClipboardBridge::new);
        connector.attach_static_channel(CliprdrClient::new(Box::new(TextClipboardBackend::new(
            bridge,
        ))));
    }
    if options.enable_audio {
        if options.transport_mode == RdpTransportMode::H264Direct {
            let (audio_handler, receiver) = WebRtcAudioHandler::new(options.session_id.clone());
            connector.attach_static_channel(Rdpsnd::new(Box::new(audio_handler)));
            encoded_audio_rx = Some(receiver);
        } else {
            connector.attach_static_channel(Rdpsnd::new(Box::new(PcmAudioHandler::new())));
        }
    }
    if options.transport_mode == RdpTransportMode::H264Direct {
        let (egfx_client, egfx_receiver, egfx_bitmap_rx, egfx_status_rx) =
            RdpEgfxBridge::new(options.session_id.clone());
        let (rdpevor_data, rdpevor_receiver) = video_data_channel(options.session_id.clone());
        connector.attach_static_channel(
            DrdynvcClient::new()
                .with_diagnostics_label(options.session_id.clone())
                .with_dynamic_channel(egfx_client)
                .with_dynamic_channel(display_control_client(options.session_id.clone()))
                .with_listener(video_control_channel(options.session_id.clone()))
                .with_listener(rdpevor_data)
                .with_listener(geometry_sink(options.session_id.clone()))
                .with_listener(input_sink(options.session_id.clone())),
        );
        encoded_video_rx.push(egfx_receiver);
        encoded_video_rx.push(rdpevor_receiver);
        bitmap_rx.push(egfx_bitmap_rx);
        status_rx.push(egfx_status_rx);
    }
    Ok(ConnectorBundle {
        connector,
        encoded_video_rx,
        bitmap_rx,
        status_rx,
        encoded_audio_rx,
    })
}

#[cfg(test)]
pub(super) fn build_h264_direct_drdynvc_for_tests(session_id: &str) -> DrdynvcClient {
    let (egfx_client, _, _, _) = RdpEgfxBridge::new(session_id.to_string());
    DrdynvcClient::new()
        .with_diagnostics_label(session_id.to_string())
        .with_dynamic_channel(egfx_client)
        .with_dynamic_channel(display_control_client(session_id.to_string()))
        .with_listener(video_control_channel(session_id.to_string()))
        .with_listener(video_data_channel(session_id.to_string()).0)
        .with_listener(geometry_sink(session_id.to_string()))
        .with_listener(input_sink(session_id.to_string()))
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
                    Some(RdpSessionCommand::Disconnect) => {
                        flush_pending_graphics_update(&frame_channel, session, &mut frame_pacer, &mut metrics)?;
                        return Ok(());
                    }
                    None => {
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
                    .map_err(|e| format!("处理 RDP 帧失败: {}", e.report()))?;
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
        forward_status_details(app, session_id, session).await?;
        forward_bitmap_frames(&frame_channel, session, &mut metrics)?;
        forward_encoded_video_frames(app, session).await?;
        forward_encoded_audio_frames(app, session).await?;
    }
}

fn forward_bitmap_frames(
    frame_channel: &Channel<Response>,
    session: &mut ConnectedRdpSession,
    metrics: &mut RdpMetrics,
) -> Result<(), String> {
    if session.bitmap_rx.is_empty() {
        return Ok(());
    }

    let mut frames = Vec::new();
    for bitmap_rx in &mut session.bitmap_rx {
        while let Ok(frame) = bitmap_rx.try_recv() {
            frames.push(frame);
        }
    }

    for frame in frames {
        let message = build_region_frame_message(&frame)?;
        metrics.record_server_update();
        metrics.record_sent_frame(message.len(), 1);
        frame_channel
            .send(Response::new(message))
            .map_err(|e| format!("发送 RDPGFX bitmap 图像帧失败: {e}"))?;
    }

    Ok(())
}

async fn forward_status_details(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
) -> Result<(), String> {
    if session.status_rx.is_empty() {
        return Ok(());
    }

    let mut details = Vec::new();
    for status_rx in &mut session.status_rx {
        while let Ok(detail) = status_rx.try_recv() {
            details.push(detail);
        }
    }

    for detail in details {
        let (state, message) = match &detail {
            RdpStatusDetail::H264DirectUnavailable { reason } => {
                ("h264DirectUnavailable", Some(reason.clone()))
            }
        };
        emit_status_with_detail(app, session_id, state, message, Some(detail));
    }

    Ok(())
}

async fn forward_encoded_video_frames(
    app: &tauri::AppHandle,
    session: &mut ConnectedRdpSession,
) -> Result<(), String> {
    if session.encoded_video_rx.is_empty() {
        return Ok(());
    }

    let mut frames = Vec::new();
    for encoded_video_rx in &mut session.encoded_video_rx {
        while let Ok(frame) = encoded_video_rx.try_recv() {
            frames.push(frame);
        }
    }

    if frames.is_empty() {
        return Ok(());
    }

    let manager = app.state::<RdpWebRtcManager>();
    for frame in frames {
        manager.write_video_frame(frame).await?;
    }

    Ok(())
}

async fn forward_encoded_audio_frames(
    app: &tauri::AppHandle,
    session: &mut ConnectedRdpSession,
) -> Result<(), String> {
    let Some(encoded_audio_rx) = session.encoded_audio_rx.as_mut() else {
        return Ok(());
    };

    let mut frames = Vec::new();
    while let Ok(frame) = encoded_audio_rx.try_recv() {
        frames.push(frame);
    }

    if frames.is_empty() {
        return Ok(());
    }

    let manager = app.state::<RdpWebRtcManager>();
    for frame in frames {
        manager.write_audio_frame(frame).await?;
    }

    Ok(())
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
    let event_summary = super::input::input_event_summary(&event);
    crate::commands::app_log::emit_log(
        app,
        "info",
        "rdp.input",
        &format!(
            "stage=protocol_received sessionId={session_id} size={}x{} event={event_summary}",
            session.width, session.height
        ),
        "backend",
    );

    let operations = input_operations(event, session.width, session.height).map_err(|error| {
        crate::commands::app_log::emit_log(
            app,
            "warn",
            "rdp.input",
            &format!(
                "stage=input_operations_failed sessionId={session_id} event={event_summary} error={error}"
            ),
            "backend",
        );
        error
    })?;
    let operation_count = operations.len();
    let operation_preview = format!("{operations:?}");
    let events = session.input.apply(operations);
    let fastpath_event_count = events.len();
    crate::commands::app_log::emit_log(
        app,
        "info",
        "rdp.input",
        &format!(
            "stage=fastpath_encode_start sessionId={session_id} event={event_summary} operationCount={operation_count} fastpathEventCount={fastpath_event_count} operations={operation_preview}"
        ),
        "backend",
    );

    let outputs = session
        .active_stage
        .process_fastpath_input(&mut session.image, events.as_slice())
        .map_err(|e| {
            let error = format!("编码 RDP 输入失败: {e}");
            crate::commands::app_log::emit_log(
                app,
                "warn",
                "rdp.input",
                &format!("stage=fastpath_encode_failed sessionId={session_id} event={event_summary} error={error}"),
                "backend",
            );
            error
        })?;
    let output_count = outputs.len();
    crate::commands::app_log::emit_log(
        app,
        "info",
        "rdp.input",
        &format!(
            "stage=fastpath_encode_done sessionId={session_id} event={event_summary} outputCount={output_count}"
        ),
        "backend",
    );
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
    emit_status_with_detail(app, session_id, state, message, None);
}

fn emit_status_with_detail(
    app: &tauri::AppHandle,
    session_id: &str,
    state: &str,
    message: Option<String>,
    detail: Option<RdpStatusDetail>,
) {
    let payload = RdpStatusPayload {
        session_id: session_id.to_string(),
        state: state.to_string(),
        message,
        detail,
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
