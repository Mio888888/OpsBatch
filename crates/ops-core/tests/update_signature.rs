//! With an updater key configured the installer needs a valid minisign signature (the Tauri updater's rule).

use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use ops_core::update::{download_update, installer_asset_name, UpdateSource};
use sha2::{Digest, Sha256};

const TAG: &str = "v9.9.9";

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn payload() -> Vec<u8> {
    (0..200_000u32).map(|i| (i % 249) as u8).collect()
}

/// Serves the installer, its checksum listing and (optionally) a signature file.
fn start(asset: String, signature: Option<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let sums = format!(
        "{}  {asset}\n",
        Sha256::digest(payload())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let signature = Arc::new(signature);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let (asset, sums, signature) = (asset.clone(), sums.clone(), signature.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                let len = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..len]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let reply = |status: &str, body: &[u8], stream: &mut std::net::TcpStream| {
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    );
                    let _ = stream.write_all(body);
                };
                if path == format!("/releases/download/{TAG}/SHA256SUMS") {
                    reply("200 OK", sums.as_bytes(), &mut stream)
                } else if path == format!("/releases/download/{TAG}/{asset}") {
                    reply("200 OK", &payload(), &mut stream)
                } else if path == format!("/releases/download/{TAG}/{asset}.sig") {
                    match signature.as_ref() {
                        Some(sig) => reply("200 OK", sig.as_bytes(), &mut stream),
                        None => reply("404 Not Found", b"", &mut stream),
                    }
                } else {
                    reply("404 Not Found", b"", &mut stream)
                }
            });
        }
    });
    base
}

fn run(source: UpdateSource, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    ops_core::rt::block_on(download_update(
        &source,
        TAG,
        "macos",
        "aarch64",
        dir,
        |_, _| {},
    ))
}

#[test]
fn a_valid_signature_is_accepted_and_everything_else_is_rejected() {
    let asset = installer_asset_name(TAG, "macos", "aarch64").unwrap();
    let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let sign = |pair: &minisign::KeyPair, data: &[u8]| {
        let sig = minisign::sign(
            Some(&pair.pk),
            &pair.sk,
            Cursor::new(data),
            Some("opsbatch"),
            None,
        )
        .unwrap();
        b64(sig.to_string().as_bytes())
    };
    let public = |pair: &minisign::KeyPair| b64(pair.pk.to_box().unwrap().to_string().as_bytes());
    let dir = std::env::temp_dir().join(format!("opsbatch-sig-{}", std::process::id()));

    // good signature
    let base = start(asset.clone(), Some(sign(&key, &payload())));
    let mut source = UpdateSource::with_base(&base);
    source.pubkey = Some(public(&key));
    let path = run(source, &dir).expect("a correctly signed installer downloads");
    assert_eq!(std::fs::read(path).unwrap(), payload());

    // no signature file at all: refused
    let base = start(asset.clone(), None);
    let mut source = UpdateSource::with_base(&base);
    source.pubkey = Some(public(&key));
    let err = run(source, &dir).unwrap_err();
    assert!(err.contains("签名"), "{err}");
    assert!(
        !dir.join(format!("{asset}.part")).exists(),
        "no partial file is left behind"
    );

    // signed by somebody else: refused
    let base = start(asset.clone(), Some(sign(&other, &payload())));
    let mut source = UpdateSource::with_base(&base);
    source.pubkey = Some(public(&key));
    assert!(run(source, &dir).unwrap_err().contains("签名"));

    // a valid signature of different bytes: refused
    let base = start(asset.clone(), Some(sign(&key, b"another installer")));
    let mut source = UpdateSource::with_base(&base);
    source.pubkey = Some(public(&key));
    assert!(run(source, &dir).unwrap_err().contains("签名"));

    // without a configured key the checksum listing alone decides, as before
    let base = start(asset, None);
    let mut source = UpdateSource::with_base(&base);
    source.pubkey = None;
    assert!(run(source, &dir).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
