//! RDP and VNC windows through real (headless) windows: a mock RFB server over TCP, RDP error handling, frame
//! painting, mouse mapping, the AI agent against a mock model, and opening them from the sidebar.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Render, TestAppContext};
use ops_ai::client::{save_ai_config, AiConfig};
use ops_rdp::RdpInputEvent;
use ops_ssh::hosts::NewHost;
use opsbatch_app::ai::store::ai_store;
use opsbatch_app::rdp::window::{HostRequest, RdpState, RdpWindow};
use opsbatch_app::vnc::window::{VncHost, VncState, VncWindow};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use common::{serial, wait_until, Fixture};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn open_view<V: Render + 'static>(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> Entity<V>,
) -> (AnyWindowHandle, Entity<V>) {
    cx.update(|cx| {
        gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(800.)),
                })),
                ..Default::default()
            },
            cx,
            build,
        )
        .unwrap()
    })
}

fn win<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(handle, |_, w, cx| f(w, cx)).unwrap()
}

/// Like `wait_until`, but also moves the test clock so timers (watchdogs, pacing) fire.
fn wait_ticking(
    cx: &mut TestAppContext,
    what: &str,
    mut cond: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        cx.run_until_parked();
        if cond(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        cx.executor().advance_clock(Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn new_remote_host(
    name: &str,
    os: &str,
    rdp_settings: Option<String>,
    password: Option<&str>,
) -> NewHost {
    NewHost {
        password: password.map(str::to_string),
        rdp_settings,
        ..common::new_host(name, "127.0.0.1", os, None)
    }
}

fn add_host(fx: &Fixture, host: NewHost) -> String {
    let db = fx.ctx.db.clone();
    ops_core::rt::block_on(async move { ops_ssh::hosts::add_host(&db, host).await.unwrap().id })
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---------------------------------------------------------------------------
// mock RFB server
// ---------------------------------------------------------------------------

struct VncMock {
    port: u16,
    /// Raw bytes the client sent after the handshake.
    received: flume::Receiver<Vec<u8>>,
    /// Send server clipboard text to the client.
    cut_text: flume::Sender<String>,
    closed: flume::Receiver<()>,
}

/// RFB 3.8 with VNC authentication (password `pw12`), a 64x32 desktop and one raw update:
/// a 4x2 rectangle at (0,0) with pixel (B,G,R) = (i*10, i*10+1, i*10+2).
/// `silent` accepts the connection and then says nothing at all.
fn start_vnc_mock(silent: bool) -> VncMock {
    let listener = ops_core::rt::block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (rx_tx, received) = flume::unbounded();
    let (cut_text, cut_rx) = flume::unbounded::<String>();
    let (closed_tx, closed) = flume::unbounded();
    ops_core::rt::spawn(async move {
        let Ok((mut s, _)) = listener.accept().await else {
            return;
        };
        if silent {
            let mut b = [0u8; 1];
            let _ = s.read(&mut b).await;
            let _ = closed_tx.send(());
            return;
        }
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut ver = [0u8; 12];
        s.read_exact(&mut ver).await.unwrap();
        s.write_all(&[1, 2]).await.unwrap();
        let mut chosen = [0u8; 1];
        s.read_exact(&mut chosen).await.unwrap();
        assert_eq!(chosen[0], 2, "VNC authentication selected");
        let challenge = [7u8; 16];
        s.write_all(&challenge).await.unwrap();
        let mut resp = [0u8; 16];
        s.read_exact(&mut resp).await.unwrap();
        let ok = resp == ops_vnc::auth::vnc_auth_response("pw12", &challenge);
        s.write_u32(if ok { 0 } else { 1 }).await.unwrap();
        if !ok {
            s.write_u32(4).await.unwrap();
            s.write_all(b"nope").await.unwrap();
            return;
        }
        let mut shared = [0u8; 1];
        s.read_exact(&mut shared).await.unwrap();
        s.write_u16(64).await.unwrap();
        s.write_u16(32).await.unwrap();
        s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0])
            .await
            .unwrap();
        s.write_u32(4).await.unwrap();
        s.write_all(b"mock").await.unwrap();
        let mut setup = [0u8; 20 + 4 + 20 + 10];
        s.read_exact(&mut setup).await.unwrap();
        let mut u = vec![0u8, 0, 0, 1];
        for v in [0u16, 0, 4, 2] {
            u.extend_from_slice(&v.to_be_bytes());
        }
        u.extend_from_slice(&0i32.to_be_bytes());
        for i in 0..8u8 {
            u.extend_from_slice(&[i * 10, i * 10 + 1, i * 10 + 2, 0]);
        }
        s.write_all(&u).await.unwrap();
        let mut buf = [0u8; 256];
        loop {
            tokio::select! {
                n = s.read(&mut buf) => {
                    let n = n.unwrap_or(0);
                    if n == 0 {
                        let _ = closed_tx.send(());
                        return;
                    }
                    let _ = rx_tx.send(buf[..n].to_vec());
                }
                text = cut_rx.recv_async() => {
                    let Ok(text) = text else { return };
                    let mut m = vec![3u8, 0, 0, 0];
                    m.extend_from_slice(&(text.len() as u32).to_be_bytes());
                    m.extend_from_slice(text.as_bytes());
                    s.write_all(&m).await.unwrap();
                }
            }
        }
    });
    VncMock {
        port,
        received,
        cut_text,
        closed,
    }
}

fn drain(rx: &flume::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut all = Vec::new();
    while let Ok(m) = rx.try_recv() {
        all.extend(m);
    }
    all
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn vnc_settings(port: u16, view_only: bool) -> String {
    serde_json::json!({"protocol": "vnc", "vncPort": port, "vncPassword": "pw12", "vncAuthMethod": "vnc", "vncViewOnly": view_only, "vncShared": true}).to_string()
}

fn open_vnc(cx: &mut TestAppContext, id: &str, port: u16) -> (AnyWindowHandle, Entity<VncWindow>) {
    let host = VncHost {
        host_id: id.into(),
        name: "vnc-box".into(),
        ip: "127.0.0.1".into(),
        port,
    };
    open_view(cx, move |window, cx| {
        cx.new(|cx| VncWindow::new(Some(host), String::new(), window, cx))
    })
}

// ---------------------------------------------------------------------------
// VNC
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn vnc_window_connects_paints_forwards_input_and_syncs_the_clipboard(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(false);
    let id = add_host(
        &fx,
        new_remote_host("vnc-box", "vnc", Some(vnc_settings(mock.port, false)), None),
    );
    let (handle, view) = open_vnc(cx, &id, mock.port);

    wait_until(cx, "connected with a first frame", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == VncState::Connected && v.surface().has_frame()
        })
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.desktop(), (64, 32));
        assert_eq!(v.surface().pixel(0, 0), Some([0, 1, 2, 255]));
        assert_eq!(v.surface().pixel(3, 1), Some([70, 71, 72, 255]));
        assert_eq!(v.surface().pixel(10, 10), Some([0, 0, 0, 0]));
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("vnc-stage").is_some() && w.try_find("vnc-status").is_some());
        assert!(w.try_find("vnc-connecting").is_none() && w.try_find("vnc-error").is_none());
    });
    let _ = drain(&mock.received);

    // Pointer: click on the stage (focuses it too).
    win(cx, handle, |w, cx| w.click("vnc-stage", cx));
    let mut pointer = Vec::new();
    wait_until(cx, "pointer press and release", |_| {
        pointer.extend(drain(&mock.received));
        contains(&pointer, &[5, 1]) && contains(&pointer, &[5, 0])
    });
    // The click lands in the middle of the 64x32 desktop (x=32, y=16, give or take scaling rounding).
    let at = pointer
        .windows(6)
        .position(|w| w[0] == 5 && w[1] == 1)
        .unwrap();
    let (x, y) = (
        u16::from_be_bytes([pointer[at + 2], pointer[at + 3]]),
        u16::from_be_bytes([pointer[at + 4], pointer[at + 5]]),
    );
    assert!(
        (28..=36).contains(&x) && (12..=20).contains(&y),
        "centre of the desktop, got {x},{y}"
    );

    // Keyboard goes to the focused stage.
    win(cx, handle, |w, cx| w.press("a", cx));
    let mut keys = Vec::new();
    wait_until(cx, "key events", |_| {
        keys.extend(drain(&mock.received));
        contains(&keys, &[4, 1, 0, 0, 0, 0, 0, 0x61])
            && contains(&keys, &[4, 0, 0, 0, 0, 0, 0, 0x61])
    });

    // Ctrl+Alt+Del button.
    win(cx, handle, |w, cx| w.click("vnc-cad", cx));
    let mut chord = Vec::new();
    wait_until(cx, "ctrl-alt-del", |_| {
        chord.extend(drain(&mock.received));
        contains(&chord, &[4, 1, 0, 0, 0, 0, 0xff, 0xff])
            && contains(&chord, &[4, 1, 0, 0, 0, 0, 0xff, 0xe3])
    });

    // Remote clipboard lands in the local one.
    mock.cut_text.send("from server".into()).unwrap();
    wait_until(cx, "clipboard from server", |cx| {
        cx.update(|cx| {
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref() == Some("from server")
        })
    });

    // Disconnect closes the window and the socket.
    win(cx, handle, |w, cx| w.click("vnc-disconnect", cx));
    wait_until(cx, "socket closed after disconnect", |_| {
        mock.closed.try_recv().is_ok()
    });
}

#[gpui_kit::test]
fn pasting_sends_the_local_clipboard_first_and_releases_command_chords(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(false);
    let id = add_host(
        &fx,
        new_remote_host("vnc-box", "vnc", Some(vnc_settings(mock.port, false)), None),
    );
    let (handle, view) = open_vnc(cx, &id, mock.port);
    wait_until(cx, "connected", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == VncState::Connected && v.surface().has_frame()
        })
    });
    win(cx, handle, |w, cx| w.click("vnc-stage", cx));
    let _ = drain(&mock.received);

    // Copy something locally and paste at once, well inside the half-second clipboard poll.
    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("from the mac".into()))
    });
    win(cx, handle, |w, cx| w.press("cmd-v", cx));
    let mut sent = Vec::new();
    wait_until(cx, "clipboard and the v key reach the server", |_| {
        sent.extend(drain(&mock.received));
        contains(&sent, b"from the mac")
            && contains(&sent, &[4, 1, 0, 0, 0, 0, 0, 0x76])
            && contains(&sent, &[4, 0, 0, 0, 0, 0, 0, 0x76])
    });
    let at = |needle: &[u8]| {
        sent.windows(needle.len())
            .position(|w| w == needle)
            .unwrap()
    };
    assert!(
        at(b"from the mac") < at(&[4, 1, 0, 0, 0, 0, 0, 0x76]),
        "the remote must have the text before the paste key arrives"
    );
    // a ClientCutText message: type 6, three padding bytes, the length
    assert_eq!(
        &sent[at(b"from the mac") - 8..at(b"from the mac")],
        &[6, 0, 0, 0, 0, 0, 0, 12]
    );

    // The same text is not sent twice by the poll afterwards.
    cx.executor().advance_clock(Duration::from_millis(1200));
    cx.run_until_parked();
    assert!(!contains(&drain(&mock.received), b"from the mac"));

    win(cx, handle, |w, cx| w.click("vnc-disconnect", cx));
}

/// The `(type, first byte)` of every client message in a stream of RFB client messages.
fn rfb_messages(mut bytes: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    while let Some(&kind) = bytes.first() {
        let len = match kind {
            3 => 10,
            4 | 6 if bytes.len() >= 8 && kind == 4 => 8,
            5 => 6,
            6 if bytes.len() >= 8 => {
                8 + u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize
            }
            _ => break,
        };
        if bytes.len() < len {
            break;
        }
        out.push((kind, bytes[1]));
        bytes = &bytes[len..];
    }
    out
}

fn pointer_buttons(bytes: &[u8]) -> Vec<u8> {
    rfb_messages(bytes)
        .into_iter()
        .filter(|(kind, _)| *kind == 5)
        .map(|(_, buttons)| buttons)
        .collect()
}

#[gpui_kit::test]
fn vnc_scrolls_in_whole_notches_and_never_leaves_a_button_held(cx: &mut TestAppContext) {
    use gpui_kit::{
        point, px, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
        ScrollDelta,
    };
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(false);
    let id = add_host(
        &fx,
        new_remote_host("vnc-box", "vnc", Some(vnc_settings(mock.port, false)), None),
    );
    let (handle, view) = open_vnc(cx, &id, mock.port);
    wait_until(cx, "connected", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == VncState::Connected && v.surface().has_frame()
        })
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("vnc-info").is_some(),
            "the bar shows the desktop size once connected"
        );
        w.click("vnc-stage", cx);
    });
    let settle = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(100));
    };
    settle(cx);
    let _ = drain(&mock.received);

    // A trackpad delivers small deltas: nothing goes out until they add up to one notch (20 px).
    for _ in 0..3 {
        win(cx, handle, |w, cx| {
            w.scroll("vnc-stage", ScrollDelta::Pixels(point(px(0.), px(5.))), cx)
        });
    }
    settle(cx);
    assert!(
        !pointer_buttons(&drain(&mock.received)).contains(&8),
        "15 px is not a notch yet"
    );
    win(cx, handle, |w, cx| {
        w.scroll("vnc-stage", ScrollDelta::Pixels(point(px(0.), px(5.))), cx)
    });
    let mut sent = Vec::new();
    wait_until(cx, "one wheel-up notch", |_| {
        sent.extend(drain(&mock.received));
        pointer_buttons(&sent).windows(2).any(|w| w == [8, 0])
    });
    assert_eq!(
        pointer_buttons(&sent).iter().filter(|b| **b == 8).count(),
        1,
        "exactly one notch"
    );
    // A mouse wheel click (one line) is one notch in the other direction; sideways goes to the horizontal buttons.
    win(cx, handle, |w, cx| {
        w.scroll("vnc-stage", ScrollDelta::Lines(point(0., -1.)), cx)
    });
    win(cx, handle, |w, cx| {
        w.scroll("vnc-stage", ScrollDelta::Lines(point(1., 0.)), cx)
    });
    let mut wheel = Vec::new();
    wait_until(cx, "down and left notches", |_| {
        wheel.extend(drain(&mock.received));
        let b = pointer_buttons(&wheel);
        b.contains(&16) && b.contains(&32)
    });

    // Press inside the picture, let go outside of it: the remote still gets the release.
    let centre = win(cx, handle, |w, _| w.find("vnc-stage").bounds().center());
    let _ = drain(&mock.received);
    win(cx, handle, |w, cx| {
        w.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: centre,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        )
    });
    win(cx, handle, |w, cx| {
        w.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position: point(px(4.), px(4.)),
                modifiers: Modifiers::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        )
    });
    let mut drag = Vec::new();
    wait_until(cx, "release outside the picture", |_| {
        drag.extend(drain(&mock.received));
        pointer_buttons(&drag).windows(2).any(|w| w == [1, 0])
    });

    // Press and lose focus (Cmd+Tab, another control): everything held is released too (the right button here).
    let _ = drain(&mock.received);
    win(cx, handle, |w, cx| {
        w.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Right,
                position: centre,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        )
    });
    // (GPUI reports focus loss only for an active window, which a headless test window never is: call what it calls)
    view.update(cx, |v, _| v.release_all_input());
    let mut blurred = Vec::new();
    wait_until(cx, "release after losing focus", |_| {
        blurred.extend(drain(&mock.received));
        pointer_buttons(&blurred).windows(2).any(|w| w == [4, 0])
    });

    win(cx, handle, |w, cx| w.click("vnc-disconnect", cx));
}

#[gpui_kit::test]
fn vnc_pointer_moves_go_out_at_once_and_only_changed_tiles_are_rebuilt(cx: &mut TestAppContext) {
    use gpui_kit::{point, px, InputEvent as _, Modifiers, MouseMoveEvent};
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(false);
    let id = add_host(
        &fx,
        new_remote_host("vnc-box", "vnc", Some(vnc_settings(mock.port, false)), None),
    );
    let (handle, view) = open_vnc(cx, &id, mock.port);
    wait_until(cx, "connected", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == VncState::Connected && v.surface().has_frame()
        })
    });
    win(cx, handle, |w, cx| w.render_frame(cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.tile_images()),
        1,
        "a 64x32 desktop is one tile"
    );

    // A move is on the wire without any frame being drawn in between, and a stream of them ends where the mouse ends.
    let _ = drain(&mock.received);
    let bounds = win(cx, handle, |w, _| w.find("vnc-stage").bounds());
    for i in 0..20 {
        let position = point(
            bounds.origin.x + px(10. + i as f32),
            bounds.origin.y + px(10.),
        );
        win(cx, handle, |w, cx| {
            w.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            )
        });
    }
    let mut sent = Vec::new();
    wait_until(cx, "moves reach the server", |_| {
        sent.extend(drain(&mock.received));
        pointer_buttons(&sent).len() >= 1
    });
    let positions = |bytes: &[u8]| {
        let (mut at, mut out) = (0, Vec::new());
        while at < bytes.len() {
            match bytes[at] {
                3 => at += 10,
                4 => at += 8,
                5 if at + 6 <= bytes.len() => {
                    out.push(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
                    at += 6;
                }
                _ => break,
            }
        }
        out
    };
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    sent.extend(drain(&mock.received));
    let xs = positions(&sent);
    assert!(!xs.is_empty() && xs.len() <= 20, "{xs:?}");
    assert_eq!(
        xs.last(),
        xs.iter().max(),
        "the last position is where the mouse ended: {xs:?}"
    );
    assert!(
        xs.windows(2).all(|w| w[0] <= w[1]),
        "positions only move forward: {xs:?}"
    );
    win(cx, handle, |w, cx| w.click("vnc-disconnect", cx));
}

#[gpui_kit::test]
fn vnc_view_only_hosts_never_send_input(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(false);
    let id = add_host(
        &fx,
        new_remote_host("vnc-ro", "vnc", Some(vnc_settings(mock.port, true)), None),
    );
    let (handle, view) = open_vnc(cx, &id, mock.port);
    wait_until(cx, "connected", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == VncState::Connected && v.surface().has_frame()
        })
    });
    assert!(view.read_with(cx, |v, _| v.view_only()));
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        w.click("vnc-stage", cx);
        w.press("a", cx);
        w.click("vnc-cad", cx); // disabled
    });
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    let got = drain(&mock.received);
    assert!(
        !contains(&got, &[5, 1]) && !contains(&got, &[4, 1]),
        "no pointer or key messages in view-only mode: {got:?}"
    );
}

#[gpui_kit::test]
fn vnc_connection_failures_show_the_error_card_and_can_be_retried(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let port = free_port();
    let id = add_host(
        &fx,
        new_remote_host("vnc-down", "vnc", Some(vnc_settings(port, false)), None),
    );
    let (handle, view) = open_vnc(cx, &id, port);
    wait_until(cx, "error", |cx| {
        view.read_with(cx, |v, _| v.state() == VncState::Error)
    });
    view.read_with(cx, |v, _| {
        assert!(
            v.status_message()
                .unwrap_or("")
                .contains("VNC TCP 连接失败"),
            "{:?}",
            v.status_message()
        )
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("vnc-error").is_some());
        w.click("vnc-reconnect-card", cx);
        w.render_frame(cx);
        assert!(w.try_find("vnc-connecting").is_some() || w.try_find("vnc-error").is_some());
    });
    wait_until(cx, "error again", |cx| {
        view.read_with(cx, |v, _| v.state() == VncState::Error)
    });

    // A host that is not configured for VNC is rejected before any connection.
    let rdp = add_host(&fx, new_remote_host("plain", "vnc", None, None));
    let (_h2, v2) = open_vnc(cx, &rdp, 5900);
    wait_until(cx, "not a vnc host", |cx| {
        v2.read_with(cx, |v, _| v.state() == VncState::Error)
    });
    v2.read_with(cx, |v, _| {
        assert_eq!(v.status_message(), Some("主机未配置为 VNC 远程桌面"))
    });
}

#[gpui_kit::test]
fn vnc_handshake_that_never_finishes_times_out(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let mock = start_vnc_mock(true);
    let id = add_host(
        &fx,
        new_remote_host(
            "vnc-silent",
            "vnc",
            Some(vnc_settings(mock.port, false)),
            None,
        ),
    );
    let (_handle, view) = open_vnc(cx, &id, mock.port);
    wait_ticking(cx, "watchdog", |cx| {
        view.read_with(cx, |v, _| v.state() == VncState::Error)
    });
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.status_message(),
            Some("VNC 连接超时，请检查主机地址、端口、防火墙或服务状态后重试。")
        )
    });
    wait_until(cx, "session dropped", |_| mock.closed.try_recv().is_ok());
}

// ---------------------------------------------------------------------------
// RDP
// ---------------------------------------------------------------------------

fn frame_message(desktop: (u16, u16), x: u16, y: u16, w: u16, h: u16, rgba: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    for v in [desktop.0, desktop.1, x, y, w, h] {
        m.extend_from_slice(&v.to_le_bytes());
    }
    m.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    m.extend_from_slice(rgba);
    m
}

fn open_rdp(cx: &mut TestAppContext, id: &str) -> (AnyWindowHandle, Entity<RdpWindow>) {
    let host = HostRequest {
        host_id: id.into(),
        name: "win-1".into(),
        ip: "127.0.0.1".into(),
    };
    open_view(cx, move |window, cx| {
        cx.new(|cx| RdpWindow::new(Some(host), String::new(), window, cx))
    })
}

#[gpui_kit::test]
fn rdp_window_without_a_host_offers_the_asset_manager(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let (handle, view) = open_view(cx, |window, cx| {
        cx.new(|cx| RdpWindow::new(None, "h-1".into(), window, cx))
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-open-assets").is_some());
        assert!(w.try_find("rdp-stage").is_none());
    });
    view.read_with(cx, |v, _| assert_eq!(v.state(), RdpState::Idle));
}

#[gpui_kit::test]
fn rdp_connection_errors_surface_in_the_window(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let port = free_port();
    let settings = serde_json::json!({"protocol": "rdp"}).to_string();
    let id = add_host(
        &fx,
        NewHost {
            port: Some(port as i32),
            ..new_remote_host("win-down", "windows", Some(settings), Some("secret"))
        },
    );
    let (handle, view) = open_rdp(cx, &id);
    wait_until(cx, "error", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    view.read_with(cx, |v, _| {
        assert!(
            v.status_message()
                .unwrap_or("")
                .contains("RDP TCP 连接失败"),
            "{:?}",
            v.status_message()
        )
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-error").is_some());
        w.click("rdp-reconnect", cx);
    });
    wait_until(cx, "error again after reconnect", |cx| {
        view.read_with(cx, |v, _| {
            v.state() == RdpState::Error && v.session_id().is_some()
        })
    });

    // Non-Windows hosts are refused by the backend.
    let linux = add_host(&fx, new_remote_host("lin", "linux", None, Some("x")));
    let (_h, v2) = open_rdp(cx, &linux);
    wait_until(cx, "refused", |cx| {
        v2.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    v2.read_with(cx, |v, _| {
        assert_eq!(v.status_message(), Some("RDP 连接仅支持 Windows 主机"))
    });
}

#[gpui_kit::test]
fn rdp_prefers_h264_and_falls_back_to_a_fresh_bitmap_session(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let port = free_port();
    let settings = serde_json::json!({"protocol": "rdp"}).to_string();
    let id = add_host(
        &fx,
        NewHost {
            port: Some(port as i32),
            ..new_remote_host("win-h264", "windows", Some(settings), Some("secret"))
        },
    );
    let (handle, view) = open_rdp(cx, &id);
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.transport(),
            ops_rdp::RdpTransportMode::H264Direct,
            "fresh connections ask for H.264"
        )
    });
    view.read_with(cx, |v, _| {
        assert!(!v.ai_open(), "the assistant starts closed")
    });
    wait_until(cx, "first attempt fails", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });

    // An unrelated error (bad password, unreachable port…) never triggers a second login attempt.
    let first_sid = view.read_with(cx, |v, _| v.session_id().unwrap().to_string());
    fx.ctx.core.emit(
        &format!("rdp-status-{first_sid}"),
        ops_rdp::RdpStatusPayload {
            session_id: first_sid.clone(),
            state: "error".into(),
            message: Some("认证失败".into()),
            detail: None,
        },
    );
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(
            (v.transport(), v.fell_back()) == (ops_rdp::RdpTransportMode::H264Direct, false)
                && v.session_id() == Some(first_sid.as_str())
        )
    });

    // The backend says H.264 is unavailable -> new session over bitmap, with an explanation.
    fx.ctx.core.emit(
        &format!("rdp-status-{first_sid}"),
        ops_rdp::RdpStatusPayload {
            session_id: first_sid.clone(),
            state: "h264DirectUnavailable".into(),
            message: Some("服务器未协商 EGFX。".into()),
            detail: Some(ops_rdp::RdpStatusDetail::H264DirectUnavailable {
                reason: "服务器未协商 EGFX。".into(),
            }),
        },
    );
    wait_until(cx, "fallback session", |cx| {
        view.read_with(cx, |v, _| {
            v.fell_back() && v.session_id().is_some_and(|s| s != first_sid)
        })
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.transport(), ops_rdp::RdpTransportMode::LegacyBitmap);
        assert!(
            v.toast().unwrap_or("").contains("正在切换到 legacy bitmap"),
            "{:?}",
            v.toast()
        );
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-meta").is_some());
    });

    // The host is remembered: connecting again goes straight to bitmap instead of trying EGFX a second time.
    win(cx, handle, |w, cx| w.click("rdp-reconnect", cx));
    view.read_with(cx, |v, _| {
        assert!(
            v.transport() == ops_rdp::RdpTransportMode::LegacyBitmap && !v.fell_back(),
            "no second H.264 attempt for a server without AVC"
        )
    });
    wait_until(cx, "error after reconnect", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    // After forgetting it, reconnect starts over with H.264.
    opsbatch_app::rdp::window::forget_no_avc(&id);
    win(cx, handle, |w, cx| w.click("rdp-reconnect", cx));
    view.read_with(cx, |v, _| {
        assert!(v.transport() == ops_rdp::RdpTransportMode::H264Direct && !v.fell_back())
    });

    // A decoder failure that ends an H.264 session also falls back.
    wait_until(cx, "error after reconnect", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    let sid = view.read_with(cx, |v, _| v.session_id().unwrap().to_string());
    fx.ctx.core.emit(
        &format!("rdp-status-{sid}"),
        ops_rdp::RdpStatusPayload {
            session_id: sid.clone(),
            state: "error".into(),
            message: Some("H.264 解码失败: bad nal".into()),
            detail: None,
        },
    );
    wait_until(cx, "decoder fallback", |cx| {
        view.read_with(cx, |v, _| {
            v.fell_back() && v.session_id().is_some_and(|s| s != sid)
        })
    });
}

#[gpui_kit::test]
fn a_dropped_connection_is_retried_a_few_times_but_other_errors_are_not(cx: &mut TestAppContext) {
    use opsbatch_app::rdp::window::{is_connection_lost, reconnect_delay, MAX_AUTO_RECONNECTS};
    assert!(
        is_connection_lost("读取 RDP 帧失败: connection reset")
            && !is_connection_lost("RDP NLA/CredSSP 握手失败: 认证失败")
    );
    assert_eq!(
        [1, 2, 3, 4].map(|n| reconnect_delay(n).as_secs()),
        [1, 2, 4, 8]
    );
    assert_eq!(MAX_AUTO_RECONNECTS, 3);

    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let port = free_port();
    let settings = serde_json::json!({"protocol": "rdp"}).to_string();
    let id = add_host(
        &fx,
        NewHost {
            port: Some(port as i32),
            ..new_remote_host("win-drop", "windows", Some(settings), Some("secret"))
        },
    );
    let (_handle, view) = open_rdp(cx, &id);
    wait_until(cx, "first attempt fails", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    let emit = |sid: &str, state: &str, message: Option<&str>| {
        fx.ctx.core.emit(
            &format!("rdp-status-{sid}"),
            ops_rdp::RdpStatusPayload {
                session_id: sid.into(),
                state: state.into(),
                message: message.map(Into::into),
                detail: None,
            },
        );
    };

    // A drop before the session ever came up is a connection failure, not something to retry silently.
    let sid = view.read_with(cx, |v, _| v.session_id().unwrap().to_string());
    emit(&sid, "error", Some("读取 RDP 帧失败: early eof"));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.state() == RdpState::Error && v.session_id() == Some(sid.as_str()))
    });

    // Once connected, a lost network starts a countdown and then a fresh session.
    emit(&sid, "connected", None);
    cx.run_until_parked();
    emit(&sid, "error", Some("读取 RDP 帧失败: connection reset"));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.state(), RdpState::Connecting);
        assert!(
            v.status_message().unwrap_or("").contains("自动重连（1/3）"),
            "{:?}",
            v.status_message()
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    wait_until(cx, "second session", |cx| {
        view.read_with(cx, |v, _| v.session_id().is_some_and(|s| s != sid))
    });

    // Authentication problems never retry on their own.
    wait_until(cx, "second attempt fails", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    let sid2 = view.read_with(cx, |v, _| v.session_id().unwrap().to_string());
    emit(&sid2, "error", Some("RDP NLA/CredSSP 握手失败: 认证失败"));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.state() == RdpState::Error && v.session_id() == Some(sid2.as_str()))
    });
}

#[gpui_kit::test]
fn rdp_frames_are_painted_mapped_and_input_is_forwarded(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let manager = opsbatch_app::rdp::global_manager();
    let sid = "rdp-test-input";
    let input = manager.register_test_session(sid);
    let (handle, view) = open_view(cx, |window, cx| {
        cx.new(|cx| {
            RdpWindow::new(
                Some(HostRequest {
                    host_id: "none".into(),
                    name: "win-1".into(),
                    ip: "10.0.0.5".into(),
                }),
                String::new(),
                window,
                cx,
            )
        })
    });
    // The connection to host "none" fails; swap in the fake session afterwards.
    wait_until(cx, "initial attempt finished", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    view.update(cx, |v, cx| v.debug_attach_test_session(sid.into(), cx));
    win(cx, handle, |w, cx| w.render_frame(cx));
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("rdp-overlay-title").is_some(),
            "connected without a picture yet"
        );
    });

    // A full frame and a partial one (RGBA in, shown as BGRA).
    let full: Vec<u8> = (0..800 * 600).flat_map(|_| [10u8, 20, 30, 255]).collect();
    view.update(cx, |v, cx| {
        v.inject_frame(frame_message((800, 600), 0, 0, 800, 600, &full), cx)
    });
    view.update(cx, |v, cx| {
        v.inject_frame(frame_message((800, 600), 5, 6, 1, 1, &[1, 2, 3, 255]), cx)
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.surface().pixel(0, 0), Some([30, 20, 10, 255]));
        assert_eq!(v.surface().pixel(5, 6), Some([3, 2, 1, 255]));
        assert_eq!(v.desktop(), (800, 600));
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("rdp-overlay-title").is_none(),
            "the first picture hides the waiting overlay"
        );
        w.click("rdp-stage", cx);
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.tile_images()),
        12,
        "an 800x600 picture is 4 x 3 tiles of 256 pixels"
    );
    // The click (stage centre) is forwarded as press + release at the middle of the desktop area.
    let mut seen = Vec::new();
    wait_until(cx, "mouse events", |_| {
        while let Ok(e) = input.try_recv() {
            seen.push(e);
        }
        seen.iter().any(|e| {
            matches!(
                e,
                RdpInputEvent::MouseButton {
                    button: 0,
                    down: true,
                    ..
                }
            )
        }) && seen.iter().any(|e| {
            matches!(
                e,
                RdpInputEvent::MouseButton {
                    button: 0,
                    down: false,
                    ..
                }
            )
        })
    });
    let (x, y) = seen
        .iter()
        .find_map(|e| {
            if let RdpInputEvent::MouseButton {
                x, y, down: true, ..
            } = e
            {
                Some((*x, *y))
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        x > 300 && x < 500 && y > 200 && y < 400,
        "centre of an 800x600 desktop, got {x},{y}"
    );

    win(cx, handle, |w, cx| w.press("a", cx));
    let mut keys = Vec::new();
    wait_until(cx, "key events", |_| {
        while let Ok(e) = input.try_recv() {
            keys.push(e);
        }
        keys.contains(&RdpInputEvent::KeyScancode {
            code: 0x1e,
            extended: false,
            down: true,
        }) && keys.contains(&RdpInputEvent::KeyScancode {
            code: 0x1e,
            extended: false,
            down: false,
        })
    });
    let shot = view
        .read_with(cx, |v, _| v.screenshot())
        .expect("a screenshot once there is a picture");
    assert!(shot.starts_with("data:image/jpeg;base64,"));
    manager.disconnect(sid).unwrap();
}

#[gpui_kit::test]
fn rdp_side_buttons_the_wheel_and_the_input_method_reach_the_server(cx: &mut TestAppContext) {
    use gpui_kit::test::ClickOptions;
    use gpui_kit::{
        point, px, EntityInputHandler as _, MouseButton, NavigationDirection, ScrollDelta,
    };
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let manager = opsbatch_app::rdp::global_manager();
    let sid = "rdp-test-input-details";
    let input = manager.register_test_session(sid);
    let (handle, view) = open_view(cx, |window, cx| {
        cx.new(|cx| {
            RdpWindow::new(
                Some(HostRequest {
                    host_id: "none".into(),
                    name: "win-1".into(),
                    ip: "10.0.0.5".into(),
                }),
                String::new(),
                window,
                cx,
            )
        })
    });
    wait_until(cx, "initial attempt finished", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    // before a session is attached the window takes no text from an input method
    win(cx, handle, |w, cx| {
        let accepts = view.update(cx, |v, cx| v.accepts_text_input(w, cx));
        assert!(
            !accepts,
            "no input method while the window is not connected"
        );
    });
    view.update(cx, |v, cx| v.debug_attach_test_session(sid.into(), cx));
    let full: Vec<u8> = (0..800 * 600).flat_map(|_| [10u8, 20, 30, 255]).collect();
    view.update(cx, |v, cx| {
        v.inject_frame(frame_message((800, 600), 0, 0, 800, 600, &full), cx)
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
    });
    let drain_events =
        |cx: &mut TestAppContext, until: &dyn Fn(&[RdpInputEvent]) -> bool| -> Vec<RdpInputEvent> {
            let mut seen = Vec::new();
            wait_until(cx, "input events", |_| {
                while let Ok(e) = input.try_recv() {
                    seen.push(e);
                }
                until(&seen)
            });
            seen
        };

    // the thumb buttons of a mouse are X1 (back) and X2 (forward)
    for (direction, code) in [
        (NavigationDirection::Back, 3u8),
        (NavigationDirection::Forward, 4u8),
    ] {
        win(cx, handle, |w, cx| {
            w.click_with_options(
                "rdp-stage",
                ClickOptions::new().with_button(MouseButton::Navigate(direction)),
                cx,
            )
        });
        let seen = drain_events(cx, &|seen| {
            seen.iter().any(|e| matches!(e, RdpInputEvent::MouseButton { button, down: true, .. } if *button == code)) && seen.iter().any(|e| matches!(e, RdpInputEvent::MouseButton { button, down: false, .. } if *button == code))
        });
        assert!(
            seen.iter()
                .all(|e| !matches!(e, RdpInputEvent::MouseButton { button: 0..=2, .. })),
            "no left click is faked: {seen:?}"
        );
    }

    // a mouse wheel click is one notch; sideways it is the other way round
    win(cx, handle, |w, cx| {
        w.scroll("rdp-stage", ScrollDelta::Lines(point(0., 1.)), cx)
    });
    let seen = drain_events(cx, &|s| {
        s.iter().any(|e| matches!(e, RdpInputEvent::Wheel { .. }))
    });
    assert!(
        seen.iter().any(|e| matches!(
            e,
            RdpInputEvent::Wheel {
                delta: 120,
                vertical: true,
                ..
            }
        )),
        "{seen:?}"
    );
    win(cx, handle, |w, cx| {
        w.scroll("rdp-stage", ScrollDelta::Lines(point(1., 0.)), cx)
    });
    let seen = drain_events(cx, &|s| {
        s.iter().any(|e| matches!(e, RdpInputEvent::Wheel { .. }))
    });
    assert!(
        seen.iter().any(|e| matches!(
            e,
            RdpInputEvent::Wheel {
                delta: -120,
                vertical: false,
                ..
            }
        )),
        "{seen:?}"
    );
    // a trackpad: small movements add up to a notch instead of one notch each
    for _ in 0..4 {
        win(cx, handle, |w, cx| {
            w.scroll("rdp-stage", ScrollDelta::Pixels(point(px(0.), px(-6.))), cx)
        });
    }
    let seen = drain_events(cx, &|s| {
        s.iter().any(|e| matches!(e, RdpInputEvent::Wheel { .. }))
    });
    let wheel: Vec<_> = seen
        .iter()
        .filter(|e| matches!(e, RdpInputEvent::Wheel { .. }))
        .collect();
    assert_eq!(wheel.len(), 1, "4 x 6 px = one notch: {seen:?}");
    assert!(matches!(
        wheel[0],
        RdpInputEvent::Wheel {
            delta: -120,
            vertical: true,
            ..
        }
    ));

    // the input method: composing text is shown, the committed text is typed as Unicode
    win(cx, handle, |w, cx| {
        assert!(view.update(cx, |v, cx| v.accepts_text_input(w, cx)));
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "ni hao", None, w, cx)
        });
        w.render_frame(cx);
        assert!(
            w.try_find("rdp-composition").is_some(),
            "the composition is shown under the picture"
        );
        assert_eq!(
            view.update(cx, |v, cx| v.marked_text_range(w, cx)),
            Some(0..6)
        );
        assert_eq!(
            view.update(cx, |v, cx| v.text_for_range(0..2, &mut None, w, cx)),
            Some("ni".to_string())
        );
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "你好", None, w, cx)
        });
        assert_eq!(
            view.update(cx, |v, cx| v.marked_text_range(w, cx)),
            Some(0..2)
        );
        let bounds = view.update(cx, |v, cx| {
            v.bounds_for_range(0..2, gpui_kit::Bounds::default(), w, cx)
        });
        assert!(bounds.is_some(), "the candidate list needs an anchor");
    });
    // nothing is sent while composing
    cx.run_until_parked();
    assert!(
        input.try_recv().is_err(),
        "the remote desktop sees nothing before the text is committed"
    );
    win(cx, handle, |w, cx| {
        view.update(cx, |v, cx| v.replace_text_in_range(None, "你好", w, cx));
        w.render_frame(cx);
        assert!(
            w.try_find("rdp-composition").is_none(),
            "committing clears the composition"
        );
        assert_eq!(view.update(cx, |v, cx| v.marked_text_range(w, cx)), None);
    });
    let seen = drain_events(cx, &|s| s.len() >= 4);
    assert_eq!(
        seen,
        vec![
            RdpInputEvent::Unicode {
                character: "你".into(),
                down: true
            },
            RdpInputEvent::Unicode {
                character: "你".into(),
                down: false
            },
            RdpInputEvent::Unicode {
                character: "好".into(),
                down: true
            },
            RdpInputEvent::Unicode {
                character: "好".into(),
                down: false
            }
        ]
    );
    // cancelling a composition (Escape in the input method) leaves nothing behind
    win(cx, handle, |w, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "abc", None, w, cx)
        });
        view.update(cx, |v, cx| v.unmark_text(w, cx));
        w.render_frame(cx);
        assert!(w.try_find("rdp-composition").is_none());
    });
    cx.run_until_parked();
    assert!(input.try_recv().is_err());
    manager.disconnect(sid).unwrap();
}

// -- AI agent against a mock model ------------------------------------------------------------

struct ModelMock {
    url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
}

fn three_step_plan() -> String {
    let step = |n: &str, x: u16| {
        format!("{{\"description\":\"{n}\",\"intent\":\"click\",\"operations\":[{{\"type\":\"click\",\"x\":{x},\"y\":{x}}}],\"expected_outcome\":\"{n}生效\"}}")
    };
    format!("三步走。\n<RDP_PLAN>\n{{\"version\":1,\"summary\":\"三步\",\"steps\":[{},{},{}]}}\n</RDP_PLAN>", step("第一步", 1), step("第二步", 2), step("第三步", 3))
}

fn one_step_plan(description: &str, x: u16) -> String {
    format!("好的。\n<RDP_PLAN>\n{{\"version\":1,\"summary\":\"s\",\"steps\":[{{\"description\":\"{description}\",\"intent\":\"click\",\"operations\":[{{\"type\":\"click\",\"x\":{x},\"y\":{x}}}],\"expected_outcome\":\"{description}完成\"}}]}}\n</RDP_PLAN>")
}

fn rdp_reply(last_user: &str) -> String {
    if last_user.contains("请校验") {
        // the second step "went wrong", everything else is fine
        if last_user.contains("步骤：第二步") {
            "<RDP_VERIFY>{\"ok\":false,\"observation\":\"弹出了错误对话框\",\"reason\":\"点到了别的按钮\"}</RDP_VERIFY>".into()
        } else {
            "<RDP_VERIFY>{\"ok\":true,\"observation\":\"画面符合预期\",\"reason\":\"\"}</RDP_VERIFY>".into()
        }
    } else if last_user.contains("刚才已执行：创建文件并输入") {
        "已完成，文件已创建并写入内容。".into()
    } else if last_user.contains("刚才已执行：打开我的电脑") {
        one_step_plan("创建文件并输入", 20)
    } else if last_user.contains("两段任务") {
        one_step_plan("打开我的电脑", 10)
    } else if last_user.contains("没有达到预期") {
        "我来修正。\n<RDP_PLAN>\n{\"version\":1,\"summary\":\"修正\",\"steps\":[{\"description\":\"关闭对话框\",\"intent\":\"key\",\"operations\":[{\"type\":\"key\",\"keys\":[\"Escape\"]}],\"expected_outcome\":\"对话框关闭\"}]}\n</RDP_PLAN>".into()
    } else if last_user.contains("三步计划") {
        three_step_plan()
    } else if last_user.contains("刚才执行了第") {
        "目标已完成，记事本已经打开。".into()
    } else if last_user.contains("记事本") || last_user.contains("目标：") {
        "好的，我来操作。\n<RDP_PLAN>\n{\"version\":1,\"summary\":\"输入文字\",\"steps\":[{\"description\":\"点击并输入\",\"intent\":\"type\",\"operations\":[{\"type\":\"click\",\"x\":10,\"y\":20},{\"type\":\"type\",\"text\":\"hi\"}],\"expected_outcome\":\"出现文字\"}]}\n</RDP_PLAN>".into()
    } else if last_user.contains("围栏计划") {
        "好的，分两步：\n**第1步：** 按 Win+R\n**第2步：** 输入 explorer 并回车\n\n```json\n{\"version\":1,\"summary\":\"打开资源管理器\",\"steps\":[{\"description\":\"按 Win+R\",\"intent\":\"key\",\"operations\":[{\"type\":\"key\",\"keys\":[\"Meta\",\"r\"]}],\"expected_outcome\":\"运行框出现\"},{\"description\":\"输入 explorer\",\"intent\":\"type\",\"operations\":[{\"type\":\"type\",\"text\":\"explorer\"},{\"type\":\"key\",\"keys\":[\"Enter\"]}],\"expected_outcome\":\"资源管理器打开\"}]}\n```".into()
    } else if last_user.contains("坏计划") {
        "<RDP_PLAN>\n{\"version\":1,\"summary\":\"x\",\"steps\":[{\"description\":\"d\",\"operations\":[{\"type\":\"key\",\"keys\":[\"Banana\"]}]}]}\n</RDP_PLAN>".into()
    } else {
        "你好".into()
    }
}

fn start_model_mock() -> ModelMock {
    let listener = ops_core::rt::block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
    let store = bodies.clone();
    ops_core::rt::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let store = store.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 16384];
                let mut header_end = None;
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none() {
                        header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                    }
                    if let Some(he) = header_end {
                        let head = String::from_utf8_lossy(&buf[..he]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() >= he + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let body = text.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("");
                let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                store.lock().unwrap().push(json.clone());
                // With a screenshot the content is a list of parts (text + image_url).
                let last_user = json["messages"]
                    .as_array()
                    .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
                    .map(|m| match &m["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(parts) => parts
                            .iter()
                            .filter_map(|p| p["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                let reply = rdp_reply(&last_user);
                if json["stream"].as_bool() != Some(true) {
                    // one-shot requests (the step verification)
                    let body = serde_json::json!({"model": "mock-1", "choices": [{"message": {"content": reply}}], "usage": {}}).to_string();
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                    return;
                }
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await;
                let chars: Vec<char> = reply.chars().collect();
                for piece in chars.chunks(20) {
                    let delta: String = piece.iter().collect();
                    let line = format!(
                        "data: {}\n\n",
                        serde_json::json!({"model": "mock-1", "choices": [{"delta": {"content": delta}}]})
                    );
                    if sock.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                }
                let _ = sock.write_all(b"data: [DONE]\n\n").await;
            });
        }
    });
    ModelMock { url, bodies }
}

fn rdp_ai_fixture(
    cx: &mut TestAppContext,
    sid: &'static str,
) -> (
    Fixture,
    ModelMock,
    AnyWindowHandle,
    Entity<RdpWindow>,
    flume::Receiver<RdpInputEvent>,
) {
    let fx = common::setup(cx, |_| {});
    let model = start_model_mock();
    {
        let (db, url) = (fx.ctx.db.clone(), model.url.clone());
        ops_core::rt::block_on(async move {
            save_ai_config(
                &db,
                AiConfig {
                    provider: "ollama".into(),
                    api_url: url,
                    api_key: String::new(),
                    model: "mock-1".into(),
                    enabled: true,
                },
            )
            .await
            .unwrap();
        });
    }
    let input = opsbatch_app::rdp::global_manager().register_test_session(sid);
    let (handle, view) = open_view(cx, |window, cx| {
        cx.new(|cx| {
            RdpWindow::new(
                Some(HostRequest {
                    host_id: "none".into(),
                    name: "win-1".into(),
                    ip: "10.0.0.5".into(),
                }),
                String::new(),
                window,
                cx,
            )
        })
    });
    wait_until(cx, "initial attempt finished", |cx| {
        view.read_with(cx, |v, _| v.state() == RdpState::Error)
    });
    view.update(cx, |v, cx| v.debug_attach_test_session(sid.into(), cx));
    let full: Vec<u8> = (0..800 * 600).flat_map(|_| [200u8, 100, 50, 255]).collect();
    view.update(cx, |v, cx| {
        v.inject_frame(frame_message((800, 600), 0, 0, 800, 600, &full), cx)
    });
    // the assistant starts closed; the AI tests open it from the toolbar like a user
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        w.click("rdp-ai-toggle", cx);
    });
    view.read_with(cx, |v, _| assert!(v.ai_open()));
    win(cx, handle, |w, cx| w.render_frame(cx));
    (fx, model, handle, view, input)
}

fn rdp_chat_idle(cx: &mut TestAppContext, key: &str) -> bool {
    cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(key)
            .is_some_and(|s| !s.streaming())
    })
}

#[gpui_kit::test]
fn rdp_ai_proposes_a_plan_with_a_screenshot_and_plays_it_on_approval(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, model, handle, view, input) = rdp_ai_fixture(cx, "rdp-test-ai");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());
    assert_eq!(key, "rdp-none");

    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input()
                .update(cx, |i, cx| i.set_value("打开记事本".to_string(), w, cx))
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "reply parsed", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.pending_actions().len() == 1)
            })
    });

    // The model got the agent prompt (resolution) and a screenshot of the desktop.
    let sent = model.bodies.lock().unwrap().clone();
    let body = serde_json::to_string(&sent[0]).unwrap();
    assert!(
        body.contains("800x600") && body.contains("<RDP_PLAN>"),
        "agent prompt in the system message"
    );
    assert!(
        body.contains("data:image/jpeg;base64,"),
        "screenshot attached"
    );

    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert_eq!(
            s.messages[1].content, "好的，我来操作。",
            "plan removed from the chat text"
        );
        let a = s.pending_actions()[0].clone();
        assert_eq!(a.description, "点击并输入");
        assert_eq!(a.rdp_operations.as_ref().unwrap().len(), 2);
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-ai-action-0").is_some() && w.try_find("rdp-ai-badge").is_some());
        w.click("rdp-ai-approve-0", cx);
    });

    // Events arrive paced by timers; tick the test clock.
    let mut events = Vec::new();
    wait_ticking(cx, "plan played", |cx| {
        while let Ok(e) = input.try_recv() {
            events.push(e);
        }
        cx.update(|cx| {
            ai_store(cx)
                .read(cx)
                .session(&key)
                .is_some_and(|s| s.executed_actions().len() == 1)
        })
    });
    assert_eq!(events[0], RdpInputEvent::MouseMove { x: 10, y: 20 });
    assert_eq!(
        events[1],
        RdpInputEvent::MouseButton {
            x: 10,
            y: 20,
            button: 0,
            down: true
        }
    );
    assert_eq!(
        events[2],
        RdpInputEvent::MouseButton {
            x: 10,
            y: 20,
            button: 0,
            down: false
        }
    );
    assert_eq!(
        events[3],
        RdpInputEvent::Unicode {
            character: "h".into(),
            down: true
        }
    );
    assert_eq!(events.len(), 3 + 4);
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("rdp-ai-approve-0").is_none(),
            "executed steps lose their buttons"
        );
    });
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-ai")
        .unwrap();
}

#[gpui_kit::test]
fn rdp_ai_checks_each_step_against_the_screen_and_stops_the_queue_when_one_went_wrong(
    cx: &mut TestAppContext,
) {
    use opsbatch_app::rdp::ai_panel::Verdict;
    let _s = serial();
    let (_fx, model, handle, view, input) = rdp_ai_fixture(cx, "rdp-test-ai-verify");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());
    panel.read_with(cx, |p, _| {
        assert!(p.verify_steps(), "checking every step is on by default")
    });

    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input().update(cx, |i, cx| {
                i.set_value("给我一个三步计划".to_string(), w, cx)
            })
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "plan parsed", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.pending_actions().len() == 3)
            })
    });
    let ids: Vec<String> = cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(&key)
            .unwrap()
            .pending_actions()
            .iter()
            .map(|a| a.id.clone())
            .collect()
    });

    // "Run all": step 1 passes, step 2 is judged wrong, step 3 is never played.
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        w.click("rdp-ai-run-all", cx);
    });
    let mut events = Vec::new();
    wait_ticking(cx, "queue stopped", |cx| {
        while let Ok(e) = input.try_recv() {
            events.push(e);
        }
        panel.read_with(cx, |p, _| p.batch_stopped())
    });
    panel.read_with(cx, |p, _| {
        assert!(matches!(p.verdict(&ids[0]), Some(Verdict::Passed(o)) if o == "画面符合预期"), "{:?}", p.verdict(&ids[0]));
        assert!(matches!(p.verdict(&ids[1]), Some(Verdict::Failed { reason, .. }) if reason == "点到了别的按钮"), "{:?}", p.verdict(&ids[1]));
        assert!(p.verdict(&ids[2]).is_none());
    });
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert_eq!(
            s.executed_actions().len(),
            2,
            "the third step stays pending"
        );
    });
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, RdpInputEvent::MouseButton { down: true, .. }))
            .count(),
        2,
        "only two clicks were played"
    );
    // the checker saw the screen before and after, and what the step should achieve
    let verification = model
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| serde_json::to_string(b).ok())
        .find(|b| b.contains("请校验"))
        .expect("a verification request");
    assert_eq!(
        verification.matches("data:image/jpeg;base64,").count(),
        2,
        "before and after screenshots"
    );
    assert!(verification.contains("预期结果：第一步生效"));

    // The card says what is wrong and offers a fix, which goes to the model with the current screen.
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-ai-batch-stopped").is_some());
        assert!(w.try_find("rdp-ai-verdict-1").is_some() && w.try_find("rdp-ai-fix-1").is_some());
    });
    // (the button sits in a scrolled list that is too short in this test window, so it is invoked directly)
    panel.update(cx, |p, cx| p.fix_step(&ids[1], cx));
    wait_until(cx, "fix plan", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx).read(cx).session(&key).is_some_and(|s| {
                    s.pending_actions()
                        .iter()
                        .any(|a| a.description == "关闭对话框")
                })
            })
    });
    let last = model
        .bodies
        .lock()
        .unwrap()
        .last()
        .map(|b| serde_json::to_string(b).unwrap())
        .unwrap();
    assert!(
        last.contains("没有达到预期")
            && last.contains("点到了别的按钮")
            && last.contains("data:image/jpeg;base64,")
    );

    // Switching the check off plays steps without asking the model about them.
    panel.update(cx, |p, cx| p.set_verify_steps(false, cx));
    panel.read_with(cx, |p, _| assert!(!p.verify_steps()));
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-ai-verify")
        .unwrap();
}

#[gpui_kit::test]
fn rdp_ai_asks_for_the_next_step_after_each_executed_step_until_the_model_says_it_is_done(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (_fx, model, handle, view, input) = rdp_ai_fixture(cx, "rdp-test-ai-continue");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());
    panel.read_with(cx, |p, _| {
        assert!(p.auto_continue(), "continuing by itself is on by default")
    });

    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input().update(cx, |i, cx| {
                i.set_value("两段任务：打开我的电脑，然后创建文件".to_string(), w, cx)
            })
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "first plan", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.pending_actions().len() == 1)
            })
    });
    let first = cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(&key)
            .unwrap()
            .pending_actions()[0]
            .id
            .clone()
    });

    // Approving the first step plays it, checks it, and then the model is asked for the next one with the new screen.
    panel.update(cx, |p, cx| p.approve(&first, cx));
    let mut events = Vec::new();
    wait_ticking(cx, "second plan arrives by itself", |cx| {
        while let Ok(e) = input.try_recv() {
            events.push(e);
        }
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx).read(cx).session(&key).is_some_and(|s| {
                    s.pending_actions()
                        .iter()
                        .any(|a| a.description == "创建文件并输入")
                })
            })
    });
    let follow_up = model
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter_map(|b| serde_json::to_string(b).ok())
        .find(|b| b.contains("刚才已执行：打开我的电脑"))
        .expect("a follow-up request");
    assert!(
        follow_up.contains("用户请求：两段任务") && follow_up.contains("data:image/jpeg;base64,"),
        "the request and the new screen go with it"
    );
    assert!(
        follow_up.contains("画面校验：画面符合预期"),
        "and what the check saw"
    );
    panel.read_with(cx, |p, _| assert_eq!(p.follow_up_rounds(), 1));

    // The second step is approved the same way; the model answers "done" and nothing more is asked.
    let second = cx.update(|cx| {
        ai_store(cx)
            .read(cx)
            .session(&key)
            .unwrap()
            .pending_actions()[0]
            .id
            .clone()
    });
    panel.update(cx, |p, cx| p.approve(&second, cx));
    wait_ticking(cx, "done", |cx| {
        while let Ok(e) = input.try_recv() {
            events.push(e);
        }
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx).read(cx).session(&key).is_some_and(|s| {
                    s.messages
                        .last()
                        .is_some_and(|m| m.content.starts_with("已完成"))
                })
            })
    });
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert!(s.pending_actions().is_empty(), "no further plan");
        assert_eq!(s.executed_actions().len(), 2);
    });
    panel.read_with(cx, |p, _| assert_eq!(p.follow_up_rounds(), 2));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, RdpInputEvent::MouseButton { down: true, .. }))
            .count(),
        2
    );
    // Switching it off leaves the next step to the user.
    panel.update(cx, |p, cx| p.set_auto_continue(false, cx));
    panel.read_with(cx, |p, _| assert!(!p.auto_continue()));
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-ai-continue")
        .unwrap();
    let _ = (handle, view);
}

#[gpui_kit::test]
fn rdp_ai_parses_a_plan_the_model_put_in_a_json_code_block(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, _model, handle, view, _input) = rdp_ai_fixture(cx, "rdp-test-ai-fence");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());
    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input().update(cx, |i, cx| {
                i.set_value("给我一个围栏计划".to_string(), w, cx)
            })
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "reply", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.messages.len() == 2)
            })
    });
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert_eq!(s.messages[1].rdp_parse_error, None);
        assert_eq!(
            s.pending_actions().len(),
            2,
            "both steps of the ```json plan become pending actions"
        );
    });
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-ai-fence")
        .unwrap();
    let _ = handle;
}

#[gpui_kit::test]
fn rdp_ai_reports_unparsable_plans_and_rejected_steps_are_not_played(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, _model, handle, view, input) = rdp_ai_fixture(cx, "rdp-test-ai2");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());

    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input()
                .update(cx, |i, cx| i.set_value("给我一个坏计划".to_string(), w, cx))
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "reply", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.messages.len() == 2)
            })
    });
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert!(s.messages[1].pending_actions.is_empty());
        assert!(s.messages[1]
            .rdp_parse_error
            .as_deref()
            .unwrap_or("")
            .contains("不支持的按键: Banana"));
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("rdp-ai-parse-error-1").is_some());
    });

    // A good plan, rejected: nothing reaches the remote.
    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.input()
                .update(cx, |i, cx| i.set_value("打开记事本".to_string(), w, cx))
        });
        w.render_frame(cx);
        w.click("rdp-ai-send", cx);
    });
    wait_until(cx, "plan", |cx| {
        rdp_chat_idle(cx, &key)
            && cx.update(|cx| {
                ai_store(cx)
                    .read(cx)
                    .session(&key)
                    .is_some_and(|s| s.pending_actions().len() == 1)
            })
    });
    win(cx, handle, |w, cx| {
        w.render_frame(cx);
        w.click("rdp-ai-reject-0", cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(ai_store(cx)
            .read(cx)
            .session(&key)
            .unwrap()
            .pending_actions()
            .is_empty())
    });
    assert!(input.try_recv().is_err());
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-ai2")
        .unwrap();
}

#[gpui_kit::test]
fn rdp_ai_goal_mode_loops_until_the_model_says_it_is_done(cx: &mut TestAppContext) {
    let _s = serial();
    let (_fx, model, handle, view, input) = rdp_ai_fixture(cx, "rdp-test-goal");
    let panel = view.read_with(cx, |v, _| v.ai_panel().clone());
    let key = panel.read_with(cx, |p, _| p.session_key().to_string());

    win(cx, handle, |w, cx| {
        panel.update(cx, |p, cx| {
            p.set_goal_mode(true, cx);
            p.goal_input()
                .update(cx, |i, cx| i.set_value("打开记事本".to_string(), w, cx));
        });
        w.render_frame(cx);
        w.click("rdp-ai-goal-start", cx);
    });
    assert!(panel.read_with(cx, |p, _| p.goal_running()));
    // plan → auto-executed → settle → screenshot check → "目标已完成" → goal stops
    let mut events = Vec::new();
    wait_ticking(cx, "goal finished", |cx| {
        while let Ok(e) = input.try_recv() {
            events.push(e);
        }
        panel.read_with(cx, |p, _| !p.goal_running()) && rdp_chat_idle(cx, &key)
    });
    assert_eq!(
        events.len(),
        7,
        "the plan ran exactly once without any click on approve"
    );
    let bodies = model.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "start + one check round");
    let check = serde_json::to_string(&bodies[1]).unwrap();
    assert!(check.contains("刚才执行了第 1 轮操作") && check.contains("data:image/jpeg;base64,"));
    cx.update(|cx| {
        let s = ai_store(cx).read(cx).session(&key).unwrap().clone();
        assert!(s.messages.last().unwrap().content.contains("目标已完成"));
    });
    assert_eq!(panel.read_with(cx, |p, _| p.goal_round()), 1);
    opsbatch_app::rdp::global_manager()
        .disconnect("rdp-test-goal")
        .unwrap();
}

// ---------------------------------------------------------------------------
// Opening from the sidebar
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn windows_and_vnc_hosts_open_their_own_windows(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let win_id = add_host(&fx, new_remote_host("win-box", "windows", None, Some("x")));
    let vnc_id = add_host(
        &fx,
        new_remote_host(
            "vnc-box",
            "vnc",
            Some(vnc_settings(free_port(), false)),
            None,
        ),
    );
    common::open_sidebar(cx, &fx, 2);
    cx.update_window(fx.handle.into(), |_, window, cx| {
        fx.workbench.update(cx, |w, cx| {
            w.open_host(&win_id, window, cx);
            w.open_host(&vnc_id, window, cx);
        });
    })
    .unwrap();
    cx.update(|cx| {
        assert!(opsbatch_app::window::is_open(&format!("rdp-{win_id}"), cx));
        assert!(opsbatch_app::window::is_open(&format!("vnc-{vnc_id}"), cx));
        // Opening again focuses instead of duplicating.
        let before = opsbatch_app::window::open_labels(cx).len();
        opsbatch_app::rdp::window::RdpWindow::open(
            HostRequest {
                host_id: win_id.clone(),
                name: "win-box".into(),
                ip: "127.0.0.1".into(),
            },
            cx,
        );
        assert_eq!(opsbatch_app::window::open_labels(cx).len(), before);
    });
}

#[gpui_kit::test]
fn closing_a_failed_remote_window_asks_for_the_asset_manager(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let (handle, _view) = open_view(cx, |window, cx| {
        cx.new(|cx| RdpWindow::new(None, String::new(), window, cx))
    });
    let workbench = fx.workbench.clone();
    cx.update(|cx| assert!(!workbench.read(cx).asset_panel_visible()));
    win(cx, handle, |w, cx| w.click("rdp-open-assets", cx));
    wait_until(cx, "sidebar requested", |cx| {
        workbench.read_with(cx, |w, _| w.asset_panel_visible())
    });
}
