use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use crate::auth::{ard_server_decrypt, vnc_auth_response};
use crate::client::*;
use flate2::Compression;
use num_bigint::BigUint;


const W: u16 = 64;
const H: u16 = 32;

async fn read_exact_vec(s: &mut DuplexStream, n: usize) -> Vec<u8> {
    let mut b = vec![0; n];
    s.read_exact(&mut b).await.unwrap();
    b
}

/// Plays the server role. `sec` selects the security type; returns Err(()) if the client failed auth.
async fn mock_server(mut s: DuplexStream, version: &str, sec: u8, user: &str, pass: &str) -> Result<(), ()> {
    s.write_all(version.as_bytes()).await.unwrap();
    let cver = read_exact_vec(&mut s, 12).await;
    let v38 = cver == b"RFB 003.008\n";
    let v33 = cver == b"RFB 003.003\n";
    if v33 {
        s.write_u32(sec as u32).await.unwrap();
    } else {
        s.write_all(&[1, sec]).await.unwrap();
        assert_eq!(read_exact_vec(&mut s, 1).await[0], sec);
    }
    let ok = match sec {
        SEC_NONE => true,
        SEC_VNC => {
            let ch = [9u8; 16];
            s.write_all(&ch).await.unwrap();
            read_exact_vec(&mut s, 16).await == vnc_auth_response(pass, &ch)
        }
        SEC_ARD => {
            let prime = hex_prime();
            let g = 2u16;
            let sp = BigUint::from_bytes_be(&[0x37; 128]);
            let pb = BigUint::from_bytes_be(&prime);
            let spub = {
                let b = BigUint::from(g).modpow(&sp, &pb).to_bytes_be();
                let mut o = vec![0; 128 - b.len()];
                o.extend(b);
                o
            };
            s.write_u16(g).await.unwrap();
            s.write_u16(128).await.unwrap();
            s.write_all(&prime).await.unwrap();
            s.write_all(&spub).await.unwrap();
            let enc = read_exact_vec(&mut s, 128).await;
            let cpub = read_exact_vec(&mut s, 128).await;
            ard_server_decrypt(&prime, &sp, &cpub, &enc) == (user.to_string(), pass.to_string())
        }
        _ => unreachable!(),
    };
    if sec != SEC_NONE || v38 {
        s.write_u32(if ok { 0 } else { 1 }).await.unwrap();
        if !ok && v38 {
            s.write_u32(4).await.unwrap();
            s.write_all(b"nope").await.unwrap();
        }
    }
    if !ok {
        return Err(());
    }
    assert_eq!(read_exact_vec(&mut s, 1).await.len(), 1); // ClientInit
    s.write_u16(W).await.unwrap();
    s.write_u16(H).await.unwrap();
    s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).await.unwrap();
    s.write_u32(4).await.unwrap();
    s.write_all(b"mock").await.unwrap();

    assert_eq!(read_exact_vec(&mut s, 20).await[0], 0); // SetPixelFormat
    let hdr = read_exact_vec(&mut s, 4).await; // SetEncodings
    assert_eq!(hdr[0], 2);
    let n = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    read_exact_vec(&mut s, n * 4).await;
    assert_eq!(read_exact_vec(&mut s, 10).await[0], 3); // FramebufferUpdateRequest

    // FramebufferUpdate: Raw 4x2 @ (0,0); CopyRect 4x2 @ (8,0) from (0,0); ZRLE solid 10x4 @ (0,8); ZRLE 70x2 with RLE.
    let mut u = vec![0u8, 0];
    u.extend_from_slice(&4u16.to_be_bytes());
    let rect = |x: u16, y: u16, w: u16, h: u16, enc: i32| {
        let mut r = Vec::new();
        for v in [x, y, w, h] {
            r.extend_from_slice(&v.to_be_bytes());
        }
        r.extend_from_slice(&enc.to_be_bytes());
        r
    };
    u.extend(rect(0, 0, 4, 2, 0));
    for i in 0..8u8 {
        u.extend_from_slice(&[i * 10, i * 10 + 1, i * 10 + 2, 0]); // B G R X
    }
    u.extend(rect(8, 0, 4, 2, 1));
    u.extend_from_slice(&0u16.to_be_bytes());
    u.extend_from_slice(&0u16.to_be_bytes());
    u.extend(rect(0, 8, 10, 4, 16));
    // Both ZRLE rects share ONE continuing zlib stream, each ending in a sync flush (as real servers do).
    // Tile bytes: solid = [1, B, G, R]; plain RLE = [0x80, B, G, R, run-1 bytes...].
    let mut stream = flate2::Compress::new(Compression::fast(), true);
    let mut chunk = |input: &[u8]| {
        let mut out = Vec::with_capacity(256);
        stream.compress_vec(input, &mut out, flate2::FlushCompress::Sync).unwrap();
        out
    };
    let c1 = chunk(&[1, 10, 20, 30]);
    // 64x2 = 128 px: run = 1 + 127 = 128 of colour (B,G,R)=(1,2,3)
    let c2 = chunk(&[0x80, 1, 2, 3, 127]);
    u.extend_from_slice(&(c1.len() as u32).to_be_bytes());
    u.extend(c1);
    u.extend(rect(0, 16, 64, 2, 16));
    u.extend_from_slice(&(c2.len() as u32).to_be_bytes());
    u.extend(c2);
    s.write_all(&u).await.unwrap();
    // keep the stream open until the client is done
    let _ = s.read_u8().await;
    Ok(())
}

fn hex_prime() -> Vec<u8> {
    let s = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece65381ffffffffffffffff";
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn opts(user: &str, pass: &str, prefer: AuthPreference) -> Options {
    Options { username: Some(user.into()), password: Some(pass.into()), prefer, shared: true }
}

type Reader = RfbReader<tokio::io::ReadHalf<DuplexStream>>;

fn px(c: &Reader, x: usize, y: usize) -> [u8; 4] {
    let o = (y * c.fb.width as usize + x) * 4;
    c.fb.data[o..o + 4].try_into().unwrap()
}

async fn run(version: &'static str, sec: u8, prefer: AuthPreference, cpass: &'static str) -> anyhow::Result<(RfbInfo, Reader)> {
    let (cs, ss) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let _ = mock_server(ss, version, sec, "alice", "pw12").await;
    });
    let (info, mut r, mut w) = connect(cs, &opts("alice", cpass, prefer)).await?;
    w.request_update(false).await?;
    loop {
        if let ServerEvent::Update(rects) = r.next_event().await? {
            assert_eq!(rects.len(), 4);
            return Ok((info, r));
        }
    }
}

fn check_pixels((info, c): &(RfbInfo, Reader)) {
    assert_eq!(info.name, "mock");
    assert_eq!((c.fb.width, c.fb.height), (W, H));
    assert_eq!(px(c, 0, 0), [0, 1, 2, 255]); // raw
    assert_eq!(px(c, 3, 1), [70, 71, 72, 255]);
    assert_eq!(px(c, 8, 0), [0, 1, 2, 255]); // copyrect
    assert_eq!(px(c, 11, 1), [70, 71, 72, 255]);
    assert_eq!(px(c, 9, 11), [10, 20, 30, 255]); // zrle solid 10x4 @ (0,8)
    assert_eq!(px(c, 10, 11), [0, 0, 0, 0]); // outside rect untouched
    assert_eq!(px(c, 0, 16), [1, 2, 3, 255]); // zrle RLE tile 1
    assert_eq!(px(c, 63, 17), [1, 2, 3, 255]);
}

#[tokio::test]
async fn security_none_rfb38() {
    check_pixels(&run("RFB 003.008\n", SEC_NONE, AuthPreference::Vnc, "").await.unwrap());
}

#[tokio::test]
async fn security_vnc_auth_ok_and_wrong_password() {
    check_pixels(&run("RFB 003.008\n", SEC_VNC, AuthPreference::Vnc, "pw12").await.unwrap());
    let err = run("RFB 003.008\n", SEC_VNC, AuthPreference::Vnc, "bad").await.err().unwrap();
    assert!(err.to_string().contains("authentication failed"), "{err}");
}

#[tokio::test]
async fn security_ard_and_apple_version_string() {
    // Apple servers announce RFB 003.889 and are spoken to as 3.8.
    check_pixels(&run("RFB 003.889\n", SEC_ARD, AuthPreference::Ard, "pw12").await.unwrap());
    assert!(run("RFB 003.889\n", SEC_ARD, AuthPreference::Ard, "bad").await.is_err());
}

#[tokio::test]
async fn rfb33_type_chosen_by_server() {
    check_pixels(&run("RFB 003.003\n", SEC_VNC, AuthPreference::Vnc, "pw12").await.unwrap());
}

#[test]
fn security_preference_order() {
    assert_eq!(choose_security(&[1, 2, 30], AuthPreference::Vnc, true), Some(SEC_VNC));
    assert_eq!(choose_security(&[1, 2, 30], AuthPreference::Ard, true), Some(SEC_ARD));
    assert_eq!(choose_security(&[1, 2], AuthPreference::Vnc, false), Some(SEC_NONE));
    assert_eq!(choose_security(&[30], AuthPreference::Vnc, true), Some(SEC_ARD));
    assert_eq!(choose_security(&[5, 6], AuthPreference::Vnc, true), None);
}


// ---------------------------------------------------------------------------
// Hextile, desktop resize, and a whole session over TCP
// ---------------------------------------------------------------------------

/// RFB 3.8, security None, 16x16 desktop; then plays `updates` after the first update request.
async fn simple_server(mut s: DuplexStream, updates: Vec<u8>) {
    s.write_all(b"RFB 003.008\n").await.unwrap();
    read_exact_vec(&mut s, 12).await;
    s.write_all(&[1, SEC_NONE]).await.unwrap();
    read_exact_vec(&mut s, 1).await;
    s.write_u32(0).await.unwrap();
    read_exact_vec(&mut s, 1).await;
    s.write_u16(32).await.unwrap();
    s.write_u16(32).await.unwrap();
    s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).await.unwrap();
    s.write_u32(1).await.unwrap();
    s.write_all(b"s").await.unwrap();
    read_exact_vec(&mut s, 20).await;
    let hdr = read_exact_vec(&mut s, 4).await;
    let n = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    let encodings = read_exact_vec(&mut s, n * 4).await;
    // ZRLE (16), Hextile (5), CopyRect (1), Raw (0), DesktopSize (-223) in that order.
    let expected: Vec<u8> = [16i32, 5, 1, 0, -223].iter().flat_map(|e| e.to_be_bytes()).collect();
    assert_eq!(encodings, expected);
    read_exact_vec(&mut s, 10).await;
    s.write_all(&updates).await.unwrap();
    let _ = s.read_u8().await;
}

fn rect_header(x: u16, y: u16, w: u16, h: u16, enc: i32) -> Vec<u8> {
    let mut r = Vec::new();
    for v in [x, y, w, h] {
        r.extend_from_slice(&v.to_be_bytes());
    }
    r.extend_from_slice(&enc.to_be_bytes());
    r
}

#[tokio::test]
async fn hextile_background_foreground_and_subrects() {
    // One 20x4 hextile rect at (2,2) => tiles 16x4 and 4x4.
    let mut u = vec![0u8, 0, 0, 1];
    u.extend(rect_header(2, 2, 20, 4, 5));
    // Tile 1: bg + fg specified, any-subrects (not coloured): bg=(1,2,3) fg=(9,8,7), one subrect at (1,1) size 2x2.
    u.push(2 | 4 | 8);
    u.extend_from_slice(&[1, 2, 3, 0]);
    u.extend_from_slice(&[9, 8, 7, 0]);
    u.push(1);
    u.push(0x11);
    u.push(0x11);
    // Tile 2: inherits the background, coloured subrect (0,0) 1x1 in (50,60,70).
    u.push(8 | 16);
    u.push(1);
    u.extend_from_slice(&[50, 60, 70, 0]);
    u.push(0x00);
    u.push(0x00);
    let (cs, ss) = tokio::io::duplex(1 << 16);
    tokio::spawn(simple_server(ss, u));
    let (_info, mut r, mut w) = connect(cs, &opts("", "", AuthPreference::Vnc)).await.unwrap();
    w.request_update(false).await.unwrap();
    assert!(matches!(r.next_event().await.unwrap(), ServerEvent::Update(v) if v.len() == 1));
    assert_eq!(px(&r, 2, 2), [1, 2, 3, 255], "background");
    assert_eq!(px(&r, 3, 3), [9, 8, 7, 255], "subrect in the foreground colour");
    assert_eq!(px(&r, 17, 2), [1, 2, 3, 255]);
    assert_eq!(px(&r, 18, 2), [50, 60, 70, 255], "coloured subrect");
    assert_eq!(px(&r, 19, 2), [1, 2, 3, 255], "inherited background");
    assert_eq!(px(&r, 0, 0), [0, 0, 0, 0], "outside the rect");
}

#[tokio::test]
async fn desktop_size_replaces_the_framebuffer() {
    let mut u = vec![0u8, 0, 0, 1];
    u.extend(rect_header(0, 0, 48, 40, -223));
    let (cs, ss) = tokio::io::duplex(1 << 16);
    tokio::spawn(simple_server(ss, u));
    let (_info, mut r, mut w) = connect(cs, &opts("", "", AuthPreference::Vnc)).await.unwrap();
    w.request_update(false).await.unwrap();
    assert!(matches!(r.next_event().await.unwrap(), ServerEvent::Resized { width: 48, height: 40 }));
    assert_eq!((r.fb.width, r.fb.height), (48, 40));
}

#[tokio::test]
async fn rects_outside_the_framebuffer_are_rejected() {
    let mut u = vec![0u8, 0, 0, 1];
    u.extend(rect_header(30, 30, 10, 10, 0));
    u.extend(vec![0u8; 400]);
    let (cs, ss) = tokio::io::duplex(1 << 16);
    tokio::spawn(simple_server(ss, u));
    let (_info, mut r, mut w) = connect(cs, &opts("", "", AuthPreference::Vnc)).await.unwrap();
    w.request_update(false).await.unwrap();
    assert!(r.next_event().await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_streams_frames_forwards_input_and_honours_view_only() {
    use crate::config::{VncAuthMethod, VncHostConfig};
    use crate::session::{start_session, VncCommand, VncEvent};
    use std::time::Duration;
    use tokio::net::TcpListener;

    for view_only in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(b"RFB 003.008\n").await.unwrap();
            let mut buf = [0u8; 12];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(&[1, SEC_NONE]).await.unwrap();
            let mut one = [0u8; 1];
            s.read_exact(&mut one).await.unwrap();
            s.write_u32(0).await.unwrap();
            s.read_exact(&mut one).await.unwrap();
            s.write_u16(4).await.unwrap();
            s.write_u16(2).await.unwrap();
            s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).await.unwrap();
            s.write_u32(4).await.unwrap();
            s.write_all(b"mock").await.unwrap();
            // SetPixelFormat(20) + SetEncodings(4 + 5*4) + first FramebufferUpdateRequest(10)
            let mut setup = [0u8; 20 + 4 + 20 + 10];
            s.read_exact(&mut setup).await.unwrap();
            // Whole-screen raw update, then every later client message is reported.
            let mut u = vec![0u8, 0, 0, 1];
            u.extend(rect_header(0, 0, 4, 2, 0));
            for i in 0..8u8 {
                u.extend_from_slice(&[i, i, i, 0]);
            }
            s.write_all(&u).await.unwrap();
            let mut msg = [0u8; 64];
            loop {
                let n = s.read(&mut msg).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                let _ = seen_tx.send(msg[..n].to_vec());
            }
        });

        let session = start_session(VncHostConfig {
            host: "127.0.0.1".into(),
            port,
            username: None,
            password: None,
            auth_method: VncAuthMethod::VncAuth,
            shared: true,
            view_only,
            proxy: None,
        });
        let next = |rx: &flume::Receiver<VncEvent>| rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let events = session.events.clone();
        let ev = tokio::task::spawn_blocking({
            let events = events.clone();
            move || (events.recv_timeout(Duration::from_secs(5)).unwrap(), events.recv_timeout(Duration::from_secs(5)).unwrap())
        })
        .await
        .unwrap();
        assert!(matches!(&ev.0, VncEvent::Connected { width: 4, height: 2, name } if name == "mock"), "{:?}", ev.0);
        let VncEvent::Frame(regions) = &ev.1 else { panic!("{:?}", ev.1) };
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].1.len(), 4 * 2 * 4);
        assert_eq!(&regions[0].1[28..32], &[7, 7, 7, 255]);
        let _ = next;

        // The follow-up incremental update request comes first, then input.
        assert_eq!(seen_rx.recv().await.unwrap()[0], 3);
        session.send(VncCommand::Pointer { x: 3, y: 1, buttons: 1 });
        session.send(VncCommand::Key { keysym: 0x61, down: true });
        session.send(VncCommand::Refresh);
        let mut got = Vec::new();
        while let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(500), seen_rx.recv()).await {
            got.extend(m);
        }
        let has = |needle: &[u8]| got.windows(needle.len()).any(|w| w == needle);
        assert!(has(&[3, 0, 0, 0, 0, 0, 0, 4, 0, 2]), "refresh is a non-incremental whole-screen request: {got:?}");
        assert_eq!(has(&[5, 1, 0, 3, 0, 1]), !view_only, "pointer event iff not view-only: {got:?}");
        assert_eq!(has(&[4, 1, 0, 0, 0, 0, 0, 0x61]), !view_only, "key event iff not view-only");
        session.disconnect();
        let closed = tokio::task::spawn_blocking(move || loop {
            match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                VncEvent::Closed { clean, .. } => break clean,
                _ => continue,
            }
        })
        .await
        .unwrap();
        assert!(closed, "a disconnect we asked for is clean");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_failures_are_reported_as_closed_events() {
    use crate::config::{VncAuthMethod, VncHostConfig};
    use crate::session::{start_session, VncEvent};
    use std::time::Duration;
    // Nothing listens on this port.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let session = start_session(VncHostConfig {
        host: "127.0.0.1".into(),
        port: free,
        username: None,
        password: None,
        auth_method: VncAuthMethod::VncAuth,
        shared: true,
        view_only: false,
        proxy: None,
    });
    let events = session.events.clone();
    let ev = tokio::task::spawn_blocking(move || events.recv_timeout(Duration::from_secs(10)).unwrap()).await.unwrap();
    assert!(matches!(&ev, VncEvent::Closed { clean: false, reason: Some(r) } if r.contains("VNC TCP 连接失败")), "{ev:?}");
}
