//! M0-05 spike (headless parts):
//!  1. the OpsBatch ironrdp stack (with vendored patches) builds inside this workspace;
//!  2. H.264 decode throughput with openh264 (Annex-B file produced by ffmpeg);
//!  3. cost of turning decoded frames into the BGRA buffer GPUI's `RenderImage` wants.
//!
//! Run: cargo run --release -p spike-rdp -- target/spike-data/test1080.h264

use std::time::Instant;

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

// Touch the patched crates so the linker really pulls them in.
#[allow(dead_code)]
fn _link_check() {
    let _ = std::any::type_name::<ironrdp::connector::ClientConnector>();
    let _ = std::any::type_name::<ironrdp::session::ActiveStage>();
    let _ = std::any::type_name::<ironrdp_dvc::DrdynvcClient>();
}

/// Split an Annex-B stream into NAL units (start code included).
fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0
            && data[i + 1] == 0
            && (data[i + 2] == 1 || (data[i + 2] == 0 && i + 4 <= data.len() && data[i + 3] == 1))
        {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| &data[s..starts.get(n + 1).copied().unwrap_or(data.len())])
        .collect()
}

fn main() -> anyhow_free::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/spike-data/test1080.h264".into());
    let data = std::fs::read(&path)?;
    let nals = nal_units(&data);
    println!("{path}: {} bytes, {} NAL units", data.len(), nals.len());

    let mut decoder = Decoder::new()?;
    let (mut frames, mut dec_ns, mut conv_ns) = (0usize, 0u128, 0u128);
    let (mut w, mut h) = (0, 0);
    let mut bgra: Vec<u8> = Vec::new();
    let t_all = Instant::now();
    for nal in nals {
        let t = Instant::now();
        let out = decoder.decode(nal)?;
        dec_ns += t.elapsed().as_nanos();
        if let Some(yuv) = out {
            let (dw, dh) = yuv.dimensions();
            if (dw, dh) != (w, h) {
                (w, h) = (dw, dh);
                bgra = vec![0; w * h * 4];
            }
            let t = Instant::now();
            // write_rgba8 gives RGBA; swap R/B to BGRA like gpui's RenderImage expects.
            yuv.write_rgba8(&mut bgra);
            for px in bgra.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            conv_ns += t.elapsed().as_nanos();
            frames += 1;
        }
    }
    let total = t_all.elapsed().as_secs_f64();
    println!(
        "{w}x{h}: {frames} frames in {total:.2}s = {:.0} fps | decode {:.2} ms/frame | yuv->bgra {:.2} ms/frame",
        frames as f64 / total,
        dec_ns as f64 / frames.max(1) as f64 / 1e6,
        conv_ns as f64 / frames.max(1) as f64 / 1e6,
    );
    Ok(())
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
