//! A local release mirror with the GitHub layout (`/releases/latest`, `/releases.atom`, `/releases/download/<tag>/…`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};

pub const TAG: &str = "v9.9.9";

pub struct Mirror {
    pub base: String,
    pub corrupt: Arc<AtomicBool>,
}

pub fn payload() -> Vec<u8> {
    (0..400_000u32).map(|i| (i % 253) as u8).collect()
}

pub fn start(asset: String) -> Mirror {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let corrupt = Arc::new(AtomicBool::new(false));
    let (c, b) = (corrupt.clone(), base.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let (c, b, asset) = (c.clone(), b.clone(), asset.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                let len = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..len]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let reply = |status: &str,
                             headers: &str,
                             body: &[u8],
                             stream: &mut std::net::TcpStream| {
                    let _ = stream.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                    for chunk in body.chunks(32 * 1024) {
                        let _ = stream.write_all(chunk);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                };
                let good = payload();
                match path.as_str() {
                    "/releases/latest" => reply(
                        "302 Found",
                        &format!("Location: {b}/releases/tag/{TAG}\r\n"),
                        b"",
                        &mut stream,
                    ),
                    p if p == format!("/releases/tag/{TAG}") => {
                        reply("200 OK", "", b"release", &mut stream)
                    }
                    "/releases.atom" => {
                        let feed = format!(
                            "<feed><entry><title>OpsBatch 9.9.9</title><link href=\"https://example.test/releases/tag/{TAG}\" /><updated>2026-10-01T00:00:00Z</updated><content type=\"html\">&lt;li&gt;mirror notes&lt;/li&gt;</content></entry></feed>"
                        );
                        reply("200 OK", "", feed.as_bytes(), &mut stream)
                    }
                    p if p == format!("/releases/download/{TAG}/SHA256SUMS") => {
                        let hash: String = Sha256::digest(&good)
                            .iter()
                            .map(|x| format!("{x:02x}"))
                            .collect();
                        reply(
                            "200 OK",
                            "",
                            format!("{hash}  {asset}\n").as_bytes(),
                            &mut stream,
                        )
                    }
                    p if p == format!("/releases/download/{TAG}/{asset}") => {
                        let mut body = good;
                        if c.load(Ordering::SeqCst) {
                            body[5000] ^= 0xff;
                        }
                        reply("200 OK", "", &body, &mut stream)
                    }
                    _ => reply("404 Not Found", "", b"", &mut stream),
                }
            });
        }
    });
    Mirror { base, corrupt }
}
