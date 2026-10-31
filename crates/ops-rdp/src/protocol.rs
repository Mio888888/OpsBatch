use std::net::SocketAddr;
use std::time::Duration;

use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::{ClientConnector, Config, ServerName};
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::Database as InputDatabase;
#[cfg(test)]
use ironrdp::pdu::nego::ResponseFlags;
use ironrdp::rdpdr::Rdpdr;
use ironrdp::rdpsnd::client::Rdpsnd;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite as _, MovableTokioFramed, TokioFramed};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use super::audio::PcmAudioHandler;
use super::drive::{DriveBackend, DRIVE_DEVICE_ID};
use super::h264::H264Pipeline;
use super::clipboard::{
    text_clipboard_formats, ClipboardAction, ClipboardBridge, TextClipboardBackend,
};
use super::config::build_ironrdp_config;
use super::dynamic_channels::{display_control_client, geometry_sink, input_sink};
use super::egfx::RdpEgfxBridge;
use super::file_transfer::{
    build_file_descriptors, collect_upload_files, read_upload_file_chunk, DownloadProgress,
    FileTransferState,
};
use super::frame::{build_frame_message, build_region_frame_message, FramePacer, QueuedFrame};
use super::input::input_operations;
use super::rdpevor::{video_control_channel, video_data_channel};
use super::types::{
    RdpBitmapFrame, RdpConnectResponse, RdpConnectionOptions, RdpCredentials,
    RdpEncodedVideoFrame, RdpMetricsPayload, RdpStatusDetail, RdpStatusPayload, RdpTransportMode,
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
    command_tx: mpsc::UnboundedSender<RdpSessionCommand>,
    encoded_video_rx: Vec<mpsc::UnboundedReceiver<RdpEncodedVideoFrame>>,
    bitmap_rx: Vec<mpsc::UnboundedReceiver<RdpBitmapFrame>>,
    status_rx: Vec<mpsc::UnboundedReceiver<RdpStatusDetail>>,
    h264: Option<H264Pipeline>,
    /// per-phase connect timings for the diagnostics log
    connect_timings: String,
    width: u16,
    height: u16,
    file_transfer: Option<FileTransferState>,
}

struct ConnectorBundle {
    connector: ClientConnector,
    encoded_video_rx: Vec<mpsc::UnboundedReceiver<RdpEncodedVideoFrame>>,
    bitmap_rx: Vec<mpsc::UnboundedReceiver<RdpBitmapFrame>>,
    status_rx: Vec<mpsc::UnboundedReceiver<RdpStatusDetail>>,
}

/// Low-volume diagnostics (one log line every 5 s): network receive rate, how long processing a PDU takes and the
/// longest stall of the session loop. This is what tells a slow server / link from a slow client.
struct SessionDiagnostics {
    started: tokio::time::Instant,
    window_started: tokio::time::Instant,
    pdus: u32,
    rx_bytes: u64,
    process_total: Duration,
    process_max: Duration,
    gaps_over_200ms: u32,
    last_pdu_at: Option<tokio::time::Instant>,
    first_graphics_logged: bool,
}

impl SessionDiagnostics {
    fn new(now: tokio::time::Instant) -> Self {
        Self { started: now, window_started: now, pdus: 0, rx_bytes: 0, process_total: Duration::ZERO, process_max: Duration::ZERO, gaps_over_200ms: 0, last_pdu_at: None, first_graphics_logged: false }
    }

    fn record_pdu(&mut self, bytes: usize, took: Duration, now: tokio::time::Instant) {
        self.pdus += 1;
        self.rx_bytes += bytes as u64;
        self.process_total += took;
        self.process_max = self.process_max.max(took);
        if let Some(last) = self.last_pdu_at {
            if now.duration_since(last) > Duration::from_millis(200) {
                self.gaps_over_200ms += 1;
            }
        }
        self.last_pdu_at = Some(now);
    }

    fn emit_if_due(&mut self, app: &ops_core::Core, session_id: &str, metrics: &RdpMetrics, now: tokio::time::Instant) {
        let elapsed = now.duration_since(self.window_started);
        if elapsed < Duration::from_secs(5) {
            return;
        }
        let secs = elapsed.as_secs_f64();
        let avg_ms = if self.pdus == 0 { 0.0 } else { self.process_total.as_secs_f64() * 1000.0 / f64::from(self.pdus) };
        ops_core::log::emit_log(
            app,
            "info",
            "rdp.stats",
            &format!(
                "session={session_id} t={:.0}s pdu/s={:.1} rx={:.1}KB/s process_avg={avg_ms:.2}ms process_max={:.1}ms gaps>200ms={} server_updates/s={:.1} sent_frames/s={:.1}",
                now.duration_since(self.started).as_secs_f64(),
                f64::from(self.pdus) / secs,
                self.rx_bytes as f64 / secs / 1024.0,
                self.process_max.as_secs_f64() * 1000.0,
                self.gaps_over_200ms,
                f64::from(metrics.server_updates) / metrics.window_secs(now).max(0.001),
                f64::from(metrics.sent_frames) / metrics.window_secs(now).max(0.001),
            ),
            "backend",
        );
        self.window_started = now;
        self.pdus = 0;
        self.rx_bytes = 0;
        self.process_total = Duration::ZERO;
        self.process_max = Duration::ZERO;
        self.gaps_over_200ms = 0;
    }
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

    fn window_secs(&self, now: tokio::time::Instant) -> f64 {
        now.duration_since(self.window_started_at).as_secs_f64()
    }

    fn record_server_update(&mut self) {
        self.server_updates = self.server_updates.saturating_add(1);
    }

    fn record_sent_frame(&mut self, bytes: usize, coalesced_updates: u32) {
        self.sent_frames = self.sent_frames.saturating_add(1);
        self.coalesced_updates = self.coalesced_updates.saturating_add(coalesced_updates);
        self.sent_bytes = self.sent_bytes.saturating_add(bytes as u64);
    }

    fn record_sent_frames(&mut self, frames: u32, bytes: u64) {
        self.sent_frames = self.sent_frames.saturating_add(frames);
        self.coalesced_updates = self.coalesced_updates.saturating_add(frames);
        self.sent_bytes = self.sent_bytes.saturating_add(bytes);
    }

    fn emit_if_due(&mut self, app: &ops_core::Core, session_id: &str, now: tokio::time::Instant) {
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
    app: ops_core::Core,
    options: RdpConnectionOptions,
    credentials: RdpCredentials,
    command_tx: mpsc::UnboundedSender<RdpSessionCommand>,
    command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
    ready: oneshot::Sender<Result<RdpConnectResponse, String>>,
    frame_channel: super::FrameChannel,
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

    let mut session = match connect_rdp_session(&options, &credentials, command_tx.clone()).await {
        Ok(session) => session,
        Err(error) => {
            eprintln!(
                "[RDP][backend][{}] session_connect_error error={}",
                session_id, error
            );
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
    ops_core::log::emit_log(&app, "info", "rdp.stats", &format!("session={session_id} connected {}x{} transport={:?}: {}", session.width, session.height, options.transport_mode, session.connect_timings), "backend");

    if let Err(error) =
        run_active_session(&app, &session_id, &mut session, command_rx, frame_channel).await
    {
        eprintln!(
            "[RDP][backend][{}] active_session_error error={}",
            session_id, error
        );
        emit_status(&app, &session_id, "error", Some(error));
        return;
    }
    emit_status(&app, &session_id, "disconnected", None);
}

async fn connect_rdp_session(
    options: &RdpConnectionOptions,
    credentials: &RdpCredentials,
    command_tx: mpsc::UnboundedSender<RdpSessionCommand>,
) -> Result<ConnectedRdpSession, String> {
    let addr = format!("{}:{}", options.host, options.port);
    let phase_started = tokio::time::Instant::now();
    let std_tcp = tokio::task::spawn_blocking({
        let host = options.host.clone();
        let port = options.port;
        let proxy = options.proxy.clone();
        move || ops_ssh::ssh::connect_tcp_stream(&host, port, proxy, RDP_CONNECT_TIMEOUT.as_secs())
    })
    .await
    .map_err(|e| format!("RDP TCP 连接线程异常 {addr}: {e}"))?
    .map_err(|e| format!("RDP TCP 连接失败 {addr}: {e}"))?;
    // Keepalive probes notice a path that died silently (NAT timeout, Wi-Fi roam) in ~30s instead of never, so the
    // window can reconnect instead of freezing on the last picture.
    let keepalive = socket2::TcpKeepalive::new().with_time(Duration::from_secs(15)).with_interval(Duration::from_secs(5));
    let _ = socket2::SockRef::from(&std_tcp).set_tcp_keepalive(&keepalive);
    std_tcp
        .set_nonblocking(true)
        .map_err(|e| format!("设置 RDP 非阻塞失败: {e}"))?;
    let tcp_ms = phase_started.elapsed().as_millis();
    let tcp = TcpStream::from_std(std_tcp).map_err(|e| format!("RDP TcpStream 转换失败: {e}"))?;
    tcp.set_nodelay(true)
        .map_err(|e| format!("设置 RDP TCP_NODELAY 失败: {e}"))?;

    let client_addr = tcp
        .local_addr()
        .map_err(|e| format!("获取本地 RDP 地址失败: {e}"))?;
    let config = build_ironrdp_config(options, credentials)?;
    let clipboard = options.enable_clipboard.then(ClipboardBridge::new);
    let download_dir = std::env::temp_dir().join("opsbatch-rdp-downloads");
    let _ = std::fs::create_dir_all(&download_dir);
    let file_transfer = options
        .enable_clipboard
        .then(|| FileTransferState::new(download_dir));
    let mut framed = TokioFramed::new(tcp);
    let ConnectorBundle {
        mut connector,
        encoded_video_rx,
        bitmap_rx,
        status_rx,
    } = build_client_connector_with_clipboard(config, client_addr, options, clipboard.clone())?;
    let should_upgrade = tokio::time::timeout(
        RDP_CONNECT_TIMEOUT,
        ironrdp_tokio::connect_begin(&mut framed, &mut connector),
    )
    .await
    .map_err(|_| "RDP 协议协商超时".to_string())?
    .map_err(|e| format!("RDP 协议协商失败: {}", e.report()))?;

    let negotiate_ms = phase_started.elapsed().as_millis();
    let stream = framed.into_inner_no_leftover();
    let (tls_stream, tls_cert) = tokio::time::timeout(
        RDP_CONNECT_TIMEOUT,
        ironrdp_tls::upgrade(stream, &options.host),
    )
    .await
    .map_err(|_| "RDP TLS 握手超时".to_string())?
    .map_err(|e| format!("RDP TLS 握手失败: {e}"))?;

    let tls_ms = phase_started.elapsed().as_millis();
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
    let total_ms = phase_started.elapsed().as_millis();
    let connect_timings = format!("tcp {tcp_ms}ms, x224 negotiation done at {negotiate_ms}ms, tls at {tls_ms}ms, NLA + capability exchange done at {total_ms}ms");
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
        h264: None,
        connect_timings,
        width,
        height,
        file_transfer,
        command_tx,
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
    if options.enable_clipboard {
        let bridge = clipboard.unwrap_or_else(ClipboardBridge::new);
        connector.attach_static_channel(CliprdrClient::new(Box::new(TextClipboardBackend::new(
            bridge,
        ))));
    }
    if options.enable_audio {
        connector.attach_static_channel(Rdpsnd::new(Box::new(PcmAudioHandler::new())));
    }
    if let Some(folder) = &options.disk_path {
        // a folder that cannot be shared stops the connection with the reason, instead of a drive that never shows up
        let backend = DriveBackend::new(folder)?;
        let label = backend.label().to_string();
        connector.attach_static_channel(Rdpdr::new(Box::new(backend), "OpsBatch".to_string()).with_drives(Some(vec![(DRIVE_DEVICE_ID, label)])));
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
    app: &ops_core::Core,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    mut command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
    frame_channel: super::FrameChannel,
) -> Result<(), String> {
    let mut frame_pacer = FramePacer::new(RDP_FRAME_INTERVAL);
    let mut metrics = RdpMetrics::new(tokio::time::Instant::now());
    let mut diagnostics = SessionDiagnostics::new(tokio::time::Instant::now());
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
                    Some(RdpSessionCommand::UploadFiles(paths, position)) => {
                        process_upload_files(session, paths, position);
                        flush_clipboard_actions(app, session_id, session).await?;
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
                let process_started = tokio::time::Instant::now();
                let outputs = session
                    .active_stage
                    .process(&mut session.image, action, &payload)
                    .map_err(|e| format!("处理 RDP 帧失败: {}", e.report()))?;
                diagnostics.record_pdu(payload.len(), process_started.elapsed(), process_started);
                if !diagnostics.first_graphics_logged && outputs.iter().any(|o| matches!(o, ActiveStageOutput::GraphicsUpdate(_))) {
                    diagnostics.first_graphics_logged = true;
                    ops_core::log::emit_log(app, "info", "rdp.stats", &format!("session={session_id} first graphics update after {:.2}s", diagnostics.started.elapsed().as_secs_f64()), "backend");
                }
                if !handle_stage_outputs(app, session_id, session, &frame_channel, &mut frame_pacer, &mut metrics, outputs).await? {
                    return Ok(());
                }
                flush_clipboard_actions(app, session_id, session).await?;
            }
            _ = sleep_until_deadline(frame_deadline), if frame_deadline.is_some() => {
                flush_due_graphics_update(&frame_channel, session, &mut frame_pacer, &mut metrics)?;
            }
            _ = clipboard_interval.tick(), if session.clipboard.is_some() => {
                poll_local_clipboard(app, session_id, session).await?;
                flush_clipboard_timeouts(session).await?;
                try_trigger_auto_paste(session).await?;
            }
        }
        diagnostics.emit_if_due(app, session_id, &metrics, tokio::time::Instant::now());
        metrics.emit_if_due(app, session_id, tokio::time::Instant::now());
        forward_status_details(app, session_id, session).await?;
        forward_bitmap_frames(&frame_channel, session, &mut metrics)?;
        forward_encoded_video_frames(&frame_channel, session, &mut metrics)?;
    }
}

fn forward_bitmap_frames(
    frame_channel: &super::FrameChannel,
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
            .send(message)
            .map_err(|e| format!("发送 RDPGFX bitmap 图像帧失败: {e}"))?;
    }

    Ok(())
}

async fn forward_status_details(
    app: &ops_core::Core,
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
        ops_core::log::emit_log(app, "info", "rdp.stats", &format!("session={session_id} status detail {state}: {}", message.as_deref().unwrap_or("")), "backend");
        emit_status_with_detail(app, session_id, state, message, Some(detail));
    }

    Ok(())
}

fn forward_encoded_video_frames(
    frame_channel: &super::FrameChannel,
    session: &mut ConnectedRdpSession,
    metrics: &mut RdpMetrics,
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

    // Decoding runs on its own thread (see `H264Pipeline`): every sample is submitted in order, the thread sends
    // only what changed, and its counters feed the metrics below.
    if frames.is_empty() {
        return match session.h264.as_ref() {
            Some(pipeline) => sync_h264_metrics(pipeline, metrics),
            None => Ok(()),
        };
    }
    let pipeline = session.h264.get_or_insert_with(|| H264Pipeline::spawn(frame_channel.clone()));
    for frame in frames {
        pipeline.submit(frame)?;
    }
    sync_h264_metrics(pipeline, metrics)
}

fn sync_h264_metrics(pipeline: &H264Pipeline, metrics: &mut RdpMetrics) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if let Some(error) = pipeline.take_error() {
        return Err(error);
    }
    for _ in 0..pipeline.counters.submitted.swap(0, Ordering::Relaxed) {
        metrics.record_server_update();
    }
    let (sent, bytes) = (pipeline.counters.sent.swap(0, Ordering::Relaxed), pipeline.counters.sent_bytes.swap(0, Ordering::Relaxed));
    if sent > 0 {
        metrics.record_sent_frames(sent as u32, bytes);
    }
    Ok(())
}

async fn sleep_until_deadline(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    }
}

async fn poll_local_clipboard(
    app: &ops_core::Core,
    session_id: &str,
    session: &mut ConnectedRdpSession,
) -> Result<(), String> {
    if let Some(clipboard) = session.clipboard.clone() {
        clipboard.poll_local_clipboard();
    }
    flush_clipboard_actions(app, session_id, session).await
}

async fn flush_clipboard_actions(
    app: &ops_core::Core,
    session_id: &str,
    session: &mut ConnectedRdpSession,
) -> Result<(), String> {
    let actions = match session.clipboard.as_ref() {
        Some(clipboard) => clipboard.drain_actions(),
        None => return Ok(()),
    };

    for action in actions {
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
                ClipboardAction::InitiateFileCopy(descriptors) => cliprdr
                    .initiate_file_copy(descriptors)
                    .map_err(|e| format!("生成 RDP 文件剪贴板格式列表失败: {e}"))?,
                ClipboardAction::AdvertiseLocalFiles(paths) => {
                    let path_bufs: Vec<std::path::PathBuf> =
                        paths.into_iter().map(std::path::PathBuf::from).collect();
                    let upload_files = collect_upload_files(&path_bufs);
                    if upload_files.is_empty() {
                        continue;
                    }
                    let descriptors = build_file_descriptors(&upload_files);
                    if let Some(ft) = session.file_transfer.as_mut() {
                        ft.upload_files = upload_files;
                    }
                    cliprdr
                        .initiate_file_copy(descriptors)
                        .map_err(|e| format!("生成 RDP 本地文件剪贴板广告失败: {e}"))?
                }
                ClipboardAction::ServeFileContentsRequest(request) => {
                    let upload_files = session
                        .file_transfer
                        .as_ref()
                        .map(|ft| ft.upload_files.as_slice())
                        .unwrap_or(&[]);
                    let response = read_upload_file_chunk(&request, upload_files);
                    cliprdr
                        .submit_file_contents(response)
                        .map_err(|e| format!("生成 RDP 文件内容响应失败: {e}"))?
                }
                ClipboardAction::RequestRemoteFileList(format) => cliprdr
                    .initiate_paste(format)
                    .map_err(|e| format!("生成 RDP 远程文件列表请求失败: {e}"))?,
                ClipboardAction::StartFileDownload {
                    files,
                    clip_data_id,
                } => {
                    if let Some(ft) = session.file_transfer.as_mut() {
                        ft.download_progress = Some(DownloadProgress::new(files, clip_data_id));
                        if let Some(dp) = ft.download_progress.as_mut() {
                            let nsid = &mut ft.next_stream_id;
                            if let Some(req) = dp.build_size_request(nsid) {
                                cliprdr
                                    .request_file_contents(req)
                                    .map_err(|e| format!("生成 RDP 文件下载请求失败: {e}"))?
                            } else {
                                Vec::new().into()
                            }
                        } else {
                            Vec::new().into()
                        }
                    } else {
                        Vec::new().into()
                    }
                }
                ClipboardAction::ProcessFileContentsResponse(response) => {
                    let Some(ft) = session.file_transfer.as_mut() else {
                        continue;
                    };
                    let Some(dp) = ft.download_progress.as_mut() else {
                        continue;
                    };
                    let ddir = ft.download_dir.clone();
                    let nsid = &mut ft.next_stream_id;
                    match dp.handle_response(&response, &ddir, nsid) {
                        Ok(Some(req)) => cliprdr
                            .request_file_contents(req)
                            .map_err(|e| format!("生成 RDP 文件下载续传请求失败: {e}"))?,
                        Ok(None) => {
                            emit_download_complete(app, session_id, ft);
                            Vec::new().into()
                        }
                        Err(e) => {
                            eprintln!("[RDP][backend][{session_id}] file_download_error error={e}");
                            emit_download_complete(app, session_id, ft);
                            return Err(e);
                        }
                    }
                }
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

/// 处理前端拖拽上传：收集文件元数据，广告到远程剪贴板
fn process_upload_files(
    session: &mut ConnectedRdpSession,
    paths: Vec<String>,
    position: Option<(u16, u16)>,
) {
    let path_bufs: Vec<std::path::PathBuf> =
        paths.into_iter().map(std::path::PathBuf::from).collect();
    let upload_files = collect_upload_files(&path_bufs);
    let descriptors = build_file_descriptors(&upload_files);
    if let Some(ft) = session.file_transfer.as_mut() {
        ft.upload_files = upload_files;
        ft.auto_paste_position = position;
    }
    if let Some(clipboard) = session.clipboard.as_ref() {
        clipboard.advertise_files(descriptors);
    }
}

/// 定期清理 CLIPRDR 超时锁和过期请求
async fn flush_clipboard_timeouts(session: &mut ConnectedRdpSession) -> Result<(), String> {
    if session.clipboard.is_none() {
        return Ok(());
    }
    let messages = {
        let cliprdr = session
            .active_stage
            .get_svc_processor_mut::<CliprdrClient>()
            .ok_or_else(|| "RDP 剪贴板通道不可用".to_string())?;
        cliprdr
            .drive_timeouts()
            .map_err(|e| format!("清理 RDP 剪贴板超时失败: {e}"))?
    };
    let frame = session
        .active_stage
        .process_svc_processor_messages(messages)
        .map_err(|e| format!("编码 RDP 剪贴板超时帧失败: {e}"))?;
    session
        .framed
        .write_all(&frame)
        .await
        .map_err(|e| format!("发送 RDP 剪贴板超时帧失败: {e}"))?;
    Ok(())
}

/// 下载完成时发送事件到前端
/// 检查是否可以触发自动粘贴：远程已确认 FormatList + 有待粘贴位置
async fn try_trigger_auto_paste(session: &mut ConnectedRdpSession) -> Result<(), String> {
    // 检查条件：format list accepted + 有 position
    let accepted = session
        .clipboard
        .as_ref()
        .is_some_and(|cb| cb.take_format_list_accepted());

    let position = session
        .file_transfer
        .as_ref()
        .and_then(|ft| ft.auto_paste_position);

    if !accepted || position.is_none() {
        return Ok(());
    }

    let pos = position.unwrap();
    // 清除待粘贴标记，防止重复触发
    if let Some(ft) = session.file_transfer.as_mut() {
        ft.auto_paste_position = None;
    }

    // 通过 command channel 排定时输入事件：先点击定位，再 Ctrl+V
    let tx = session.command_tx.clone();
    tokio::spawn(async move {
        // 移动到释放位置
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::MouseMove { x: pos.0, y: pos.1 },
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 左键点击：让该位置窗口获得焦点
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::MouseButton {
                x: pos.0,
                y: pos.1,
                button: 0,
                down: true,
            },
        ));
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::MouseButton {
                x: pos.0,
                y: pos.1,
                button: 0,
                down: false,
            },
        ));
        // 等待窗口焦点切换完成
        tokio::time::sleep(Duration::from_millis(300)).await;
        // Ctrl+V
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::KeyScancode {
                code: 0x1d,
                extended: false,
                down: true,
            },
        ));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::KeyScancode {
                code: 0x2f,
                extended: false,
                down: true,
            },
        ));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::KeyScancode {
                code: 0x2f,
                extended: false,
                down: false,
            },
        ));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx.send(RdpSessionCommand::Input(
            super::types::RdpInputEvent::KeyScancode {
                code: 0x1d,
                extended: false,
                down: false,
            },
        ));
    });

    Ok(())
}

fn emit_download_complete(app: &ops_core::Core, session_id: &str, ft: &FileTransferState) {
    let Some(dp) = &ft.download_progress else {
        return;
    };
    if !dp.is_finished() {
        return;
    }
    let payload = serde_json::json!({
        "sessionId": session_id,
        "completed": dp.completed.clone(),
        "failed": dp.failed.clone(),
        "downloadDir": ft.download_dir.to_string_lossy(),
    });
    let _ = app.emit(&format!("rdp-file-transfer-{session_id}"), payload);
}

/// Per-input-event tracing (4 database rows and 4 log events for every mouse move) is off unless
/// `OPSBATCH_RDP_TRACE=1`: it ran on the protocol task and was the main source of lag while moving the mouse.
pub(super) fn input_trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("OPSBATCH_RDP_TRACE").is_ok_and(|v| v == "1"))
}

async fn process_input_event(
    app: &ops_core::Core,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    frame_channel: &super::FrameChannel,
    frame_pacer: &mut FramePacer,
    metrics: &mut RdpMetrics,
    event: super::types::RdpInputEvent,
) -> Result<bool, String> {
    let trace = input_trace_enabled();
    let event_summary = if trace { super::input::input_event_summary(&event) } else { String::new() };
    if trace {
        ops_core::log::emit_log(
            app,
            "info",
            "rdp.input",
            &format!("stage=protocol_received sessionId={session_id} size={}x{} event={event_summary}", session.width, session.height),
            "backend",
        );
    }

    let operations = input_operations(event, session.width, session.height).map_err(|error| {
        ops_core::log::emit_log(
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
    let operation_preview = if trace { format!("{operations:?}") } else { String::new() };
    let events = session.input.apply(operations);
    let fastpath_event_count = events.len();
    if trace {
        ops_core::log::emit_log(
            app,
            "info",
            "rdp.input",
            &format!(
                "stage=fastpath_encode_start sessionId={session_id} event={event_summary} operationCount={operation_count} fastpathEventCount={fastpath_event_count} operations={operation_preview}"
            ),
            "backend",
        );
    }

    let outputs = session
        .active_stage
        .process_fastpath_input(&mut session.image, events.as_slice())
        .map_err(|e| {
            let error = format!("编码 RDP 输入失败: {e}");
            ops_core::log::emit_log(
                app,
                "warn",
                "rdp.input",
                &format!("stage=fastpath_encode_failed sessionId={session_id} event={event_summary} error={error}"),
                "backend",
            );
            error
        })?;
    if trace {
        let output_count = outputs.len();
        ops_core::log::emit_log(
            app,
            "info",
            "rdp.input",
            &format!("stage=fastpath_encode_done sessionId={session_id} event={event_summary} outputCount={output_count}"),
            "backend",
        );
    }
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
    app: &ops_core::Core,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    frame_channel: &super::FrameChannel,
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
    frame_channel: &super::FrameChannel,
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
    frame_channel: &super::FrameChannel,
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
    frame_channel: &super::FrameChannel,
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
    frame_channel: &super::FrameChannel,
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
            .send(message)
            .map_err(|e| format!("发送 RDP 图像帧失败: {e}"))?;
    }

    Ok(())
}

fn emit_status(app: &ops_core::Core, session_id: &str, state: &str, message: Option<String>) {
    emit_status_with_detail(app, session_id, state, message, None);
}

fn emit_status_with_detail(
    app: &ops_core::Core,
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
