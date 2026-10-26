//! The hand-painted terminal grid (cells, selection, cursor, IME text, scroll thumb) for a `TerminalPane`.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui_kit::*;
use crate::scale::px;

use super::pane::TerminalPane;

/// First family of a CSS font stack that is installed (generic names like `monospace` are skipped).
pub fn pick_font_family(stack: &str, installed: &[String], fallback: &str) -> String {
    for raw in stack.split(',') {
        let name = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if name.is_empty() || matches!(name.to_lowercase().as_str(), "monospace" | "sans-serif" | "serif" | "system-ui") {
            continue;
        }
        if let Some(found) = installed.iter().find(|f| f.eq_ignore_ascii_case(name)) {
            return found.clone();
        }
    }
    fallback.to_string()
}

pub fn platform_fallback_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

pub fn hsla_of(c: Rgb) -> Hsla {
    rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32).into()
}

pub const LINE_HEIGHT_FACTOR: f32 = 1.35;

/// Geometry of the last paint, used to map mouse positions and IME bounds back to cells.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub bounds: Bounds<Pixels>,
    pub cell_w: Pixels,
    pub line_h: Pixels,
}

impl Layout {
    /// Window position -> (column, row, in-left-half-of-cell), clamped to the grid.
    pub fn cell_at(&self, position: Point<Pixels>, cols: usize, lines: usize) -> (usize, usize, bool) {
        let x = (position.x - self.bounds.origin.x).max(px(0.));
        let y = (position.y - self.bounds.origin.y).max(px(0.));
        let col = ((x / self.cell_w).floor() as usize).min(cols.saturating_sub(1));
        let row = ((y / self.line_h).floor() as usize).min(lines.saturating_sub(1));
        let frac = (x - self.cell_w * col as f32) / self.cell_w;
        (col, row, frac < 0.5)
    }
}

pub struct TerminalElement {
    pub pane: Entity<TerminalPane>,
}

impl TerminalElement {
    pub fn new(pane: Entity<TerminalPane>) -> Self {
        Self { pane }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

struct Seg {
    col: usize,
    width_cols: usize,
    text: String,
    run: TextRun,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Layout;

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
        let style = Style { size: size(relative(1.).into(), relative(1.).into()), ..Default::default() };
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
        let (family, font_size) = {
            let p = self.pane.read(cx);
            (p.font_family().clone(), p.font_size())
        };
        let ts = window.text_system();
        let f = font(family);
        let font_id = ts.resolve_font(&f);
        let measured = ts.advance(font_id, px(font_size), 'm').map(|s| s.width).unwrap_or(px(0.));
        // A text system that cannot measure (headless tests) must not produce a 1px grid.
        let cell_w = if measured >= px(font_size * 0.3) { measured } else { px(font_size * 0.6) };
        let line_h = px((font_size * LINE_HEIGHT_FACTOR).round());
        let cols = (bounds.size.width / cell_w).floor().max(1.) as usize;
        let lines = (bounds.size.height / line_h).floor().max(1.) as usize;
        let layout = Layout { bounds, cell_w, line_h };
        self.pane.update(cx, |p, cx| p.set_viewport(cols, lines, layout, cx));
        layout
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
        let focus = self.pane.read(cx).focus_handle().clone();
        let focused = focus.is_focused(window);
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.pane.clone()), cx);

        let pane = self.pane.read(cx);
        let engine = pane.engine();
        let colors = engine.colors;
        let fs = px(pane.font_size());
        let base_font = font(pane.font_family().clone());
        let term = engine.term();
        let content = term.renderable_content();
        let overrides = content.colors;
        let display_offset = content.display_offset as i32;
        let cursor = content.cursor;
        let selection = content.selection;
        let n_lines = engine.lines();
        let n_cols = engine.cols();
        let cursor_visible = focused && pane.cursor_phase_on();
        let marked = pane.marked_text().map(str::to_string);
        let history = engine.history_size();

        let mut rows: Vec<Vec<Seg>> = (0..n_lines).map(|_| Vec::new()).collect();
        let mut bg_quads: Vec<(usize, usize, usize, Hsla)> = Vec::new();

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
            let mut fg = colors.resolve(cell.fg, overrides, true);
            let mut bg = colors.resolve(cell.bg, overrides, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let ncols = if wide { 2 } else { 1 };
            let selected = selection.is_some_and(|s| s.contains(ic.point));

            let default_bg = matches!(cell.bg, Color::Named(NamedColor::Background)) && !cell.flags.contains(Flags::INVERSE);
            if selected || !default_bg {
                let c = if selected { hsla_of(colors.selection) } else { hsla_of(bg) };
                match bg_quads.last_mut() {
                    Some((r, c0, n, last)) if *r == row && *c0 + *n == col && *last == c => *n += ncols,
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
            let underline = cell.flags.intersects(Flags::ALL_UNDERLINES).then(|| UnderlineStyle {
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
                    .then(|| StrikethroughStyle { thickness: px(1.), color: Some(fg_h) }),
            };

            // Merge adjacent narrow cells with identical style into one shaped segment; wide chars stay alone so
            // columns stay aligned for CJK.
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
            segs.push(Seg { col, width_cols: ncols, text: cell.c.to_string(), run });
        }

        let cell_origin = |row: usize, col: usize| point(bounds.origin.x + m.cell_w * col as f32, bounds.origin.y + m.line_h * row as f32);

        window.paint_quad(fill(bounds, hsla_of(colors.background)));
        for (row, col, n, color) in bg_quads {
            window.paint_quad(fill(Bounds::new(cell_origin(row, col), size(m.cell_w * n as f32, m.line_h)), color));
        }

        // Cursor (under the text; the glyph on it flips to the background colour for contrast).
        let cursor_row = cursor.point.line.0 + display_offset;
        let cursor_cell = (cursor_row, cursor.point.column.0);
        let draw_cursor = cursor.shape != CursorShape::Hidden && cursor_row >= 0 && (cursor_row as usize) < n_lines;
        if draw_cursor {
            let origin = cell_origin(cursor_row as usize, cursor.point.column.0);
            let cursor_color = hsla_of(colors.cursor);
            let full = Bounds::new(origin, size(m.cell_w, m.line_h));
            if !focused {
                window.paint_quad(outline(full, cursor_color, BorderStyle::Solid));
            } else if cursor_visible {
                match cursor.shape {
                    CursorShape::Underline => window.paint_quad(fill(Bounds::new(point(origin.x, origin.y + m.line_h - px(2.)), size(m.cell_w, px(2.))), cursor_color)),
                    CursorShape::Beam => window.paint_quad(fill(Bounds::new(origin, size(px(2.), m.line_h)), cursor_color)),
                    CursorShape::HollowBlock => window.paint_quad(outline(full, cursor_color, BorderStyle::Solid)),
                    _ => window.paint_quad(fill(full, cursor_color)),
                }
            }
        }
        let block_cursor_on = draw_cursor && focused && cursor_visible && matches!(cursor.shape, CursorShape::Block);

        for (row, segs) in rows.into_iter().enumerate() {
            for mut seg in segs {
                if block_cursor_on && (row as i32, seg.col) == cursor_cell {
                    seg.run.color = hsla_of(colors.background);
                }
                let shaped = window.text_system().shape_line(seg.text.into(), fs, &[seg.run], Some(m.cell_w));
                let _ = shaped.paint(cell_origin(row, seg.col), m.line_h, TextAlign::Left, None, window, cx);
            }
        }

        // IME composition text, drawn over the cursor cell.
        if let Some(marked) = marked.filter(|s| !s.is_empty()) {
            if draw_cursor {
                let origin = cell_origin(cursor_row as usize, cursor.point.column.0);
                let width_cols: usize = marked.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
                window.paint_quad(fill(Bounds::new(origin, size(m.cell_w * width_cols as f32, m.line_h)), hsla_of(colors.background)));
                let run = TextRun {
                    len: marked.len(),
                    font: base_font.clone(),
                    color: hsla_of(colors.foreground),
                    background_color: None,
                    underline: Some(UnderlineStyle { thickness: px(1.), color: Some(hsla_of(colors.foreground)), wavy: false }),
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(marked.into(), fs, &[run], None);
                let _ = shaped.paint(origin, m.line_h, TextAlign::Left, None, window, cx);
            }
        }

        // Scroll position hint on the right edge while looking at history.
        if history > 0 && display_offset > 0 {
            let total = (history + n_lines) as f32;
            let thumb_h = (bounds.size.height * (n_lines as f32 / total)).max(px(16.));
            let travel = bounds.size.height - thumb_h;
            let top = travel * (1.0 - display_offset as f32 / history as f32);
            let mut c = hsla_of(colors.foreground);
            c.a = 0.35;
            window.paint_quad(fill(
                Bounds::new(point(bounds.origin.x + bounds.size.width - px(5.), bounds.origin.y + top), size(px(4.), thumb_h)),
                c,
            ));
        }

    }
}

#[cfg(test)]
mod tests {
    use super::pick_font_family;

    #[test]
    fn font_stack_picks_first_installed_family() {
        let installed = vec!["Menlo".to_string(), "JetBrains Mono".to_string()];
        let stack = "\"Cascadia Code\", Consolas, \"JetBrains Mono\", Menlo, monospace";
        assert_eq!(pick_font_family(stack, &installed, "Fallback"), "JetBrains Mono");
        assert_eq!(pick_font_family("'menlo'", &installed, "Fallback"), "Menlo", "case-insensitive, quotes stripped");
        assert_eq!(pick_font_family("monospace, serif", &installed, "Fallback"), "Fallback");
        assert_eq!(pick_font_family("", &installed, "Fallback"), "Fallback");
    }
}
