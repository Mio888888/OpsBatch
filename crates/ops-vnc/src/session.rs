//! One live VNC session: connect (optionally through the host's proxy), then run the RFB reader in its own task and
//! translate UI commands into RFB messages. The UI talks to it with two channels.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::client::{connect, AuthPreference, Options, Rect, ServerEvent};
use crate::config::{VncAuthMethod, VncHostConfig};
use crate::keys;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub enum VncEvent {
    Connected { width: u16, height: u16, name: String, apple: bool },
    /// Updated rectangles with their BGRA pixels (row-major, tightly packed).
    Frame(Vec<(Rect, Vec<u8>)>),
    /// The desktop was resized; the next frame update covers the whole new area.
    Resized { width: u16, height: u16 },
    Clipboard(String),
    Bell,
    Closed { clean: bool, reason: Option<String> },
}

#[derive(Debug, Clone)]
pub enum VncCommand {
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { keysym: u32, down: bool },
    CtrlAltDel,
    Refresh,
    Clipboard(String),
    Disconnect,
}

pub struct VncSession {
    commands: mpsc::UnboundedSender<VncCommand>,
    pub events: flume::Receiver<VncEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl VncSession {
    pub fn send(&self, command: VncCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(VncCommand::Disconnect);
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for VncSession {
    fn drop(&mut self) {
        let _ = self.commands.send(VncCommand::Disconnect);
        self.task.abort();
    }
}

/// Start a session for a loaded host configuration.
pub fn start_session(config: VncHostConfig) -> VncSession {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = flume::unbounded();
    let task = ops_core::rt::spawn(async move {
        let result = run(config, cmd_rx, &event_tx).await;
        let closed = match result {
            Ok(()) => VncEvent::Closed { clean: true, reason: None },
            Err(e) => VncEvent::Closed { clean: false, reason: Some(e) },
        };
        let _ = event_tx.send(closed);
    });
    VncSession { commands: cmd_tx, events: event_rx, task }
}

/// Aborts the task when dropped, so cancelling the session future also stops the reader (and closes the socket).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn open_stream(config: &VncHostConfig) -> Result<tokio::net::TcpStream, String> {
    let (host, port, proxy) = (config.host.clone(), config.port, config.proxy.clone());
    let addr = format!("{host}:{port}");
    let std_tcp = tokio::task::spawn_blocking(move || ops_ssh::ssh::connect_tcp_stream(&host, port, proxy, CONNECT_TIMEOUT.as_secs()))
        .await
        .map_err(|e| format!("VNC TCP 连接线程异常 {addr}: {e}"))?
        .map_err(|e| format!("VNC TCP 连接失败 {addr}: {e}"))?;
    std_tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let tcp = tokio::net::TcpStream::from_std(std_tcp).map_err(|e| e.to_string())?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    Ok(tcp)
}

async fn run(config: VncHostConfig, mut commands: mpsc::UnboundedReceiver<VncCommand>, events: &flume::Sender<VncEvent>) -> Result<(), String> {
    let tcp = open_stream(&config).await?;
    let opts = Options {
        username: config.username.clone(),
        password: config.password.clone(),
        prefer: if config.auth_method == VncAuthMethod::AppleRemoteDesktop { AuthPreference::Ard } else { AuthPreference::Vnc },
        shared: config.shared,
    };
    let (info, mut reader, mut writer) = tokio::time::timeout(CONNECT_TIMEOUT, connect(tcp, &opts))
        .await
        .map_err(|_| "VNC 握手超时".to_string())?
        .map_err(|e| format!("{e:#}"))?;
    let _ = events.send(VncEvent::Connected { width: info.width, height: info.height, name: info.name, apple: info.apple });
    writer.request_update(false).await.map_err(|e| e.to_string())?;

    // The reader owns the framebuffer; it forwards what changed.
    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Result<VncEvent, String>>();
    let _reader_guard = AbortOnDrop(tokio::spawn(async move {
        loop {
            let event = match reader.next_event().await {
                Ok(ServerEvent::Update(rects)) => {
                    let fb = &reader.fb;
                    let regions = rects
                        .into_iter()
                        .filter(|r| r.w > 0 && r.h > 0)
                        .map(|r| {
                            let mut pixels = Vec::with_capacity(r.w as usize * r.h as usize * 4);
                            for row in 0..r.h as usize {
                                let o = ((r.y as usize + row) * fb.width as usize + r.x as usize) * 4;
                                pixels.extend_from_slice(&fb.data[o..o + r.w as usize * 4]);
                            }
                            (r, pixels)
                        })
                        .collect();
                    VncEvent::Frame(regions)
                }
                Ok(ServerEvent::Resized { width, height }) => VncEvent::Resized { width, height },
                Ok(ServerEvent::CutText(text)) => VncEvent::Clipboard(text),
                Ok(ServerEvent::Bell) => VncEvent::Bell,
                Ok(ServerEvent::ColorMap) => continue,
                Err(e) => {
                    let _ = read_tx.send(Err(e.to_string()));
                    return;
                }
            };
            if read_tx.send(Ok(event)).is_err() {
                return;
            }
        }
    }));

    let view_only = config.view_only;
    let result = loop {
        tokio::select! {
            msg = read_rx.recv() => match msg {
                Some(Ok(event)) => {
                    match &event {
                        VncEvent::Resized { width, height } => {
                            writer.set_size(*width, *height);
                            writer.request_update(false).await.map_err(|e| e.to_string())?;
                        }
                        VncEvent::Frame(_) => writer.request_update(true).await.map_err(|e| e.to_string())?,
                        _ => {}
                    }
                    if events.send(event).is_err() {
                        break Ok(());
                    }
                }
                Some(Err(e)) => break Err(e),
                None => break Err("VNC 连接已断开".to_string()),
            },
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { break Ok(()) };
                let sent = match cmd {
                    VncCommand::Disconnect => break Ok(()),
                    VncCommand::Refresh => writer.request_update(false).await,
                    _ if view_only => Ok(()),
                    VncCommand::Pointer { x, y, buttons } => writer.send_pointer(x, y, buttons).await,
                    VncCommand::Key { keysym, down } => writer.send_key(keysym, down).await,
                    VncCommand::CtrlAltDel => {
                        let mut r = Ok(());
                        for (sym, down) in keys::ctrl_alt_del() {
                            r = writer.send_key(sym, down).await;
                            if r.is_err() {
                                break;
                            }
                        }
                        r
                    }
                    VncCommand::Clipboard(text) => writer.send_cut_text(&text).await,
                };
                if let Err(e) = sent {
                    break Err(e.to_string());
                }
            }
        }
    };
    result
}
