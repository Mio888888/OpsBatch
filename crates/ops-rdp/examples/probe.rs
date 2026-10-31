//! Live RDP probe: connects to a stored Windows host through the same code path as the app and reports connect time,
//! frame cadence, stalls and status changes.
//!
//!     cargo run -p ops-rdp --example probe -- <data-dir> <host-id> [seconds] [h264|bitmap]
//!
//! The data directory is copied to a temporary one first, so the database of the app is never touched. Credentials
//! stay inside the copied database and are never printed. Interaction is limited to mouse movement and opening /
//! closing the Start menu (Win, Esc) to make the desktop repaint.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ops_rdp::{
    RdpConnectRequest, RdpInputEvent, RdpManager, RdpMetricsPayload, RdpStatusPayload,
    RdpTransportMode,
};

#[derive(Default)]
struct Stats {
    frames: u64,
    bytes: u64,
    pixels: u64,
    first_frame: Option<Duration>,
    last_frame: Option<Instant>,
    max_gap: Duration,
    gaps_over_250ms: u32,
    per_second: Vec<u32>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (data_dir, host_id) = (
        args.get(1).expect("data dir"),
        args.get(2).expect("host id"),
    );
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);
    let mode = match args.get(4).map(String::as_str) {
        Some("bitmap") => RdpTransportMode::LegacyBitmap,
        _ => RdpTransportMode::H264Direct,
    };
    let tmp = std::env::temp_dir().join(format!("opsbatch-rdp-probe-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    for name in ["opsbatch.db", "opsbatch.db-wal", "opsbatch.db-shm"] {
        let src = std::path::Path::new(data_dir).join(name);
        if src.exists() {
            std::fs::copy(&src, tmp.join(name)).unwrap();
        }
    }
    let core = ops_core::Core::open(&tmp).expect("open the copied data directory");
    // Passwords of the build without the encrypted vault live in memory only, so the probe takes it from the
    // environment of whoever runs it (nothing is printed or written):  OPSBATCH_PROBE_PASSWORD='...' probe ...
    match std::env::var("OPSBATCH_PROBE_PASSWORD") {
        Ok(password) if !password.is_empty() => ops_core::keychain::store_host_password(host_id, &password).expect("keep the password in memory"),
        _ => println!("note: OPSBATCH_PROBE_PASSWORD is not set; the stored credential is only available inside a running app"),
    }
    let manager = Arc::new(RdpManager::new());

    let session_id = format!("probe-{}", std::process::id());
    let status_rx = core.events.subscribe(format!("rdp-status-{session_id}"));
    let metrics_rx = core.events.subscribe(format!("rdp-metrics-{session_id}"));
    let (frame_tx, frame_rx) = flume::unbounded::<Vec<u8>>();
    let stats = Arc::new(Mutex::new(Stats::default()));
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));

    // frame consumer
    let (s2, stop2) = (stats.clone(), stop.clone());
    let consumer = std::thread::spawn(move || {
        let mut second_count = 0u32;
        let mut second_start = Instant::now();
        while !stop2.load(Ordering::Relaxed) {
            match frame_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => {
                    let now = Instant::now();
                    let mut s = s2.lock().unwrap();
                    if frame.len() >= 16 {
                        let (rw, rh) = (
                            u16::from_le_bytes([frame[8], frame[9]]) as u64,
                            u16::from_le_bytes([frame[10], frame[11]]) as u64,
                        );
                        s.pixels += rw * rh;
                    }
                    s.frames += 1;
                    s.bytes += frame.len() as u64;
                    if s.first_frame.is_none() {
                        s.first_frame = Some(started.elapsed());
                    }
                    if let Some(last) = s.last_frame {
                        let gap = now - last;
                        s.max_gap = s.max_gap.max(gap);
                        if gap > Duration::from_millis(250) {
                            s.gaps_over_250ms += 1;
                        }
                    }
                    s.last_frame = Some(now);
                    second_count += 1;
                    if second_start.elapsed() >= Duration::from_secs(1) {
                        s.per_second.push(second_count);
                        second_count = 0;
                        second_start = Instant::now();
                    }
                }
                Err(_) => {
                    if second_start.elapsed() >= Duration::from_secs(1) {
                        s2.lock().unwrap().per_second.push(second_count);
                        second_count = 0;
                        second_start = Instant::now();
                    }
                }
            }
        }
    });

    let request = RdpConnectRequest {
        host_id: host_id.clone(),
        session_id: Some(session_id.clone()),
        width: None,
        height: None,
        domain: None,
        transport_mode: Some(mode),
    };
    println!("connecting ({mode:?}) ...");
    let t0 = Instant::now();
    let result = ops_core::rt::block_on(manager.connect_host(&core, request, frame_tx));
    let connect_time = t0.elapsed();
    let response = match result {
        Ok(r) => r,
        Err(e) => {
            println!(
                "CONNECT FAILED after {:.1}s: {e}",
                connect_time.as_secs_f32()
            );
            while let Ok(ev) = status_rx.try_recv() {
                println!(
                    "status: {:?}",
                    ev.parse::<RdpStatusPayload>().map(|s| (s.state, s.message))
                );
            }
            return;
        }
    };
    println!(
        "connected in {:.1}s ({}x{})",
        connect_time.as_secs_f32(),
        response.width,
        response.height
    );

    // light interaction: sweeps of the mouse and Start menu open / close cycles
    let send = |event: RdpInputEvent| {
        let _ = manager.send_input(&core, &session_id, event);
    };
    let begin = Instant::now();
    let (w, h) = (response.width, response.height);
    let mut tick = 0u32;
    while begin.elapsed() < Duration::from_secs(seconds) {
        let t = begin.elapsed().as_secs_f32();
        if t > 4.0 {
            let phase = (t * 1.5).sin() * 0.5 + 0.5;
            send(RdpInputEvent::MouseMove {
                x: (phase * (w as f32 - 40.0)) as u16 + 20,
                y: ((1.0 - phase) * (h as f32 - 80.0)) as u16 + 20,
            });
        }
        if t > 4.0 && tick.is_multiple_of(80) {
            // Start menu: Win down/up, later Esc
            send(RdpInputEvent::KeyScancode {
                code: 0x5B,
                extended: true,
                down: true,
            });
            send(RdpInputEvent::KeyScancode {
                code: 0x5B,
                extended: true,
                down: false,
            });
        }
        if t > 4.0 && tick % 80 == 40 {
            send(RdpInputEvent::KeyScancode {
                code: 0x01,
                extended: false,
                down: true,
            });
            send(RdpInputEvent::KeyScancode {
                code: 0x01,
                extended: false,
                down: false,
            });
        }
        tick += 1;
        while let Ok(ev) = status_rx.try_recv() {
            if let Some(s) = ev.parse::<RdpStatusPayload>() {
                println!(
                    "[{:>5.1}s] status {} {}",
                    begin.elapsed().as_secs_f32(),
                    s.state,
                    s.message.unwrap_or_default()
                );
            }
        }
        while let Ok(ev) = metrics_rx.try_recv() {
            if let Some(m) = ev.parse::<RdpMetricsPayload>() {
                println!(
                    "[{:>5.1}s] server {}/s sent {}/s coalesced {}/s {:.2} MB/s",
                    begin.elapsed().as_secs_f32(),
                    m.server_updates_per_second,
                    m.sent_frames_per_second,
                    m.coalesced_updates_per_second,
                    m.sent_mbytes_per_second
                );
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    send(RdpInputEvent::KeyScancode {
        code: 0x01,
        extended: false,
        down: true,
    });
    send(RdpInputEvent::KeyScancode {
        code: 0x01,
        extended: false,
        down: false,
    });
    let alive = manager.has_session(&session_id);
    let _ = manager.disconnect(&session_id);
    stop.store(true, Ordering::Relaxed);
    let _ = consumer.join();
    while let Ok(ev) = status_rx.try_recv() {
        if let Some(s) = ev.parse::<RdpStatusPayload>() {
            println!("final status {} {}", s.state, s.message.unwrap_or_default());
        }
    }
    let s = stats.lock().unwrap();
    let active: Vec<u32> = s.per_second.iter().copied().filter(|n| *n > 0).collect();
    println!("--- summary ({mode:?}) ---");
    println!("session still alive at the end: {alive}");
    println!("first frame after {:?}", s.first_frame);
    println!(
        "frames {} | {:.1} MB | {:.1} Mpixels",
        s.frames,
        s.bytes as f64 / 1e6,
        s.pixels as f64 / 1e6
    );
    println!("frames/s per second: {:?}", s.per_second);
    println!(
        "active seconds {} | avg fps while active {:.1} | max gap {:?} | gaps > 250ms: {}",
        active.len(),
        active.iter().sum::<u32>() as f32 / active.len().max(1) as f32,
        s.max_gap,
        s.gaps_over_250ms
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
