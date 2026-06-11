use ::vnc::{
    ClientKeyEvent, ClientMouseEvent, PixelFormat, Rect, VncClient, VncConnector, VncEncoding,
    VncEvent, X11Event,
};
use dashmap::DashMap;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

use crate::db::Database;

const DEFAULT_VNC_PORT: u16 = 5900;

#[derive(Default)]
pub struct VncManager {
    sessions: DashMap<String, VncSession>,
}

#[derive(Clone)]
struct VncSession {
    client: VncClient,
    stop: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VncSettings {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default, rename = "vncPort")]
    pub vnc_port: Option<u16>,
    #[serde(default, rename = "vncPassword")]
    pub vnc_password: Option<String>,
    #[serde(default, rename = "vncShared")]
    pub vnc_shared: Option<bool>,
}

#[derive(Debug, Clone)]
struct VncHostConfig {
    host: String,
    port: u16,
    password: String,
    shared: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VncConnectResponse {
    pub session_id: String,
    pub host_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct VncStatusPayload {
    session_id: String,
    state: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum VncFramePayload {
    Resize {
        session_id: String,
        width: u16,
        height: u16,
    },
    Raw {
        session_id: String,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        data: Vec<u8>,
    },
    Copy {
        session_id: String,
        dst_x: u16,
        dst_y: u16,
        src_x: u16,
        src_y: u16,
        width: u16,
        height: u16,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum VncInputEvent {
    #[serde(rename = "mouse")]
    Mouse { x: u16, y: u16, buttons: u8 },
    #[serde(rename = "key")]
    Key { keycode: u32, down: bool },
    #[serde(rename = "refresh")]
    Refresh,
    #[serde(rename = "clipboard")]
    Clipboard { text: String },
}

pub fn parse_vnc_settings(value: &str) -> VncSettings {
    serde_json::from_str(value).unwrap_or(VncSettings {
        protocol: None,
        vnc_port: None,
        vnc_password: None,
        vnc_shared: None,
    })
}

pub fn vnc_port_from_settings(value: &str, _fallback: i32) -> u16 {
    let settings = parse_vnc_settings(value);
    settings
        .vnc_port
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_VNC_PORT)
}

fn rect_payload(session_id: &str, rect: Rect, data: Vec<u8>) -> VncFramePayload {
    VncFramePayload::Raw {
        session_id: session_id.to_string(),
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        data,
    }
}

fn emit_status(app: &AppHandle, session_id: &str, state: &str, message: impl Into<String>) {
    let _ = app.emit(
        &format!("vnc-status-{}", session_id),
        VncStatusPayload {
            session_id: session_id.to_string(),
            state: state.to_string(),
            message: message.into(),
        },
    );
}

fn emit_frame(app: &AppHandle, session_id: &str, payload: VncFramePayload) {
    let _ = app.emit(&format!("vnc-frame-{}", session_id), payload);
}

fn load_vnc_host_config(db: &Database, host_id: &str) -> Result<VncHostConfig, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let (host, port, rdp_settings): (String, i32, String) = conn
        .query_row(
            "SELECT ip, port, COALESCE(rdp_settings, '{}') FROM hosts WHERE id=?1",
            params![host_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| format!("host not found: {}", e))?;

    let settings = parse_vnc_settings(&rdp_settings);
    if settings.protocol.as_deref() != Some("vnc") {
        return Err("主机未配置为 VNC 远程桌面".to_string());
    }

    Ok(VncHostConfig {
        host,
        port: vnc_port_from_settings(&rdp_settings, port),
        password: settings.vnc_password.unwrap_or_default(),
        shared: settings.vnc_shared.unwrap_or(true),
    })
}

async fn connect_vnc_client(config: VncHostConfig) -> Result<VncClient, String> {
    let tcp = tokio::time::timeout(
        Duration::from_secs(12),
        TcpStream::connect((config.host.as_str(), config.port)),
    )
    .await
    .map_err(|_| "VNC TCP 连接超时".to_string())?
    .map_err(|e| format!("VNC TCP 连接失败: {}", e))?;

    let password = config.password.clone();
    tokio::time::timeout(
        Duration::from_secs(15),
        VncConnector::new(tcp)
            .set_auth_method(async move { Ok(password) })
            .add_encoding(VncEncoding::Zrle)
            .add_encoding(VncEncoding::CopyRect)
            .add_encoding(VncEncoding::Raw)
            .allow_shared(config.shared)
            .set_pixel_format(PixelFormat::rgba())
            .build()
            .map_err(|e| e.to_string())?
            .try_start(),
    )
    .await
    .map_err(|_| "VNC 握手超时".to_string())?
    .map_err(|e| format!("VNC 握手失败: {}", e))?
    .finish()
    .map_err(|e| format!("VNC 初始化失败: {}", e))
}

#[tauri::command]
pub async fn vnc_connect(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    manager: tauri::State<'_, VncManager>,
    host_id: String,
    session_id: String,
) -> Result<VncConnectResponse, String> {
    let config = load_vnc_host_config(&db, &host_id)?;
    emit_status(&app, &session_id, "connecting", "正在连接 VNC/RFB 服务");

    let client = connect_vnc_client(config).await?;

    let (stop_tx, mut stop_rx) = oneshot::channel();
    manager.sessions.insert(
        session_id.clone(),
        VncSession {
            client: client.clone(),
            stop: Arc::new(std::sync::Mutex::new(Some(stop_tx))),
        },
    );

    emit_status(
        &app,
        &session_id,
        "connected",
        "VNC 已连接，正在等待远程画面",
    );
    let app_for_loop = app.clone();
    let session_for_loop = session_id.clone();
    let client_for_loop = client.clone();
    tokio::spawn(async move {
        let mut refresh = tokio::time::interval(Duration::from_millis(33));
        let mut cleanup_required = true;
        loop {
            tokio::select! {
                _ = refresh.tick() => {
                    if let Err(error) = client_for_loop.input(X11Event::Refresh).await {
                        emit_status(&app_for_loop, &session_for_loop, "error", format!("VNC 刷新失败: {}", error));
                        break;
                    }
                    let mut should_stop = false;
                    loop {
                        match client_for_loop.poll_event().await {
                            Ok(Some(event)) => handle_vnc_event(&app_for_loop, &session_for_loop, event),
                            Ok(None) => break,
                            Err(error) => {
                                emit_status(&app_for_loop, &session_for_loop, "error", format!("VNC 连接中断: {}", error));
                                should_stop = true;
                                break;
                            }
                        }
                    }
                    if should_stop {
                        break;
                    }
                }
                _ = &mut stop_rx => {
                    cleanup_required = false;
                    let _ = client_for_loop.close().await;
                    emit_status(&app_for_loop, &session_for_loop, "disconnected", "VNC 已断开");
                    break;
                }
            }
        }
        if cleanup_required {
            let _ = app_for_loop
                .state::<VncManager>()
                .sessions
                .remove(&session_for_loop);
            let _ = client_for_loop.close().await;
        }
    });

    Ok(VncConnectResponse {
        session_id,
        host_id,
    })
}

fn handle_vnc_event(app: &AppHandle, session_id: &str, event: VncEvent) {
    match event {
        VncEvent::SetResolution(screen) => emit_frame(
            app,
            session_id,
            VncFramePayload::Resize {
                session_id: session_id.to_string(),
                width: screen.width,
                height: screen.height,
            },
        ),
        VncEvent::RawImage(rect, data) => {
            emit_frame(app, session_id, rect_payload(session_id, rect, data))
        }
        VncEvent::Copy(dst, src) => emit_frame(
            app,
            session_id,
            VncFramePayload::Copy {
                session_id: session_id.to_string(),
                dst_x: dst.x,
                dst_y: dst.y,
                src_x: src.x,
                src_y: src.y,
                width: dst.width,
                height: dst.height,
            },
        ),
        VncEvent::JpegImage(_, _) => emit_status(
            app,
            session_id,
            "connected",
            "收到 Tight JPEG 帧，当前版本暂未渲染该编码",
        ),
        VncEvent::Bell => emit_status(app, session_id, "connected", "远程 VNC 响铃"),
        VncEvent::Text(_) | VncEvent::SetCursor(_, _) | VncEvent::SetPixelFormat(_) => {}
        VncEvent::Error(error) => emit_status(app, session_id, "error", error),
        _ => {}
    }
}

#[tauri::command]
pub async fn vnc_send_input(
    manager: tauri::State<'_, VncManager>,
    session_id: String,
    event: VncInputEvent,
) -> Result<(), String> {
    let client = manager
        .sessions
        .get(&session_id)
        .ok_or_else(|| "VNC session not found".to_string())?
        .client
        .clone();
    let input = match event {
        VncInputEvent::Mouse { x, y, buttons } => X11Event::PointerEvent(ClientMouseEvent {
            position_x: x,
            position_y: y,
            bottons: buttons,
        }),
        VncInputEvent::Key { keycode, down } => {
            X11Event::KeyEvent(ClientKeyEvent { keycode, down })
        }
        VncInputEvent::Refresh => X11Event::FullRefresh,
        VncInputEvent::Clipboard { text } => X11Event::CopyText(text),
    };
    client
        .input(input)
        .await
        .map_err(|e| format!("VNC input failed: {}", e))
}

#[tauri::command]
pub async fn vnc_disconnect(
    manager: tauri::State<'_, VncManager>,
    session_id: String,
) -> Result<(), String> {
    if let Some((_, session)) = manager.sessions.remove(&session_id) {
        if let Some(stop) = session.stop.lock().map_err(|e| e.to_string())?.take() {
            let _ = stop.send(());
        }
        let _ = session.client.close().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn vnc_port_prefers_settings_over_host_port() {
        assert_eq!(
            5902,
            vnc_port_from_settings(r#"{"protocol":"vnc","vncPort":5902}"#, 22)
        );
    }

    #[test]
    fn vnc_port_falls_back_to_default_when_missing() {
        assert_eq!(
            DEFAULT_VNC_PORT,
            vnc_port_from_settings(r#"{"protocol":"vnc"}"#, 22)
        );
    }

    #[tokio::test]
    async fn vnc_client_connects_to_mock_rfb_server_and_receives_raw_frame() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock vnc server");
        let addr = listener.local_addr().expect("mock server addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept vnc client");

            socket
                .write_all(b"RFB 003.008\n")
                .await
                .expect("server version");
            let mut client_version = [0_u8; 12];
            socket
                .read_exact(&mut client_version)
                .await
                .expect("client version");
            assert_eq!(&client_version, b"RFB 003.008\n");

            socket.write_all(&[1, 1]).await.expect("security types");
            assert_eq!(socket.read_u8().await.expect("security choice"), 1);
            socket.write_u32(0).await.expect("security result");

            assert_eq!(socket.read_u8().await.expect("shared flag"), 1);
            socket.write_u16(1).await.expect("width");
            socket.write_u16(1).await.expect("height");
            socket
                .write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0])
                .await
                .expect("server pixel format");
            socket.write_u32(4).await.expect("name length");
            socket.write_all(b"mock").await.expect("name");

            assert_eq!(socket.read_u8().await.expect("set pixel format"), 0);
            let mut padding = [0_u8; 3];
            socket
                .read_exact(&mut padding)
                .await
                .expect("pixel format padding");
            let mut pixel_format = [0_u8; 16];
            socket
                .read_exact(&mut pixel_format)
                .await
                .expect("client pixel format");

            assert_eq!(socket.read_u8().await.expect("set encodings"), 2);
            assert_eq!(socket.read_u8().await.expect("encoding padding"), 0);
            let encoding_count = socket.read_u16().await.expect("encoding count");
            for _ in 0..encoding_count {
                let _ = socket.read_i32().await.expect("encoding");
            }

            assert_eq!(socket.read_u8().await.expect("frame request"), 3);
            let _incremental = socket.read_u8().await.expect("incremental flag");
            let _x = socket.read_u16().await.expect("x");
            let _y = socket.read_u16().await.expect("y");
            let _width = socket.read_u16().await.expect("request width");
            let _height = socket.read_u16().await.expect("request height");

            socket.write_u8(0).await.expect("framebuffer update");
            socket.write_u8(0).await.expect("frame padding");
            socket.write_u16(1).await.expect("rect count");
            socket.write_u16(0).await.expect("rect x");
            socket.write_u16(0).await.expect("rect y");
            socket.write_u16(1).await.expect("rect width");
            socket.write_u16(1).await.expect("rect height");
            socket.write_i32(0).await.expect("raw encoding");
            socket
                .write_all(&[255, 0, 0, 255])
                .await
                .expect("rgba pixel");
        });

        let client = connect_vnc_client(VncHostConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            password: String::new(),
            shared: true,
        })
        .await
        .expect("connect mock vnc");

        let resolution = client.recv_event().await.expect("resolution event");
        assert!(matches!(
            resolution,
            VncEvent::SetResolution(screen) if screen.width == 1 && screen.height == 1
        ));

        let frame = client.recv_event().await.expect("raw frame event");
        assert!(matches!(
            frame,
            VncEvent::RawImage(rect, data)
                if rect.x == 0
                    && rect.y == 0
                    && rect.width == 1
                    && rect.height == 1
                    && data == vec![255, 0, 0, 255]
        ));

        let _ = client.close().await;
        server.await.expect("mock server task");
    }
}
