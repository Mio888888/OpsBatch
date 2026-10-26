//! M0-05 (GPU part): can GPUI present full-screen decoded video frames fast enough for a remote desktop?
//!
//! A worker thread decodes an H.264 file (loops) -> BGRA frames -> bounded channel. The UI thread turns each frame
//! into a `RenderImage` (what a real RDP session would do with every bitmap/H.264 update), shows it with `img()`
//! and drops the previous one. We count presented frames per second and per-frame UI-thread cost.
//!
//! Run: SPIKE_FILE=target/spike-data/test1080.h264 SPIKE_EXIT_AFTER=8 cargo run --release -p spike-rdp-gpu

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use image::{Frame, RgbaImage};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

struct Frame4 {
    w: u32,
    h: u32,
    bgra: Vec<u8>,
}

fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && (data[i + 2] == 1 || (data[i + 2] == 0 && i + 4 <= data.len() && data[i + 3] == 1)) {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts.iter().enumerate().map(|(n, &s)| &data[s..starts.get(n + 1).copied().unwrap_or(data.len())]).collect()
}

struct Stats {
    presented: u64,
    started: Instant,
    ui_cost_us: u128,
    max_ui_cost_us: u128,
    last_second: (Instant, u64),
    fps_now: f64,
}

struct VideoView {
    render_started: Rc<Cell<Option<Instant>>>,
    paint_total_us: Rc<Cell<(u64, u128, u128)>>, // frames, sum, max (render start -> end of paint)
    last_render: Option<Instant>,
    interval_sum_us: u128,
    interval_n: u64,
    rx: flume::Receiver<Frame4>,
    current: Option<Arc<RenderImage>>,
    stats: Stats,
}

impl VideoView {
    fn new(path: String) -> Self {
        let (tx, rx) = flume::bounded::<Frame4>(2);
        std::thread::spawn(move || {
            let data = std::fs::read(&path).expect("read h264 file");
            let nals = nal_units(&data);
            let mut dec = Decoder::new().expect("decoder");
            let mut buf: Vec<u8> = Vec::new();
            loop {
                for nal in &nals {
                    if let Ok(Some(yuv)) = dec.decode(nal) {
                        let (w, h) = yuv.dimensions();
                        buf.resize(w * h * 4, 0);
                        yuv.write_rgba8(&mut buf);
                        for px in buf.chunks_exact_mut(4) {
                            px.swap(0, 2); // RGBA -> BGRA
                        }
                        if tx.send(Frame4 { w: w as u32, h: h as u32, bgra: buf.clone() }).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let now = Instant::now();
        Self {
            rx,
            current: None,
            render_started: Rc::new(Cell::new(None)),
            paint_total_us: Rc::new(Cell::new((0, 0, 0))),
            last_render: None,
            interval_sum_us: 0,
            interval_n: 0,
            stats: Stats { presented: 0, started: now, ui_cost_us: 0, max_ui_cost_us: 0, last_second: (now, 0), fps_now: 0.0 },
        }
    }
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t0 = Instant::now();
        if let Some(l) = self.last_render.replace(t0) {
            self.interval_sum_us += l.elapsed().as_micros();
            self.interval_n += 1;
        }
        self.render_started.set(Some(t0));
        if let Ok(f) = self.rx.try_recv() {
            let img = RgbaImage::from_raw(f.w, f.h, f.bgra).expect("frame size"); // bytes are BGRA; gpui treats them as BGRA
            let render = Arc::new(RenderImage::new(vec![Frame::new(img)]));
            if let Some(old) = self.current.replace(render) {
                let _ = window.drop_image(old);
            }
            let cost = t0.elapsed().as_micros();
            let s = &mut self.stats;
            s.presented += 1;
            s.ui_cost_us += cost;
            s.max_ui_cost_us = s.max_ui_cost_us.max(cost);
            if s.last_second.0.elapsed() >= Duration::from_secs(1) {
                s.fps_now = (s.presented - s.last_second.1) as f64 / s.last_second.0.elapsed().as_secs_f64();
                s.last_second = (Instant::now(), s.presented);
            }
        }
        window.request_animation_frame(); // keep pulling frames

        let s = &self.stats;
        let avg = if s.presented > 0 { s.ui_cost_us / s.presented as u128 } else { 0 };
        let label = format!(
            "presented {} | now {:.1} fps | avg {:.1} fps | RenderImage build+swap avg {} us max {} us",
            s.presented,
            s.fps_now,
            s.presented as f64 / s.started.elapsed().as_secs_f64(),
            avg,
            s.max_ui_cost_us
        );
        let started = self.render_started.clone();
        let paint_acc = self.paint_total_us.clone();
        div()
            .size_full()
            .bg(rgb(0x101018))
            .flex()
            .flex_col()
            .child(div().h(px(22.)).px_2().text_size(px(12.)).text_color(rgb(0x7aa2f7)).child(label))
            .child(match &self.current {
                Some(img_) => gpui_kit::img(ImageSource::Render(img_.clone())).flex_1().min_h_0().w_full().object_fit(ObjectFit::Contain).into_any_element(),
                None => div().flex_1().child("waiting for first frame").into_any_element(),
            })
            .child(canvas(|_, _, _| (), {
                let started = started.clone();
                let acc = paint_acc.clone();
                move |_, _, _, _| {
                    // runs in paint order; the img above has already uploaded its texture by now
                    if let Some(t) = started.get() {
                        let us = t.elapsed().as_micros();
                        let (n, sum, max) = acc.get();
                        acc.set((n + 1, sum + us, max.max(us)));
                    }
                }
            }).h(px(0.)))
    }
}

fn main() {
    let path = std::env::var("SPIKE_FILE").unwrap_or_else(|_| "target/spike-data/test1080.h264".into());
    let exit_after: u64 = std::env::var("SPIKE_EXIT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(760.)), cx))),
            inactive_frame_interval: if std::env::var("SPIKE_THROTTLE").is_ok() { WindowOptions::default().inactive_frame_interval } else { None },
            ..Default::default()
        };
        let mut handle = None;
        gpui_kit::open_window(opts, cx, |_, cx| {
            let v = cx.new(|_| VideoView::new(path));
            handle = Some(v.clone());
            v
        })
        .expect("window");
        if exit_after > 0 {
            let v = handle.unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(exit_after)).await;
                let _ = cx.update(|cx| {
                    let s = &v.read(cx).stats;
                    let secs = s.started.elapsed().as_secs_f64();
                    let vv = v.read(cx);
                    let (pn, psum, pmax) = vv.paint_total_us.get();
                    eprintln!(
                        "GPU_TIMING render_calls={} avg_frame_interval_us={} paint_done_avg_us={} paint_done_max_us={}",
                        vv.interval_n,
                        if vv.interval_n > 0 { vv.interval_sum_us / vv.interval_n as u128 } else { 0 },
                        if pn > 0 { psum / pn as u128 } else { 0 },
                        pmax
                    );
                    eprintln!(
                        "GPU_STATS presented={} secs={:.2} avg_fps={:.1} ui_cost_avg_us={} ui_cost_max_us={}",
                        s.presented,
                        secs,
                        s.presented as f64 / secs,
                        if s.presented > 0 { s.ui_cost_us / s.presented as u128 } else { 0 },
                        s.max_ui_cost_us
                    );
                    cx.quit();
                });
            })
            .detach();
        }
    });
}
