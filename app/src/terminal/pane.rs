//! One live terminal session: emulator + PTY/SSH channel + input handling (keyboard, IME, mouse, paste, search,
//! danger guard, background commands, tracked command execution). Port of `components/TerminalView.tsx`.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::scale::px;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use futures::channel::oneshot;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_ssh::Ctx;
use ops_terminal::keys::{self, KeyModes, Mods, MouseAction, MouseButton as TermButton};
use ops_terminal::search::{find_terminal_search_matches, next_search_index, SearchLine};
use ops_terminal::tracker::TrackedCommandOutputCleaner;

use super::element::{pick_font_family, platform_fallback_font, Layout, TerminalElement};
use super::engine::{Engine, TerminalColors};
use super::executions::{
    ExecStatus, ExecutionResult, ExecutionTracker, Started, DEFAULT_COMMAND_TIMEOUT,
};
use super::guard::{GuardAction, InputGuard};

const CURSOR_BLINK: Duration = Duration::from_millis(530);
/// Output processed per UI update; the rest stays queued so input and painting stay responsive during floods.
const MAX_CHARS_PER_UPDATE: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum PaneEvent {
    /// Broadcast mode only: text the user typed or pasted, to be forwarded by the owner instead of written here.
    UserInput(String),
    TitleChanged(Option<String>),
    /// The session ended (remote closed the channel, shell exited).
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchResult {
    pub found: bool,
    pub match_count: usize,
    pub match_index: Option<usize>,
}

enum PtyCmd {
    Write(String),
    Resize(u16, u16),
}

pub struct TerminalPane {
    pub session_id: String,
    ctx: Arc<Ctx>,
    engine: Engine,
    guard: InputGuard,
    executions: ExecutionTracker,
    cleaner: TrackedCommandOutputCleaner,
    pty: flume::Sender<PtyCmd>,
    focus: FocusHandle,
    closed: bool,
    broadcast: bool,
    title: Option<String>,
    marked: Option<String>,
    layout: Option<Layout>,
    font_family: SharedString,
    font_size: f32,
    cursor_on: bool,
    last_activity: Instant,
    scroll_px: f32,
    selecting: bool,
    /// Cell of a plain left press that may turn out to be a click on a URL.
    click_cell: Option<(usize, usize)>,
    mouse_down: Option<TermButton>,
    last_mouse_cell: Option<(usize, usize)>,
    backend_size: (u16, u16),
    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaneEvent> for TerminalPane {}

fn term_mods(m: &Modifiers) -> Mods {
    Mods {
        shift: m.shift,
        alt: m.alt,
        ctrl: m.control,
    }
}

impl TerminalPane {
    /// Claims the session's output stream; the session must have been opened by `terminal_connect*`.
    pub fn new(ctx: Arc<Ctx>, session_id: String, cx: &mut Context<Self>) -> Self {
        let (colors, scrollback) = Self::theme_inputs(cx);
        let engine = Engine::new(80, 24, scrollback, colors);
        let (pty_tx, pty_rx) = flume::unbounded::<PtyCmd>();

        // Everything written to the session goes through one ordered queue (writes and resizes must not reorder).
        {
            let ctx = ctx.clone();
            let sid = session_id.clone();
            ops_core::rt::spawn(async move {
                while let Ok(cmd) = pty_rx.recv_async().await {
                    let result = match cmd {
                        PtyCmd::Write(data) => {
                            ops_ssh::terminal::terminal_write(
                                &ctx.terminals_mgr,
                                &ctx.registry,
                                sid.clone(),
                                data,
                            )
                            .await
                        }
                        PtyCmd::Resize(cols, rows) => {
                            ops_ssh::terminal::terminal_resize(
                                &ctx.terminals_mgr,
                                &ctx.registry,
                                sid.clone(),
                                cols,
                                rows,
                            )
                            .await
                        }
                    };
                    if let Err(error) = result {
                        log::warn!("terminal {sid}: {error}");
                        if error.contains("session not found") {
                            break;
                        }
                    }
                }
            });
        }

        let mut tasks = Vec::new();
        match ctx.terminals.take(&session_id) {
            Some(rx) => tasks.push(cx.spawn(async move |this, cx| {
                loop {
                    let Ok(first) = rx.recv_async().await else {
                        break;
                    };
                    let mut batch = vec![first];
                    let mut chars = batch[0].len();
                    while chars < MAX_CHARS_PER_UPDATE {
                        match rx.try_recv() {
                            Ok(more) => {
                                chars += more.len();
                                batch.push(more);
                            }
                            Err(_) => break,
                        }
                    }
                    if this
                        .update(cx, |this, cx| this.on_output(batch, cx))
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = this.update(cx, |this, cx| this.on_closed(cx));
            })),
            None => log::warn!("terminal {session_id}: output stream already claimed"),
        }

        // Cursor blink (only repaints while this pane has focus).
        tasks.push(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(CURSOR_BLINK).await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.blink_due() {
                        this.cursor_on = !this.cursor_on;
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        }));

        let subs = vec![cx.observe_global::<crate::theme::AppTheme>(|this, cx| {
            this.apply_appearance(cx);
            cx.notify();
        })];

        let mut pane = Self {
            session_id,
            ctx,
            engine,
            guard: InputGuard::new(),
            executions: ExecutionTracker::new(),
            cleaner: TrackedCommandOutputCleaner::new(),
            pty: pty_tx,
            focus: cx.focus_handle(),
            closed: false,
            broadcast: false,
            title: None,
            marked: None,
            layout: None,
            font_family: SharedString::from(platform_fallback_font()),
            font_size: 14.0,
            cursor_on: true,
            last_activity: Instant::now(),
            scroll_px: 0.0,
            selecting: false,
            click_cell: None,
            mouse_down: None,
            last_mouse_cell: None,
            backend_size: (80, 24),
            _tasks: tasks,
            _subs: subs,
        };
        pane.apply_appearance(cx);
        pane.load_danger_rules(cx);
        pane
    }

    fn theme_inputs(cx: &App) -> (TerminalColors, usize) {
        let t = crate::theme::current(cx);
        (
            TerminalColors::from_palette(&t.resolved.theme.terminal),
            t.settings.terminal_scrollback as usize,
        )
    }

    fn apply_appearance(&mut self, cx: &mut Context<Self>) {
        let (colors, scrollback) = Self::theme_inputs(cx);
        self.engine.colors = colors;
        self.engine.set_scrollback(scrollback);
        let (stack, size) = {
            let s = &crate::theme::current(cx).settings;
            (s.terminal_font_family.clone(), s.terminal_font_size)
        };
        let installed = cx.text_system().all_font_names();
        self.font_family = pick_font_family(&stack, &installed, platform_fallback_font()).into();
        self.font_size = size.clamp(8, 48) as f32;
    }

    fn load_danger_rules(&mut self, cx: &mut Context<Self>) {
        let ctx = self.ctx.clone();
        cx.spawn(async move |this, cx| {
            let rules =
                ops_core::rt::run(
                    async move { ops_core::settings::list_danger_rules(&ctx.db).await },
                )
                .await
                .and_then(|r| r);
            match rules {
                Ok(rules) => {
                    let compiled = ops_terminal::danger::compile_danger_rules(&rules);
                    let _ = this.update(cx, |this, _| this.guard.set_rules(compiled));
                }
                Err(error) => log::warn!("could not load danger rules: {error}"),
            }
        })
        .detach();
    }

    // ---- accessors ----

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    pub fn font_family(&self) -> &SharedString {
        &self.font_family
    }
    pub fn font_size(&self) -> f32 {
        self.font_size
    }
    pub fn marked_text(&self) -> Option<&str> {
        self.marked.as_deref()
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    pub fn cursor_phase_on(&self) -> bool {
        self.cursor_on || self.last_activity.elapsed() < CURSOR_BLINK
    }
    pub fn cols(&self) -> usize {
        self.engine.cols()
    }
    pub fn lines(&self) -> usize {
        self.engine.lines()
    }
    pub fn screen_text(&self) -> String {
        self.engine.screen_text()
    }
    /// Whole buffer as text (scrollback + screen); the AI panel uses it as context.
    pub fn buffer_text(&self) -> String {
        self.engine.buffer_text()
    }
    pub fn selection_text(&self) -> Option<String> {
        self.engine.selection_text()
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn blink_due(&self) -> bool {
        self.last_activity.elapsed() >= CURSOR_BLINK
    }

    // ---- layout / backend size ----

    pub fn set_viewport(
        &mut self,
        cols: usize,
        lines: usize,
        layout: Layout,
        _cx: &mut Context<Self>,
    ) {
        self.layout = Some(layout);
        self.engine.set_cell_px(
            f32::from(layout.cell_w).round() as u16,
            f32::from(layout.line_h).round() as u16,
        );
        if self.engine.resize(cols, lines)
            || self.backend_size != (self.engine.cols() as u16, self.engine.lines() as u16)
        {
            self.backend_size = (self.engine.cols() as u16, self.engine.lines() as u16);
            let _ = self
                .pty
                .send(PtyCmd::Resize(self.backend_size.0, self.backend_size.1));
        }
    }

    // ---- output ----

    fn on_output(&mut self, batch: Vec<String>, cx: &mut Context<Self>) {
        let mut raw = String::new();
        let mut closed = false;
        for msg in batch {
            if msg.is_empty() {
                closed = true;
                break;
            }
            raw.push_str(&msg);
        }
        if !raw.is_empty() {
            let cleaned = self.cleaner.clean(&raw);
            if let Some(started) = self.executions.on_output(&raw, &cleaned) {
                self.begin_execution(started, cx);
            }
            if !cleaned.is_empty() {
                let out = self.engine.feed(cleaned.as_bytes());
                for reply in out.writes {
                    self.write(reply);
                }
                if let Some(title) = out.title {
                    self.title = title.clone();
                    cx.emit(PaneEvent::TitleChanged(title));
                }
                if let Some(text) = out.clipboard_store {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
        if closed {
            self.on_closed(cx);
        }
        cx.notify();
    }

    fn on_closed(&mut self, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.executions.fail_all();
        let msg = format!("\r\n\x1b[31m{}\x1b[0m\r\n", t!("terminalRuntime.closed"));
        self.engine.feed(msg.as_bytes());
        cx.emit(PaneEvent::Closed);
        cx.notify();
    }

    /// Show text in the terminal without sending it to the shell.
    fn print_local(&mut self, text: &str) {
        self.engine.feed(text.as_bytes());
    }

    // ---- input ----

    /// Broadcast mode: user input is reported through [`PaneEvent::UserInput`] instead of being written to this session.
    pub fn set_broadcast(&mut self, on: bool) {
        self.broadcast = on;
    }

    /// Write bytes straight to the session (no danger guard, no local echo handling).
    pub fn write_raw(&self, data: impl Into<String>) {
        self.write(data);
    }

    fn user_write(&mut self, data: String, cx: &mut Context<Self>) {
        if self.broadcast {
            cx.emit(PaneEvent::UserInput(data));
        } else {
            self.write(data);
        }
    }

    fn write(&self, data: impl Into<String>) {
        let data = data.into();
        if !data.is_empty() {
            let _ = self.pty.send(PtyCmd::Write(data));
        }
    }

    fn touch(&mut self) {
        self.cursor_on = true;
        self.last_activity = Instant::now();
    }

    /// User-typed text (keyboard or IME commit): goes through the background/danger guard.
    pub fn send_user_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.closed || text.is_empty() {
            return;
        }
        self.touch();
        self.engine.scroll(Scroll::Bottom);
        self.engine.clear_selection();
        let action = {
            let engine = &self.engine;
            self.guard.process(text, || engine.cursor_line_text())
        };
        match action {
            GuardAction::Forward => self.user_write(text.to_string(), cx),
            GuardAction::Background(command) => self.run_background(command, cx),
            GuardAction::Blocked(rules) => {
                let msg = format!(
                    "\r\n\x1b[33m{}\x1b[0m\r\n\x1b[33m{}\x1b[0m\r\n",
                    t!("terminalRuntime.dangerBlocked", rules = rules.join(", ")),
                    t!("terminalRuntime.dangerConfirm")
                );
                self.print_local(&msg);
            }
        }
        cx.notify();
    }

    fn run_background(&mut self, command: String, cx: &mut Context<Self>) {
        self.write("\x15");
        self.print_local(&format!(
            "\r\n\x1b[36m{}\x1b[0m\r\n",
            t!("terminalRuntime.backgroundStart", command = command)
        ));
        let ctx = self.ctx.clone();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                ops_ssh::terminal::terminal_spawn_background_process(ctx, command, None, None).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                let msg = match result {
                    Ok(info) => format!(
                        "\x1b[32m{}\x1b[0m\r\n{}\r\n",
                        t!("terminalRuntime.backgroundStarted", pid = info.pid),
                        t!("terminalRuntime.backgroundLog", path = info.log_file)
                    ),
                    Err(error) => format!(
                        "\x1b[31m{}\x1b[0m\r\n",
                        t!("terminalRuntime.backgroundFailed", error = error)
                    ),
                };
                this.print_local(&msg);
                cx.notify();
            });
        })
        .detach();
    }

    /// Paste text as the terminal would for ⌘V (bracketed when the program asked for it).
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.closed || text.is_empty() {
            return;
        }
        self.touch();
        self.engine.scroll(Scroll::Bottom);
        let data = if self.engine.mode().contains(TermMode::BRACKETED_PASTE) {
            String::from_utf8_lossy(&keys::bracketed_paste(text)).into_owned()
        } else {
            String::from_utf8_lossy(&keys::plain_paste(text)).into_owned()
        };
        self.user_write(data, cx);
        self.guard.reset_line();
        cx.notify();
    }

    /// Insert a command at the prompt without running it (command/script panels). Always bracketed.
    pub fn insert_command(&mut self, command: &str, cx: &mut Context<Self>) {
        let body = command.trim_end();
        if self.closed || body.is_empty() {
            return;
        }
        self.touch();
        self.write(format!("\x1b[200~{body}\x1b[201~"));
        self.guard.reset_line();
        cx.notify();
    }

    /// Run a command in this terminal and resolve with its exit code and output once it finishes.
    pub fn execute_command(
        &mut self,
        command: &str,
        timeout: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<ExecutionResult> {
        if self.closed {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(ExecutionResult {
                execution_id: String::new(),
                status: ExecStatus::Closed,
                exit_code: None,
                output: String::new(),
            });
            return rx;
        }
        let (rx, started) = self.executions.submit(
            command.to_string(),
            timeout.unwrap_or(DEFAULT_COMMAND_TIMEOUT),
        );
        if let Some(started) = started {
            self.begin_execution(started, cx);
        }
        rx
    }

    fn begin_execution(&mut self, started: Started, cx: &mut Context<Self>) {
        self.guard.reset_line();
        self.write(started.write);
        let id = started.execution_id;
        let timeout = started.timeout;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(next) = this.executions.timeout(&id) {
                    this.begin_execution(next, cx);
                }
            });
        })
        .detach();
    }

    // ---- search ----

    pub fn search(
        &mut self,
        query: &str,
        backwards: bool,
        from_index: Option<usize>,
    ) -> SearchResult {
        if query.trim().is_empty() {
            self.engine.clear_selection();
            return SearchResult::default();
        }
        let rows = self.engine.search_rows();
        let lines: Vec<SearchLine<'_>> = rows
            .iter()
            .map(|r| SearchLine {
                row: r.row,
                text: &r.text,
            })
            .collect();
        let matches = find_terminal_search_matches(&lines, query);
        let Some(index) = next_search_index(matches.len(), backwards, from_index) else {
            self.engine.clear_selection();
            return SearchResult::default();
        };
        let m = matches[index];
        self.engine.select_match(&rows, m.row, m.column, m.length);
        SearchResult {
            found: true,
            match_count: matches.len(),
            match_index: Some(index),
        }
    }

    pub fn clear_search(&mut self) {
        self.engine.clear_selection();
    }

    // ---- keyboard ----

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        match self.engine.selection_text() {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            None => false,
        }
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let key = ks.key.as_str();

        // Clipboard shortcuts: ⌘C/⌘V/⌘A on macOS, Ctrl+Shift+C/V/A elsewhere.
        let clip = if cfg!(target_os = "macos") {
            m.platform && !m.control && !m.alt
        } else {
            m.control && m.shift && !m.alt
        };
        if clip {
            match key {
                "c" => {
                    self.copy_selection(cx);
                    cx.stop_propagation();
                }
                "v" => {
                    self.paste_clipboard(cx);
                    cx.stop_propagation();
                }
                "a" => {
                    self.engine.select_all();
                    cx.notify();
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        if m.platform {
            return; // other ⌘ shortcuts belong to the app
        }
        if m.control && !m.shift && !m.alt && key == "f" && !cfg!(target_os = "macos") {
            return; // Ctrl+F opens the terminal search (handled by the page), it is not sent to the shell
        }

        // Plain text (including Option-composed characters on macOS) arrives through the input handler so IME works.
        let single = key.chars().count() == 1 || key == "space";
        let alt_as_meta = !cfg!(target_os = "macos");
        if single && !m.control && (!m.alt || !alt_as_meta) {
            return;
        }

        let modes = KeyModes {
            app_cursor: self.engine.mode().contains(TermMode::APP_CURSOR),
            option_composes_text: false,
        };
        if let Some(bytes) = keys::encode_key(key, ks.key_char.as_deref(), term_mods(m), modes) {
            if let Ok(text) = String::from_utf8(bytes) {
                self.send_user_text(&text, cx);
                cx.stop_propagation();
            }
        }
    }

    // ---- mouse ----

    fn report_mouse(
        &self,
        button: TermButton,
        action: MouseAction,
        mods: Mods,
        cell: (usize, usize),
    ) {
        if !self.engine.mode().contains(TermMode::SGR_MOUSE) {
            return;
        }
        if let Some(bytes) = keys::encode_mouse_sgr(button, action, mods, cell.0, cell.1) {
            self.write(String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    /// Cell size in pixels once the pane has been laid out (tests, hit-testing).
    pub fn cell_metrics(&self) -> Option<(Pixels, Pixels)> {
        self.layout.map(|l| (l.cell_w, l.line_h))
    }

    /// URL printed under a viewport cell, if any.
    pub fn url_at(&self, col: usize, row: usize) -> Option<String> {
        let (text, columns) = self.engine.viewport_row(row);
        super::links::url_at(&text, &columns, col)
    }

    fn mouse_mode(&self, shift: bool) -> bool {
        !shift && self.engine.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn cell(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        match &self.layout {
            Some(l) => l.cell_at(position, self.engine.cols(), self.engine.lines()),
            None => (0, 0, true),
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (col, row, left) = self.cell(ev.position);
        if ev.button == gpui_kit::MouseButton::Right {
            return; // the context menu wrapper around the pane handles it
        }
        let button = match ev.button {
            gpui_kit::MouseButton::Left => TermButton::Left,
            gpui_kit::MouseButton::Middle => TermButton::Middle,
            _ => return,
        };
        if self.mouse_mode(ev.modifiers.shift) {
            self.mouse_down = Some(button);
            self.last_mouse_cell = Some((col, row));
            self.report_mouse(
                button,
                MouseAction::Press,
                term_mods(&ev.modifiers),
                (col, row),
            );
            return;
        }
        if button == TermButton::Middle {
            self.paste_clipboard(cx);
            return;
        }
        let kind = match ev.click_count {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        self.click_cell = (ev.click_count <= 1).then_some((col, row));
        self.engine
            .start_selection(kind, col, row, if left { Side::Left } else { Side::Right });
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (col, row, left) = self.cell(ev.position);
        if let Some(button) = self.mouse_down {
            let mode = self.engine.mode();
            if (mode.contains(TermMode::MOUSE_DRAG) || mode.contains(TermMode::MOUSE_MOTION))
                && self.last_mouse_cell != Some((col, row))
            {
                self.last_mouse_cell = Some((col, row));
                self.report_mouse(
                    button,
                    MouseAction::Motion,
                    term_mods(&ev.modifiers),
                    (col, row),
                );
            }
            return;
        }
        if self.selecting && ev.pressed_button == Some(gpui_kit::MouseButton::Left) {
            // Dragging past the top/bottom edge scrolls the history.
            if let Some(l) = &self.layout {
                if ev.position.y < l.bounds.origin.y {
                    self.engine.scroll(Scroll::Delta(1));
                } else if ev.position.y > l.bounds.origin.y + l.bounds.size.height {
                    self.engine.scroll(Scroll::Delta(-1));
                }
            }
            self.engine
                .update_selection(col, row, if left { Side::Left } else { Side::Right });
            cx.notify();
        } else if self.engine.mode().contains(TermMode::MOUSE_MOTION)
            && !ev.modifiers.shift
            && self.last_mouse_cell != Some((col, row))
        {
            self.last_mouse_cell = Some((col, row));
            self.report_mouse(
                TermButton::None,
                MouseAction::Motion,
                term_mods(&ev.modifiers),
                (col, row),
            );
        }
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        let (col, row, _) = self.cell(ev.position);
        if let Some(button) = self.mouse_down.take() {
            self.report_mouse(
                button,
                MouseAction::Release,
                term_mods(&ev.modifiers),
                (col, row),
            );
        }
        if self.selecting {
            self.selecting = false;
            if !self.engine.has_selection() {
                self.engine.clear_selection();
                // a click (press and release on the same cell, nothing selected) on a URL opens it
                if self.click_cell == Some((col, row)) {
                    if let Some(url) = self.url_at(col, row) {
                        cx.open_url(&url);
                    }
                }
            }
            cx.notify();
        }
        self.click_cell = None;
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line_h = self
            .layout
            .map(|l| f32::from(l.line_h))
            .unwrap_or(self.font_size * 1.35);
        self.scroll_px += f32::from(ev.delta.pixel_delta(px(line_h)).y);
        let lines = (self.scroll_px / line_h).trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_px -= lines as f32 * line_h;
        let mode = self.engine.mode();
        if self.mouse_mode(ev.modifiers.shift) {
            let (col, row, _) = self.cell(ev.position);
            let button = if lines > 0 {
                TermButton::WheelUp
            } else {
                TermButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.report_mouse(
                    button,
                    MouseAction::Press,
                    term_mods(&ev.modifiers),
                    (col, row),
                );
            }
        } else if mode.contains(TermMode::ALT_SCREEN) {
            if mode.contains(TermMode::ALTERNATE_SCROLL) {
                let up = lines > 0;
                let seq = match (up, mode.contains(TermMode::APP_CURSOR)) {
                    (true, true) => "\x1bOA",
                    (true, false) => "\x1b[A",
                    (false, true) => "\x1bOB",
                    (false, false) => "\x1b[B",
                };
                for _ in 0..lines.unsigned_abs().min(10) {
                    self.write(seq);
                }
            }
        } else {
            self.engine.scroll(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

pub fn pane_element_id(session_id: &str) -> ElementId {
    ElementId::Name(format!("terminal-pane-{session_id}").into())
}

impl Render for TerminalPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(pane_element_id(&self.session_id))
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| this.on_key_down(ev, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev, w, cx| this.on_mouse_down(ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, ev, w, cx| this.on_mouse_down(ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev, w, cx| this.on_mouse_down(ev, w, cx)),
            )
            .on_mouse_move(cx.listener(|this, ev, _w, cx| this.on_mouse_move(ev, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev, _w, cx| this.on_mouse_up(ev, cx)),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, ev, _w, cx| this.on_mouse_up(ev, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, ev, _w, cx| this.on_mouse_up(ev, cx)),
            )
            .on_scroll_wheel(cx.listener(|this, ev, _w, cx| this.on_scroll(ev, cx)))
            .child(TerminalElement::new(cx.entity()))
    }
}

/// IME / text input: committed text is typed into the shell; composition is previewed at the cursor.
impl EntityInputHandler for TerminalPane {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _w: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        self.send_user_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _sel: Option<Range<usize>>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let l = self.layout?;
        let (col, row) = self.engine.cursor();
        Some(Bounds::new(
            point(
                element_bounds.origin.x + l.cell_w * col as f32,
                element_bounds.origin.y + l.line_h * row as f32,
            ),
            size(l.cell_w, l.line_h),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
