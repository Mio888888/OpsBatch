//! M0-03 spike: alacritty_terminal + portable-pty + hand-painted GPUI grid.
//!
//! Run: `cargo run -p spike-terminal`
//! Env: `SPIKE_CMD="seq 1 200000"` runs a command instead of the login shell,
//!      `SPIKE_EXIT_AFTER=5` quits after N seconds and prints throughput stats.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use gpui_kit::*;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

const FONT_FAMILY: &str = "Menlo";
const FONT_SIZE: f32 = 14.0;

// Tokyo Night Storm (matches OpsBatch default terminal theme).
const BG: u32 = 0x24283b;
const FG: u32 = 0xc0caf5;
const PALETTE: [u32; 16] = [
    0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868,
    0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
];

struct TermSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone)]
struct Listener {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    title: flume::Sender<String>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::PtyWrite(text) => {
                let _ = self.writer.lock().write_all(text.as_bytes());
            }
            TermEvent::Title(t) => {
                let _ = self.title.send(t);
            }
            _ => {}
        }
    }
}

struct TerminalView {
    term: Term<Listener>,
    parser: Processor,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    cols: usize,
    lines: usize,
    focus: FocusHandle,
    bytes_total: usize,
    first_byte_at: Option<Instant>,
    last_byte_at: Option<Instant>,
    last_paint_us: u128,
    exited: bool,
    _reader: Option<Task<()>>,
}

impl TerminalView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (cols, lines) = (100usize, 30usize);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: lines as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = match std::env::var("SPIKE_CMD") {
            Ok(c) => {
                let mut b = CommandBuilder::new("/bin/sh");
                b.args(["-c", &c]);
                b
            }
            Err(_) => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                let mut b = CommandBuilder::new(shell);
                b.arg("-l");
                b
            }
        };
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Ok(home) = std::env::var("HOME") {
            cmd.cwd(home);
        }
        let _child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));

        let (tx, rx) = flume::unbounded::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let (title_tx, title_rx) = flume::unbounded::<String>();
        let listener = Listener {
            writer: writer.clone(),
            title: title_tx,
        };
        let term = Term::new(Config::default(), &TermSize { cols, lines }, listener);

        let reader_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = rx.recv_async().await {
                // Coalesce everything already queued into one UI update.
                let mut chunk = first;
                while let Ok(more) = rx.try_recv() {
                    chunk.extend_from_slice(&more);
                    if chunk.len() > 1 << 20 {
                        break;
                    }
                }
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        this.feed(&chunk);
                        while let Ok(t) = title_rx.try_recv() {
                            window.set_window_title(&t);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.exited = true;
                cx.notify();
            });
        });

        Self {
            term,
            parser: Processor::new(),
            writer,
            master: pair.master,
            cols,
            lines,
            focus: cx.focus_handle(),
            bytes_total: 0,
            first_byte_at: None,
            last_byte_at: None,
            last_paint_us: 0,
            exited: false,
            _reader: Some(reader_task),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        let now = Instant::now();
        self.first_byte_at.get_or_insert(now);
        self.last_byte_at = Some(now);
        self.bytes_total += bytes.len();
        self.parser.advance(&mut self.term, bytes);
    }

    fn write(&self, data: &[u8]) {
        let _ = self.writer.lock().write_all(data);
    }

    fn set_size(&mut self, cols: usize, lines: usize) {
        if cols == 0 || lines == 0 || (cols == self.cols && lines == self.lines) {
            return;
        }
        self.cols = cols;
        self.lines = lines;
        self.term.resize(TermSize { cols, lines });
        let _ = self.master.resize(PtySize {
            rows: lines as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let app_cursor = self.term.mode().contains(TermMode::APP_CURSOR);

        if m.platform {
            if ks.key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
                        self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
                    } else {
                        self.write(text.as_bytes());
                    }
                }
            }
            return;
        }

        let arrow = |c: char| -> Vec<u8> {
            if app_cursor {
                format!("\x1bO{c}").into_bytes()
            } else {
                format!("\x1b[{c}").into_bytes()
            }
        };
        let bytes: Option<Vec<u8>> = match ks.key.as_str() {
            "enter" => Some(b"\r".to_vec()),
            "backspace" => Some(vec![0x7f]),
            "tab" => Some(if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }),
            "escape" => Some(vec![0x1b]),
            "up" => Some(arrow('A')),
            "down" => Some(arrow('B')),
            "right" => Some(arrow('C')),
            "left" => Some(arrow('D')),
            "home" => Some(b"\x1b[H".to_vec()),
            "end" => Some(b"\x1b[F".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "space" if m.control => Some(vec![0]),
            k if m.control && k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic() => {
                Some(vec![k.as_bytes()[0].to_ascii_lowercase() - b'a' + 1])
            }
            _ => ks.key_char.as_ref().map(|s| {
                let mut v = Vec::new();
                if m.alt {
                    v.push(0x1b);
                }
                v.extend_from_slice(s.as_bytes());
                v
            }),
        };
        if let Some(b) = bytes {
            // Snap back to the bottom on input.
            self.term.scroll_display(Scroll::Bottom);
            self.write(&b);
            cx.notify();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = {
            let secs = match (self.first_byte_at, self.last_byte_at) {
                (Some(a), Some(b)) => (b - a).as_secs_f64(),
                _ => 0.0,
            };
            format!(
                "{}x{} | bytes {} | fed in {:.2}s ({:.1} MB/s) | last paint {} us{}",
                self.cols,
                self.lines,
                self.bytes_total,
                secs,
                if secs > 0.0 {
                    self.bytes_total as f64 / secs / 1e6
                } else {
                    0.0
                },
                self.last_paint_us,
                if self.exited { " | child exited" } else { "" }
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .child(
                div()
                    .id("term")
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| this.on_key(ev, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                    )
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _w, cx| {
                        let lh = px(FONT_SIZE * 1.35);
                        let lines = (ev.delta.pixel_delta(lh).y / lh).round() as i32;
                        if lines == 0 {
                            return;
                        }
                        let mode = *this.term.mode();
                        if mode.contains(TermMode::ALT_SCREEN) {
                            let key: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                            for _ in 0..lines.unsigned_abs().min(10) {
                                this.write(key);
                            }
                        } else {
                            this.term.scroll_display(Scroll::Delta(lines));
                            cx.notify();
                        }
                    }))
                    .child(TerminalElement { view: cx.entity() }),
            )
            .child(
                div()
                    .h(px(18.))
                    .px_2()
                    .text_size(px(11.))
                    .text_color(rgb(0x7aa2f7))
                    .child(stats),
            )
    }
}

struct TerminalElement {
    view: Entity<TerminalView>,
}

struct Metrics {
    cell_w: Pixels,
    line_h: Pixels,
}

fn color_to_rgb(c: Color, is_fg: bool) -> Rgb {
    let from_u32 = |v: u32| Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    };
    match c {
        Color::Spec(rgb) => rgb,
        Color::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground => from_u32(FG),
            NamedColor::Background => from_u32(BG),
            NamedColor::Cursor => from_u32(FG),
            n => {
                let i = n as usize;
                if i < 16 {
                    from_u32(PALETTE[i])
                } else if is_fg {
                    from_u32(FG)
                } else {
                    from_u32(BG)
                }
            }
        },
        Color::Indexed(i) => match i {
            0..=15 => from_u32(PALETTE[i as usize]),
            16..=231 => {
                let i = i - 16;
                let lvl = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb {
                    r: lvl(i / 36),
                    g: lvl((i / 6) % 6),
                    b: lvl(i % 6),
                }
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        },
    }
}

fn hsla_of(c: Rgb) -> Hsla {
    rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32).into()
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Metrics;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let ts = window.text_system();
        let font = font(FONT_FAMILY);
        let font_id = ts.resolve_font(&font);
        let fs = px(FONT_SIZE);
        let cell_w = ts
            .advance(font_id, fs, 'm')
            .map(|s| s.width)
            .unwrap_or(px(8.4));
        let line_h = px((FONT_SIZE * 1.35).round());
        let cols = (bounds.size.width / cell_w).floor() as usize;
        let lines = (bounds.size.height / line_h).floor() as usize;
        self.view.update(cx, |v, _| v.set_size(cols, lines));
        Metrics { cell_w, line_h }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        m: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let t0 = Instant::now();
        let fs = px(FONT_SIZE);
        let base_font = font(FONT_FAMILY);
        let focused = self.view.read(cx).focus.is_focused(window);

        struct Seg {
            col: usize,
            width_cols: usize,
            text: String,
            run: TextRun,
        }

        let view = self.view.read(cx);
        let content = view.term.renderable_content();
        let display_offset = content.display_offset as i32;
        let cursor = content.cursor;
        let n_lines = view.lines;
        let n_cols = view.cols;

        let mut rows: Vec<Vec<Seg>> = (0..n_lines).map(|_| Vec::new()).collect();
        let mut bg_quads: Vec<(usize, usize, usize, Hsla)> = Vec::new(); // row, col, ncols, color

        for ic in content.display_iter {
            let row = ic.point.line.0 + display_offset;
            if row < 0 || row as usize >= n_lines {
                continue;
            }
            let row = row as usize;
            let col = ic.point.column.0;
            if col >= n_cols {
                continue;
            }
            let cell = &*ic;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let wide = cell.flags.contains(Flags::WIDE_CHAR);
            let mut fg = color_to_rgb(cell.fg, true);
            let mut bg = color_to_rgb(cell.bg, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let ncols = if wide { 2 } else { 1 };

            let default_bg = matches!(cell.bg, Color::Named(NamedColor::Background))
                && !cell.flags.contains(Flags::INVERSE);
            if !default_bg {
                let c = hsla_of(bg);
                match bg_quads.last_mut() {
                    Some((r, c0, n, last)) if *r == row && *c0 + *n == col && *last == c => {
                        *n += ncols
                    }
                    _ => bg_quads.push((row, col, ncols, c)),
                }
            }

            if cell.c == ' ' || cell.c == '\0' || cell.flags.contains(Flags::HIDDEN) {
                continue;
            }
            let mut fg_h = hsla_of(fg);
            if cell.flags.intersects(Flags::DIM) {
                fg_h.a = 0.6;
            }
            let mut f = base_font.clone();
            if cell.flags.contains(Flags::BOLD) {
                f.weight = FontWeight::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                f.style = FontStyle::Italic;
            }
            let underline = cell
                .flags
                .intersects(Flags::ALL_UNDERLINES)
                .then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(fg_h),
                    wavy: cell.flags.contains(Flags::UNDERCURL),
                });
            let run = TextRun {
                len: cell.c.len_utf8(),
                font: f,
                color: fg_h,
                background_color: None,
                underline,
                strikethrough: cell
                    .flags
                    .contains(Flags::STRIKEOUT)
                    .then(|| StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(fg_h),
                    }),
            };

            // Merge adjacent narrow cells with an identical style into one shaped segment;
            // wide chars always get their own segment so columns stay aligned for CJK.
            let segs = &mut rows[row];
            if !wide {
                if let Some(last) = segs.last_mut() {
                    if last.width_cols == last.text.chars().count()
                        && last.col + last.width_cols == col
                        && last.run.font == run.font
                        && last.run.color == run.color
                        && last.run.underline == run.underline
                        && last.run.strikethrough == run.strikethrough
                    {
                        last.text.push(cell.c);
                        last.width_cols += 1;
                        last.run.len += run.len;
                        continue;
                    }
                }
            }
            segs.push(Seg {
                col,
                width_cols: ncols,
                text: cell.c.to_string(),
                run,
            });
        }

        // Paint.
        window.paint_quad(fill(bounds, rgb(BG)));
        for (row, col, n, color) in bg_quads {
            let origin = point(
                bounds.origin.x + m.cell_w * col as f32,
                bounds.origin.y + m.line_h * row as f32,
            );
            window.paint_quad(fill(
                Bounds::new(origin, size(m.cell_w * n as f32, m.line_h)),
                color,
            ));
        }

        // Cursor under text.
        if cursor.shape != CursorShape::Hidden {
            let row = cursor.point.line.0 + display_offset;
            if row >= 0 && (row as usize) < n_lines {
                let origin = point(
                    bounds.origin.x + m.cell_w * cursor.point.column.0 as f32,
                    bounds.origin.y + m.line_h * row as f32,
                );
                let b = Bounds::new(origin, size(m.cell_w, m.line_h));
                if focused {
                    window.paint_quad(fill(
                        b,
                        hsla_of(Rgb {
                            r: 0xc0,
                            g: 0xca,
                            b: 0xf5,
                        }),
                    ));
                } else {
                    window.paint_quad(outline(b, rgb(FG), BorderStyle::Solid));
                }
            }
        }

        let cursor_cell = (cursor.point.line.0 + display_offset, cursor.point.column.0);
        for (row, segs) in rows.into_iter().enumerate() {
            for mut seg in segs {
                // Flip glyph color on the (focused) cursor cell for contrast.
                if focused
                    && cursor.shape != CursorShape::Hidden
                    && (row as i32, seg.col) == cursor_cell
                {
                    seg.run.color = rgb(BG).into();
                }
                let shaped = window.text_system().shape_line(
                    seg.text.into(),
                    fs,
                    &[seg.run],
                    Some(m.cell_w),
                );
                let origin = point(
                    bounds.origin.x + m.cell_w * seg.col as f32,
                    bounds.origin.y + m.line_h * row as f32,
                );
                let _ = shaped.paint(origin, m.line_h, TextAlign::Left, None, window, cx);
            }
        }

        let us = t0.elapsed().as_micros();
        self.view.update(cx, |v, _| v.last_paint_us = us);
    }
}

fn main() {
    let exit_after: Option<u64> = std::env::var("SPIKE_EXIT_AFTER")
        .ok()
        .and_then(|s| s.parse().ok());
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let mut view_handle = None;
            let (w, h) = std::env::var("SPIKE_SIZE")
                .ok()
                .and_then(|s| {
                    s.split_once('x')
                        .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
                })
                .unwrap_or((1000., 640.));
            let scroll_up: Option<i32> = std::env::var("SPIKE_SCROLL_UP")
                .ok()
                .and_then(|s| s.parse().ok());
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(w), px(h)),
                    cx,
                ))),
                ..Default::default()
            };
            let handle = gpui_kit::open_window(opts, cx, |window, cx| {
                let v = cx.new(|cx| TerminalView::new(window, cx));
                window.focus(&v.read(cx).focus.clone(), cx);
                view_handle = Some(v.clone());
                v
            })
            .expect("open window");
            let _ = handle;
            // Test hook: scroll the viewport back N lines after output settles.
            if let (Some(n), Some(v)) = (scroll_up, view_handle.clone()) {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_secs(4)).await;
                    cx.update(|cx| {
                        v.update(cx, |v, cx| {
                            v.term.scroll_display(Scroll::Delta(n));
                            cx.notify();
                        })
                    });
                })
                .detach();
            }
            if let (Some(secs), Some(v)) = (exit_after, view_handle) {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs(secs))
                        .await;
                    cx.update(|cx| {
                        let v = v.read(cx);
                        let secs = match (v.first_byte_at, v.last_byte_at) {
                            (Some(a), Some(b)) => (b - a).as_secs_f64(),
                            _ => 0.0,
                        };
                        eprintln!(
                            "STATS bytes={} feed_secs={:.3} mbps={:.2} last_paint_us={}",
                            v.bytes_total,
                            secs,
                            if secs > 0.0 {
                                v.bytes_total as f64 / secs / 1e6
                            } else {
                                0.0
                            },
                            v.last_paint_us
                        );
                        cx.quit();
                    });
                })
                .detach();
            }
        });
}
