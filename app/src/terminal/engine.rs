//! Terminal emulation without any UI dependency: an alacritty `Term` plus the byte parser, with the extras the
//! views need (selection, search lines, full-buffer text, colour answers for programs that query the palette).

use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use ops_core::theme::{parse_css_color, TerminalPalette};

/// Palette used to resolve ANSI/indexed colours (taken from the selected terminal theme).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalColors {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

fn rgb_of(css: &str) -> Rgb {
    parse_css_color(css)
        .map(|c| Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        })
        .unwrap_or(Rgb {
            r: 255,
            g: 0,
            b: 255,
        })
}

impl TerminalColors {
    pub fn from_palette(p: &TerminalPalette) -> Self {
        let names = p.ansi16();
        let mut ansi = [Rgb { r: 0, g: 0, b: 0 }; 16];
        for (slot, name) in ansi.iter_mut().zip(names) {
            *slot = rgb_of(name);
        }
        Self {
            foreground: rgb_of(p.foreground),
            background: rgb_of(p.background),
            cursor: rgb_of(p.cursor),
            selection: rgb_of(p.selection_background),
            ansi,
        }
    }

    /// 256-colour palette entry (16..=231 colour cube, 232..=255 greys).
    pub fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb {
                    r: level(i / 36),
                    g: level((i / 6) % 6),
                    b: level(i % 6),
                }
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    /// Resolve a cell colour. `overrides` are colours a program changed at runtime (OSC 4/10/11).
    pub fn resolve(
        &self,
        color: Color,
        overrides: &alacritty_terminal::term::color::Colors,
        is_fg: bool,
    ) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => overrides[i as usize].unwrap_or_else(|| self.indexed(i)),
            Color::Named(n) => {
                if let Some(rgb) = overrides[n as usize] {
                    return rgb;
                }
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
                    NamedColor::DimForeground => dim(self.foreground),
                    NamedColor::Background => self.background,
                    NamedColor::Cursor => self.cursor,
                    n => {
                        let i = n as usize;
                        if i < 16 {
                            self.ansi[i]
                        } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                            .contains(&i)
                        {
                            dim(self.ansi[i - NamedColor::DimBlack as usize])
                        } else if is_fg {
                            self.foreground
                        } else {
                            self.background
                        }
                    }
                }
            }
        }
    }
}

fn dim(c: Rgb) -> Rgb {
    Rgb {
        r: (c.r as u16 * 2 / 3) as u8,
        g: (c.g as u16 * 2 / 3) as u8,
        b: (c.b as u16 * 2 / 3) as u8,
    }
}

/// What the emulator asks its host to do after consuming output.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EngineOutput {
    /// Replies for the PTY (device attributes, cursor position, colour queries, ...), already formatted.
    pub writes: Vec<String>,
    /// `Some(None)` = reset to the default title.
    pub title: Option<Option<String>>,
    pub bell: bool,
    pub clipboard_store: Option<String>,
}

#[derive(Clone)]
pub struct Listener {
    tx: flume::Sender<TermEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.tx.send(event);
    }
}

struct Size {
    cols: usize,
    lines: usize,
}

impl Dimensions for Size {
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

/// One searchable text row: absolute row (0 = oldest scrollback line) and the column of every character.
pub struct SearchRow {
    pub row: usize,
    pub text: String,
    pub columns: Vec<usize>,
}

pub struct Engine {
    term: Term<Listener>,
    parser: Processor,
    events: flume::Receiver<TermEvent>,
    pub colors: TerminalColors,
    cols: usize,
    lines: usize,
    cell_px: (u16, u16),
}

impl Engine {
    pub fn new(cols: usize, lines: usize, scrollback: usize, colors: TerminalColors) -> Self {
        let (tx, events) = flume::unbounded();
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let (cols, lines) = (cols.max(2), lines.max(1));
        let term = Term::new(config, &Size { cols, lines }, Listener { tx });
        Self {
            term,
            parser: Processor::new(),
            events,
            colors,
            cols,
            lines,
            cell_px: (8, 16),
        }
    }

    pub fn term(&self) -> &Term<Listener> {
        &self.term
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn set_cell_px(&mut self, w: u16, h: u16) {
        self.cell_px = (w, h);
    }

    pub fn set_scrollback(&mut self, scrollback: usize) {
        self.term.set_options(Config {
            scrolling_history: scrollback,
            ..Config::default()
        });
    }

    /// Feed PTY output and collect what the emulator wants to send back.
    pub fn feed(&mut self, bytes: &[u8]) -> EngineOutput {
        self.parser.advance(&mut self.term, bytes);
        self.drain()
    }

    fn drain(&mut self) -> EngineOutput {
        let mut out = EngineOutput::default();
        while let Ok(event) = self.events.try_recv() {
            match event {
                TermEvent::PtyWrite(text) => out.writes.push(text),
                TermEvent::Title(t) => out.title = Some(Some(t)),
                TermEvent::ResetTitle => out.title = Some(None),
                TermEvent::Bell => out.bell = true,
                TermEvent::ClipboardStore(_, text) => out.clipboard_store = Some(text),
                TermEvent::ColorRequest(index, format) => {
                    let c = match index {
                        256 => self.colors.foreground,
                        257 => self.colors.background,
                        258 => self.colors.cursor,
                        i => self.term.colors()[i]
                            .unwrap_or_else(|| self.colors.indexed(i.min(255) as u8)),
                    };
                    out.writes.push(format(c));
                }
                TermEvent::TextAreaSizeRequest(format) => out.writes.push(format(WindowSize {
                    num_cols: self.cols as u16,
                    num_lines: self.lines as u16,
                    cell_width: self.cell_px.0,
                    cell_height: self.cell_px.1,
                })),
                _ => {}
            }
        }
        out
    }

    /// Returns true when the size actually changed.
    pub fn resize(&mut self, cols: usize, lines: usize) -> bool {
        let (cols, lines) = (cols.max(2), lines.max(1));
        if cols == self.cols && lines == self.lines {
            return false;
        }
        self.cols = cols;
        self.lines = lines;
        self.term.resize(Size { cols, lines });
        true
    }

    // ---- scrolling ----

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
    }

    // ---- coordinates ----

    /// Viewport cell -> grid point (accounts for the scrolled-back display).
    pub fn point_at(&self, col: usize, row: usize) -> Point {
        let line = row as i32 - self.display_offset() as i32;
        Point::new(Line(line), Column(col.min(self.cols.saturating_sub(1))))
    }

    // ---- selection ----

    pub fn start_selection(&mut self, kind: SelectionType, col: usize, row: usize, side: Side) {
        let p = self.point_at(col, row);
        self.term.selection = Some(Selection::new(kind, p, side));
    }

    pub fn update_selection(&mut self, col: usize, row: usize, side: Side) {
        let p = self.point_at(col, row);
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(p, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn select_all(&mut self) {
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(self.term.bottommost_line(), Column(self.cols - 1));
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        self.term.selection = Some(sel);
    }

    // ---- text ----

    /// The whole buffer (scrollback + screen) as plain text, trailing blank lines removed.
    pub fn buffer_text(&self) -> String {
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(self.term.bottommost_line(), Column(self.cols - 1));
        let text = self.term.bounds_to_string(start, end);
        text.trim_end().to_string()
    }

    /// Text of the visible screen only.
    pub fn screen_text(&self) -> String {
        let start = Point::new(Line(0), Column(0));
        let end = Point::new(Line(self.lines as i32 - 1), Column(self.cols - 1));
        self.term
            .bounds_to_string(start, end)
            .trim_end()
            .to_string()
    }

    /// Text of the screen row under the cursor, trimmed.
    pub fn cursor_line_text(&self) -> String {
        let (_, row) = self.cursor();
        let p = Point::new(Line(row as i32), Column(0));
        let end = Point::new(Line(row as i32), Column(self.cols - 1));
        self.term.bounds_to_string(p, end).trim().to_string()
    }

    /// Cursor position on the screen (column, row).
    pub fn cursor(&self) -> (usize, usize) {
        let p = self.term.grid().cursor.point;
        (p.column.0, p.line.0.max(0) as usize)
    }

    // ---- search ----

    pub fn search_rows(&self) -> Vec<SearchRow> {
        let grid = self.term.grid();
        let top = self.term.topmost_line();
        let bottom = self.term.bottommost_line();
        let history = grid.history_size() as i32;
        let mut rows = Vec::new();
        let mut line = top;
        while line <= bottom {
            let mut text = String::new();
            let mut columns = Vec::new();
            for c in 0..self.cols {
                let cell = &grid[Point::new(line, Column(c))];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = if cell.c == '\0' { ' ' } else { cell.c };
                text.push(ch);
                columns.push(c);
            }
            // translateToString(true): trailing blanks are not searchable.
            let trimmed = text.trim_end_matches(' ').chars().count();
            text = text.chars().take(trimmed).collect();
            columns.truncate(trimmed);
            rows.push(SearchRow {
                row: (line.0 + history) as usize,
                text,
                columns,
            });
            line += 1;
        }
        rows
    }

    /// Text of one visible row (what is on screen, honouring scrollback) and the cell column of each char.
    pub fn viewport_row(&self, row: usize) -> (String, Vec<usize>) {
        let grid = self.term.grid();
        let line = Line(row as i32 - self.display_offset() as i32);
        let (mut text, mut columns) = (String::new(), Vec::new());
        for c in 0..self.cols {
            let cell = &grid[Point::new(line, Column(c))];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            text.push(if cell.c == '\0' { ' ' } else { cell.c });
            columns.push(c);
        }
        (text, columns)
    }

    /// Select `len` characters starting at character `char_col` of absolute row `row` and scroll them into view.
    pub fn select_match(&mut self, rows: &[SearchRow], row: usize, char_col: usize, len: usize) {
        let Some(r) = rows.iter().find(|r| r.row == row) else {
            return;
        };
        let (Some(&first), Some(&last)) = (
            r.columns.get(char_col),
            r.columns.get((char_col + len).saturating_sub(1)),
        ) else {
            return;
        };
        let history = self.history_size() as i32;
        let line = Line(row as i32 - history);
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(line, Column(first)),
            Side::Left,
        );
        sel.update(Point::new(line, Column(last)), Side::Right);
        self.term.selection = Some(sel);
        self.scroll_line_into_view(line);
    }

    fn scroll_line_into_view(&mut self, line: Line) {
        let offset = self.display_offset() as i32;
        let top = -offset;
        let bottom = top + self.lines as i32 - 1;
        if line.0 >= top && line.0 <= bottom {
            return;
        }
        let wanted = (self.lines as i32 / 2 - line.0).clamp(0, self.history_size() as i32);
        self.term.scroll_display(Scroll::Delta(wanted - offset));
    }
}

/// Shared handle type used when an engine is read from painting code.
pub type SharedColors = Arc<TerminalColors>;

#[cfg(test)]
mod tests {
    use super::*;
    use ops_core::theme::THEMES;

    fn engine(cols: usize, lines: usize) -> Engine {
        Engine::new(
            cols,
            lines,
            1000,
            TerminalColors::from_palette(&THEMES[0].terminal),
        )
    }

    #[test]
    fn plain_text_and_cursor() {
        let mut e = engine(20, 5);
        e.feed(b"hello\r\nworld");
        assert_eq!(e.screen_text(), "hello\nworld");
        assert_eq!(e.cursor(), (5, 1));
    }

    /// zsh marks a missing final newline with a reverse-video `%`, pads the line to the terminal width, returns to the
    /// start of the line and clears below before it draws the prompt. The text before the marker has to survive.
    #[test]
    fn text_without_a_final_newline_survives_the_zsh_end_of_line_marker() {
        for cols in [20usize, 40] {
            let mut e = engine(cols, 5);
            e.feed(b"$ printf noeol\r\nnoeol");
            let pad = " ".repeat(cols - 1);
            e.feed(format!("\x1b[1m\x1b[7m%\x1b[27m\x1b[1m\x1b[0m{pad}\r \r\r\x1b[0m\x1b[27m\x1b[24m\x1b[JMac-mini% ").as_bytes());
            let text = e.screen_text();
            assert!(text.contains("noeol%"), "cols={cols}: {text:?}");
            assert!(text.ends_with("Mac-mini%"), "cols={cols}: {text:?}");
        }
    }

    #[test]
    fn cjk_and_ansi_do_not_leak_escape_text() {
        let mut e = engine(20, 3);
        e.feed("\x1b[31m你好\x1b[0m abc".as_bytes());
        assert_eq!(e.screen_text(), "你好 abc");
    }

    #[test]
    fn device_queries_produce_replies() {
        let mut e = engine(20, 3);
        let out = e.feed(b"\x1b[c\x1b[6n");
        assert!(
            out.writes.iter().any(|w| w.starts_with("\x1b[?")),
            "primary DA answered: {:?}",
            out.writes
        );
        assert!(
            out.writes.iter().any(|w| w == "\x1b[1;1R"),
            "cursor position report: {:?}",
            out.writes
        );
    }

    #[test]
    fn osc_color_queries_use_the_theme_palette() {
        let mut e = engine(20, 3);
        let out = e.feed(b"\x1b]11;?\x1b\\");
        let bg = e.colors.background;
        let expected = format!(
            "\x1b]11;rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
            bg.r, bg.r, bg.g, bg.g, bg.b, bg.b
        );
        assert_eq!(out.writes, vec![expected]);
    }

    #[test]
    fn title_changes_are_reported() {
        let mut e = engine(20, 3);
        assert_eq!(
            e.feed(b"\x1b]0;root@web\x07").title,
            Some(Some("root@web".to_string()))
        );
    }

    #[test]
    fn resize_reflows_and_reports_change() {
        let mut e = engine(10, 3);
        assert!(e.resize(30, 6));
        assert!(!e.resize(30, 6));
        assert_eq!((e.cols(), e.lines()), (30, 6));
    }

    #[test]
    fn buffer_text_includes_scrollback() {
        let mut e = engine(20, 3);
        for i in 0..10 {
            e.feed(format!("line {i}\r\n").as_bytes());
        }
        let text = e.buffer_text();
        assert!(text.starts_with("line 0\n"), "{text}");
        assert!(text.contains("line 9"));
        assert!(e.history_size() > 0);
    }

    #[test]
    fn selection_text_roundtrip() {
        let mut e = engine(20, 3);
        e.feed(b"hello world");
        e.start_selection(SelectionType::Simple, 6, 0, Side::Left);
        e.update_selection(10, 0, Side::Right);
        assert_eq!(e.selection_text().as_deref(), Some("world"));
        e.clear_selection();
        assert!(e.selection_text().is_none());
    }

    #[test]
    fn search_rows_map_characters_to_columns_across_wide_chars() {
        let mut e = engine(20, 3);
        e.feed("你好 err".as_bytes());
        let rows = e.search_rows();
        let r = rows.iter().find(|r| r.text.contains("err")).unwrap();
        assert_eq!(r.text, "你好 err");
        // 你(0-1) 好(2-3) space(4) e(5)
        assert_eq!(r.columns[3], 5);
    }

    #[test]
    fn select_match_highlights_and_scrolls_to_old_lines() {
        let mut e = engine(30, 3);
        e.feed(b"needle in history\r\n");
        for i in 0..20 {
            e.feed(format!("filler {i}\r\n").as_bytes());
        }
        let rows = e.search_rows();
        let hit = rows.iter().find(|r| r.text.contains("needle")).unwrap();
        e.select_match(&rows, hit.row, 0, 6);
        assert_eq!(e.selection_text().as_deref(), Some("needle"));
        assert!(e.display_offset() > 0, "scrolled back to show the match");
    }

    #[test]
    fn indexed_colors_follow_xterm_cube() {
        let c = TerminalColors::from_palette(&THEMES[0].terminal);
        assert_eq!(c.indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            c.indexed(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(c.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(c.indexed(1), c.ansi[1]);
    }
}
