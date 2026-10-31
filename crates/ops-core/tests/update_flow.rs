//! Update check + verified download against a local mirror with the GitHub release layout.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ops_core::update::{
    check_app_update_from, download_update, installer_asset_name, UpdateSource,
};
use sha2::{Digest, Sha256};

const TAG: &str = "v9.9.9";

struct Mirror {
    base: String,
    corrupt: Arc<AtomicBool>,
    no_sums: Arc<AtomicBool>,
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn payload() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

fn start(os: &'static str, arch: &'static str) -> Mirror {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (corrupt, no_sums) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (c, n, b) = (corrupt.clone(), no_sums.clone(), base.clone());
    let asset = installer_asset_name(TAG, os, arch).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let (c, n, b, asset) = (c.clone(), n.clone(), b.clone(), asset.clone());
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
                            "<feed><entry><title>{TAG}</title><link href=\"https://example.test/releases/tag/{TAG}\" /><updated>2026-10-01T00:00:00Z</updated><content type=\"html\">&lt;li&gt;mirror notes&lt;/li&gt;</content></entry></feed>"
                        );
                        reply("200 OK", "", feed.as_bytes(), &mut stream)
                    }
                    p if p == format!("/releases/download/{TAG}/SHA256SUMS") => {
                        if n.load(Ordering::SeqCst) {
                            reply(
                                "200 OK",
                                "",
                                format!("{}  unrelated.bin\n", "0".repeat(64)).as_bytes(),
                                &mut stream,
                            )
                        } else {
                            reply(
                                "200 OK",
                                "",
                                format!("{}  {asset}\n", sha(&good)).as_bytes(),
                                &mut stream,
                            )
                        }
                    }
                    p if p == format!("/releases/download/{TAG}/{asset}") => {
                        let mut body = good;
                        if c.load(Ordering::SeqCst) {
                            body[1000] ^= 0xff;
                        }
                        reply("200 OK", "", &body, &mut stream)
                    }
                    _ => reply("404 Not Found", "", b"", &mut stream),
                }
            });
        }
    });
    Mirror {
        base,
        corrupt,
        no_sums,
    }
}

#[test]
fn newer_release_is_reported_with_notes_and_older_is_not() {
    let m = start("macos", "aarch64");
    let source = UpdateSource::with_base(&m.base);
    let info = ops_core::rt::block_on(check_app_update_from(&source, "0.1.0")).unwrap();
    assert!(info.has_update);
    assert_eq!(info.latest_version.as_deref(), Some(TAG));
    assert!(info.release_url.ends_with("/releases/tag/v9.9.9"));
    let same = ops_core::rt::block_on(check_app_update_from(&source, "9.9.9")).unwrap();
    assert!(!same.has_update);
}

#[test]
fn verified_download_reports_progress_and_leaves_no_partial_file() {
    let m = start("macos", "aarch64");
    let source = UpdateSource::with_base(&m.base);
    let dir = std::env::temp_dir().join(format!("opsbatch-update-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut seen = Vec::new();
    let path = ops_core::rt::block_on(download_update(
        &source,
        TAG,
        "macos",
        "aarch64",
        &dir,
        |d, t| seen.push((d, t)),
    ))
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), payload());
    assert_eq!(
        path.file_name().unwrap(),
        "OpsBatch-9.9.9-macos-aarch64.dmg"
    );
    assert!(
        seen.windows(2).all(|w| w[0].0 <= w[1].0),
        "progress never goes backwards"
    );
    assert_eq!(seen.first().map(|s| s.0), Some(0));
    assert_eq!(
        seen.last().copied(),
        Some((payload().len() as u64, Some(payload().len() as u64)))
    );
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".part")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tampered_missing_or_unsupported_downloads_are_rejected() {
    let m = start("windows", "x86_64");
    let source = UpdateSource::with_base(&m.base);
    let dir = std::env::temp_dir().join(format!("opsbatch-update-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let run = |os: &str, arch: &str| {
        ops_core::rt::block_on(download_update(&source, TAG, os, arch, &dir, |_, _| {}))
    };
    let empty = |dir: &std::path::Path| {
        std::fs::read_dir(dir)
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    };

    m.corrupt.store(true, Ordering::SeqCst);
    let err = run("windows", "x86_64").unwrap_err();
    assert!(err.contains("SHA-256"), "{err}");
    assert!(empty(&dir), "nothing is kept after a failed verification");

    m.corrupt.store(false, Ordering::SeqCst);
    m.no_sums.store(true, Ordering::SeqCst);
    assert!(run("windows", "x86_64")
        .unwrap_err()
        .contains("校验文件中没有"));
    m.no_sums.store(false, Ordering::SeqCst);

    assert!(run("freebsd", "x86_64").unwrap_err().contains("暂无可安装"));
    assert!(run("windows", "x86_64").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
