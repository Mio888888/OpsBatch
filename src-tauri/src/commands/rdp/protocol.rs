use ironrdp::connector::{ClientConnector, ServerName};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::Database as InputDatabase;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite as _, MovableTokioFramed, TokioFramed};
use tauri::Emitter;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use super::config::build_ironrdp_config;
use super::frame::build_frame_payload;
use super::input::input_operations;
use super::types::{
    RdpConnectResponse, RdpConnectionOptions, RdpCredentials, RdpFramePayload, RdpStatusPayload,
    RectRegion,
};
use super::{RdpSessionCommand, RDP_CONNECT_TIMEOUT};

type RdpFramed = MovableTokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

struct ConnectedRdpSession {
    framed: RdpFramed,
    active_stage: ActiveStage,
    image: DecodedImage,
    input: InputDatabase,
    width: u16,
    height: u16,
}

pub(super) async fn run_rdp_session(
    app: tauri::AppHandle,
    options: RdpConnectionOptions,
    credentials: RdpCredentials,
    command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
    ready: oneshot::Sender<Result<RdpConnectResponse, String>>,
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

    if let Err(error) = run_active_session(&app, &session_id, &mut session, command_rx).await {
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
    let mut framed = TokioFramed::new(tcp);
    let mut connector = ClientConnector::new(config, client_addr);

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
        width,
        height,
    })
}

async fn run_active_session(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    mut command_rx: mpsc::UnboundedReceiver<RdpSessionCommand>,
) -> Result<(), String> {
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                match command {
                    Some(RdpSessionCommand::Input(event)) => {
                        if !process_input_event(app, session_id, session, event).await? {
                            return Ok(());
                        }
                    }
                    Some(RdpSessionCommand::Disconnect) | None => return Ok(()),
                }
            }
            frame = session.framed.read_pdu() => {
                let (action, payload) = frame.map_err(|e| format!("读取 RDP 帧失败: {e}"))?;
                let outputs = session
                    .active_stage
                    .process(&mut session.image, action, &payload)
                    .map_err(|e| format!("处理 RDP 帧失败: {e}"))?;
                if !handle_stage_outputs(app, session_id, session, outputs).await? {
                    return Ok(());
                }
            }
        }
    }
}

async fn process_input_event(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    event: super::types::RdpInputEvent,
) -> Result<bool, String> {
    let operations = input_operations(event, session.width, session.height)?;
    let events = session.input.apply(operations);
    let outputs = session
        .active_stage
        .process_fastpath_input(&mut session.image, events.as_slice())
        .map_err(|e| format!("编码 RDP 输入失败: {e}"))?;
    handle_stage_outputs(app, session_id, session, outputs).await
}

async fn handle_stage_outputs(
    app: &tauri::AppHandle,
    session_id: &str,
    session: &mut ConnectedRdpSession,
    outputs: Vec<ActiveStageOutput>,
) -> Result<bool, String> {
    for output in outputs {
        match output {
            ActiveStageOutput::ResponseFrame(frame) => session
                .framed
                .write_all(&frame)
                .await
                .map_err(|e| format!("发送 RDP 响应帧失败: {e}"))?,
            ActiveStageOutput::GraphicsUpdate(rect) => {
                let payload = build_frame_payload(
                    session_id,
                    session.width,
                    session.height,
                    session.image.data(),
                    RectRegion::from_inclusive(rect),
                )?;
                emit_frame(app, payload);
            }
            ActiveStageOutput::Terminate(reason) => {
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

    Ok(true)
}

fn emit_status(app: &tauri::AppHandle, session_id: &str, state: &str, message: Option<String>) {
    let payload = RdpStatusPayload {
        session_id: session_id.to_string(),
        state: state.to_string(),
        message,
    };
    let _ = app.emit(&format!("rdp-status-{}", session_id), payload);
}

fn emit_frame(app: &tauri::AppHandle, payload: RdpFramePayload) {
    let _ = app.emit(&format!("rdp-frame-{}", payload.session_id), payload);
}

fn disconnect_reason(reason: GracefulDisconnectReason) -> String {
    match reason {
        GracefulDisconnectReason::UserInitiated => "用户主动断开".to_string(),
        GracefulDisconnectReason::ServerInitiated => "服务器主动断开".to_string(),
        GracefulDisconnectReason::Other(message) => message,
    }
}
