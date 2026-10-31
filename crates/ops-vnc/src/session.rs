//! One live VNC session: connect (optionally through the host's proxy), then run the RFB reader in its own task and
//! translate UI commands into RFB messages. The UI talks to it with two channels.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::client::{
    connect, cut_text_message, key_message, pointer_message, Options, Rect, ServerEvent,
};
use crate::config::VncHostConfig;
use crate::keys;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub enum VncEvent {
    Connected {
        width: u16,
        height: u16,
        name: String,
        apple: bool,
    },
    /// Updated rectangles with their BGRA pixels (row-major, tightly packed).
    Frame(Vec<(Rect, Vec<u8>)>),
    /// The desktop was resized; the next frame update covers the whole new area.
    Resized {
        width: u16,
        height: u16,
    },
    Clipboard(String),
    Bell,
    Closed {
        clean: bool,
        reason: Option<String>,
    },
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
            Ok(()) => VncEvent::Closed {
                clean: true,
                reason: None,
            },
            Err(e) => VncEvent::Closed {
                clean: false,
                reason: Some(e),
            },
        };
        let _ = event_tx.send(closed);
    });
    VncSession {
        commands: cmd_tx,
        events: event_rx,
        task,
    }
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
    let std_tcp = tokio::task::spawn_blocking(move || {
        ops_ssh::ssh::connect_tcp_stream(&host, port, proxy, CONNECT_TIMEOUT.as_secs())
    })
    .await
    .map_err(|e| format!("VNC TCP 连接线程异常 {addr}: {e}"))?
    .map_err(|e| format!("VNC TCP 连接失败 {addr}: {e}"))?;
    std_tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let tcp = tokio::net::TcpStream::from_std(std_tcp).map_err(|e| e.to_string())?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    Ok(tcp)
}

async fn run(
    config: VncHostConfig,
    mut commands: mpsc::UnboundedReceiver<VncCommand>,
    events: &flume::Sender<VncEvent>,
) -> Result<(), String> {
    let tcp = open_stream(&config).await?;
    let opts = Options {
        username: config.username.clone(),
        password: config.password.clone(),
        shared: config.shared,
    };
    let (info, mut reader, mut writer) = tokio::time::timeout(CONNECT_TIMEOUT, connect(tcp, &opts))
        .await
        .map_err(|_| "VNC 握手超时".to_string())?
        .map_err(|e| format!("{e:#}"))?;
    let _ = events.send(VncEvent::Connected {
        width: info.width,
        height: info.height,
        name: info.name,
        apple: info.apple,
    });
    writer
        .request_update(false)
        .await
        .map_err(|e| e.to_string())?;

    // The reader owns the framebuffer; it forwards what changed. It also tells this loop the moment an update starts
    // arriving, and the next request goes out right then, so the server does not wait for decoding and the UI.
    let (want_tx, mut want_rx) = mpsc::unbounded_channel::<()>();
    reader.set_update_hook(move || {
        let _ = want_tx.send(());
    });
    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Result<VncEvent, String>>();
    let _reader_guard = AbortOnDrop(tokio::spawn(async move {
        loop {
            let event = match reader.next_event().await {
                Ok(ServerEvent::Update(rects)) => {
                    let fb = &reader.fb;
                    let regions = merge_rects(rects)
                        .into_iter()
                        .filter(|r| r.w > 0 && r.h > 0)
                        .map(|r| {
                            let mut pixels = Vec::with_capacity(r.w as usize * r.h as usize * 4);
                            for row in 0..r.h as usize {
                                let o =
                                    ((r.y as usize + row) * fb.width as usize + r.x as usize) * 4;
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
                    if let VncEvent::Resized { width, height } = &event {
                        writer.set_size(*width, *height);
                        writer.request_update(false).await.map_err(|e| e.to_string())?;
                    }
                    if events.send(event).is_err() {
                        break Ok(());
                    }
                }
                Some(Err(e)) => break Err(e),
                None => break Err("VNC 连接已断开".to_string()),
            },
            Some(()) = want_rx.recv() => {
                while want_rx.try_recv().is_ok() {}
                writer.request_update(true).await.map_err(|e| e.to_string())?;
            }
            cmd = commands.recv() => {
                let Some(first) = cmd else { break Ok(()) };
                // Everything that queued up while the last write was in flight goes out together.
                let mut queued = vec![first];
                while queued.len() < 512 {
                    match commands.try_recv() {
                        Ok(c) => queued.push(c),
                        Err(_) => break,
                    }
                }
                let (bytes, disconnect) = encode_commands(queued, |incremental| writer.update_request(incremental), view_only);
                if !bytes.is_empty() {
                    if let Err(e) = writer.send_raw(&bytes).await {
                        break Err(e.to_string());
                    }
                }
                if disconnect {
                    break Ok(());
                }
            }
        }
    };
    result
}

/// The wire bytes for a run of UI commands, and whether one of them asked to disconnect (anything after it is
/// dropped). Consecutive pointer moves with the same buttons collapse into the last one: the remote only needs to
/// know where the pointer ended up, and a slow link must not be left replaying every intermediate position.
pub(crate) fn encode_commands(
    commands: Vec<VncCommand>,
    update_request: impl Fn(bool) -> [u8; 10],
    view_only: bool,
) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut pointer: Option<(u16, u16, u8)> = None;
    let flush = |out: &mut Vec<u8>, pointer: &mut Option<(u16, u16, u8)>| {
        if let Some((x, y, buttons)) = pointer.take() {
            out.extend_from_slice(&pointer_message(x, y, buttons));
        }
    };
    let mut disconnect = false;
    for command in commands {
        match command {
            VncCommand::Disconnect => {
                disconnect = true;
                break;
            }
            VncCommand::Refresh => {
                flush(&mut out, &mut pointer);
                out.extend_from_slice(&update_request(false));
            }
            _ if view_only => {}
            VncCommand::Pointer { x, y, buttons } => match pointer {
                Some((_, _, b)) if b == buttons => pointer = Some((x, y, buttons)),
                _ => {
                    flush(&mut out, &mut pointer);
                    pointer = Some((x, y, buttons));
                }
            },
            VncCommand::Key { keysym, down } => {
                flush(&mut out, &mut pointer);
                out.extend_from_slice(&key_message(keysym, down));
            }
            VncCommand::CtrlAltDel => {
                flush(&mut out, &mut pointer);
                for (sym, down) in keys::ctrl_alt_del() {
                    out.extend_from_slice(&key_message(sym, down));
                }
            }
            VncCommand::Clipboard(text) => {
                flush(&mut out, &mut pointer);
                out.extend_from_slice(&cut_text_message(&text));
            }
        }
    }
    flush(&mut out, &mut pointer);
    (out, disconnect)
}

/// An update made of many small rectangles is forwarded as their bounding box when that costs little extra: one copy
/// and one UI patch instead of dozens. The pixels between the rectangles are already in the framebuffer, so the
/// box is always correct.
pub(crate) fn merge_rects(rects: Vec<Rect>) -> Vec<Rect> {
    if rects.len() < 4 {
        return rects;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut covered = 0u64;
    for r in rects.iter().filter(|r| r.w > 0 && r.h > 0) {
        x0 = x0.min(u32::from(r.x));
        y0 = y0.min(u32::from(r.y));
        x1 = x1.max(u32::from(r.x) + u32::from(r.w));
        y1 = y1.max(u32::from(r.y) + u32::from(r.h));
        covered += u64::from(r.w) * u64::from(r.h);
    }
    if x1 <= x0 || y1 <= y0 {
        return rects;
    }
    let bounding = u64::from(x1 - x0) * u64::from(y1 - y0);
    if rects.len() > 64 || bounding <= covered * 3 {
        vec![Rect {
            x: x0 as u16,
            y: y0 as u16,
            w: (x1 - x0) as u16,
            h: (y1 - y0) as u16,
        }]
    } else {
        rects
    }
}
