//! M0-06 spike: native RFB client (replaces noVNC + WebSocket bridge).
//!
//! Real-server run (no GUI):
//!   VNC_USER=me VNC_PASS=secret VNC_AUTH=ard cargo run -p spike-vnc -- 192.168.1.20 5900
//! writes the first full frame to target/spike-data/vnc.ppm.

mod auth;
mod client;

use client::{AuthPreference, Options, RfbClient, ServerEvent};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(host), port) = (args.next(), args.next().unwrap_or_else(|| "5900".into())) else {
        eprintln!("usage: spike-vnc <host> [port]   (env: VNC_USER, VNC_PASS, VNC_AUTH=ard|vnc)");
        return Ok(());
    };
    let opts = Options {
        username: std::env::var("VNC_USER").ok(),
        password: std::env::var("VNC_PASS").ok(),
        prefer: if std::env::var("VNC_AUTH").as_deref() == Ok("ard") { AuthPreference::Ard } else { AuthPreference::Vnc },
        shared: true,
    };
    let stream = TcpStream::connect(format!("{host}:{port}")).await?;
    let mut c = RfbClient::connect(stream, &opts).await?;
    println!("connected: {:?} {}x{} rfb {:?}", c.name, c.fb.width, c.fb.height, c.version);
    c.request_update(false).await?;
    let t0 = std::time::Instant::now();
    let (mut rects, mut updates) = (0usize, 0usize);
    while t0.elapsed().as_secs() < 5 {
        match c.next_event().await? {
            ServerEvent::Update(r) => {
                updates += 1;
                rects += r.len();
                if updates == 1 {
                    let (w, h) = (c.fb.width as usize, c.fb.height as usize);
                    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                    for px in c.fb.data.chunks_exact(4) {
                        ppm.extend_from_slice(&[px[2], px[1], px[0]]);
                    }
                    std::fs::create_dir_all("target/spike-data")?;
                    tokio::fs::File::create("target/spike-data/vnc.ppm").await?.write_all(&ppm).await?;
                }
                c.request_update(true).await?;
            }
            ev => println!("event {ev:?}"),
        }
    }
    println!("updates={updates} rects={rects} in {:?}", t0.elapsed());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{ard_server_decrypt, vnc_auth_response};
    use client::{choose_security, SEC_ARD, SEC_NONE, SEC_VNC};
    use flate2::Compression;
    use num_bigint::BigUint;
    use tokio::io::{AsyncReadExt, DuplexStream};

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

    fn px(c: &RfbClient<DuplexStream>, x: usize, y: usize) -> [u8; 4] {
        let o = (y * c.fb.width as usize + x) * 4;
        c.fb.data[o..o + 4].try_into().unwrap()
    }

    async fn run(version: &'static str, sec: u8, prefer: AuthPreference, cpass: &'static str) -> anyhow::Result<RfbClient<DuplexStream>> {
        let (cs, ss) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let _ = mock_server(ss, version, sec, "alice", "pw12").await;
        });
        let mut c = RfbClient::connect(cs, &opts("alice", cpass, prefer)).await?;
        c.request_update(false).await?;
        loop {
            if let ServerEvent::Update(r) = c.next_event().await? {
                assert_eq!(r.len(), 4);
                return Ok(c);
            }
        }
    }

    fn check_pixels(c: &RfbClient<DuplexStream>) {
        assert_eq!(c.name, "mock");
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
}
