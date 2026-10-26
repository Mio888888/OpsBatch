//! Broadcast terminal window. Port of `pages/Commands/BatchTerminalWindow.tsx` + `TerminalPane.tsx`.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_ssh::hosts::Host;
use ops_ssh::Ctx;

use super::{effective_columns, rows, MAX_CONCURRENT_CONNECTS, MAX_HOSTS};
use crate::terminal::pane::{PaneEvent, TerminalPane};
use crate::terminal::workspace::ConnState;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::window::{self, WindowKind};

pub struct PaneSession {
    pub host: Host,
    pub state: ConnState,
    pub error: Option<String>,
    pub pane: Option<Entity<TerminalPane>>,
    session_id: Option<String>,
    attempt: u64,
    _sub: Option<Subscription>,
}

pub struct BatchTerminalWindow {
    ctx: Arc<Ctx>,
    host_ids: Vec<String>,
    loading: bool,
    sessions: Vec<PaneSession>,
    semaphore: Arc<tokio::sync::Semaphore>,
    columns: usize,
    columns_select: Entity<ChoiceSelect>,
    next_attempt: u64,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl BatchTerminalWindow {
    pub fn open(host_ids: Vec<String>, cx: &mut App) {
        let ids: Vec<String> = host_ids.into_iter().take(MAX_HOSTS).collect();
        let options = window::window_options(WindowKind::BatchTerminal, &t!("assets.broadcastTerminalWindowTitle", count = ids.len()), cx);
        window::replace_window("batch-terminal", options, cx, move |window, cx| cx.new(|cx| Self::new(ids, window, cx)));
    }

    pub fn new(host_ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ctx = crate::services::ctx(cx);
        let choices = vec![
            Choice::new("0", t!("batchTerminal.auto")),
            Choice::new("1", t!("batchTerminal.columns", count = 1)),
            Choice::new("2", t!("batchTerminal.columns", count = 2)),
            Choice::new("3", t!("batchTerminal.columns", count = 3)),
            Choice::new("4", t!("batchTerminal.columns", count = 4)),
        ];
        let columns_select = cx.new(|cx| new_select(choices, "2", window, cx));
        let subs = vec![cx.subscribe_in(&columns_select, window, |this, _, ev, _, cx| {
            if let Some(v) = confirmed_value(ev) {
                this.columns = v.parse().unwrap_or(2);
                cx.notify();
            }
        })];

        let ids: Vec<String> = host_ids.into_iter().take(MAX_HOSTS).collect();
        let c = ctx.clone();
        let wanted = ids.clone();
        let load = cx.spawn(async move |this, cx| {
            let all = ops_core::rt::run(async move { ops_ssh::hosts::list_hosts(&c.db).await }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match all {
                    Ok(all) => {
                        // Keep the requested order.
                        let hosts: Vec<Host> = wanted.iter().filter_map(|id| all.iter().find(|h| &h.id == id).cloned()).collect();
                        this.sessions = hosts
                            .into_iter()
                            .map(|host| PaneSession { host, state: ConnState::Connecting, error: None, pane: None, session_id: None, attempt: 0, _sub: None })
                            .collect();
                        for ix in 0..this.sessions.len() {
                            this.connect(ix, cx);
                        }
                    }
                    Err(e) => log::warn!("broadcast terminal: could not load hosts: {e}"),
                }
                cx.notify();
            });
        });

        Self {
            ctx,
            host_ids: ids,
            loading: true,
            sessions: Vec::new(),
            semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTS)),
            columns: 2,
            columns_select,
            next_attempt: 1,
            _subs: subs,
            _tasks: vec![load],
        }
    }

    // ---- accessors (tests) ----

    pub fn sessions(&self) -> &[PaneSession] {
        &self.sessions
    }
    pub fn is_loading(&self) -> bool {
        self.loading
    }
    pub fn host_ids(&self) -> &[String] {
        &self.host_ids
    }
    pub fn stats(&self) -> (usize, usize, usize) {
        let ok = self.sessions.iter().filter(|s| s.state == ConnState::Connected).count();
        let err = self.sessions.iter().filter(|s| s.state == ConnState::Error).count();
        (ok, err, self.sessions.len() - ok - err)
    }

    // ---- connecting ----

    fn disconnect_session(&self, session_id: Option<String>, host_id: String) {
        if let Some(sid) = session_id {
            let ctx = self.ctx.clone();
            ops_core::rt::spawn(async move {
                let _ = ops_ssh::terminal::terminal_disconnect(&ctx.terminals_mgr, sid, Some(host_id)).await;
            });
        }
    }

    pub fn connect(&mut self, ix: usize, cx: &mut Context<Self>) {
        let attempt = self.next_attempt;
        self.next_attempt += 1;
        let (host_id, old) = {
            let s = &mut self.sessions[ix];
            s.attempt = attempt;
            s.state = ConnState::Connecting;
            s.error = None;
            s._sub = None;
            s.pane = None;
            (s.host.id.clone(), s.session_id.take())
        };
        self.disconnect_session(old, host_id.clone());
        cx.notify();
        let (ctx, sem) = (self.ctx.clone(), self.semaphore.clone());
        cx.spawn(async move |this, cx| {
            let c = ctx.clone();
            let hid = host_id.clone();
            let result = ops_core::rt::run(async move {
                let _permit = sem.acquire_owned().await.map_err(|e| e.to_string())?;
                ops_ssh::terminal::terminal_connect(c, hid, Some(80), Some(24), None).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| this.finish_connect(ix, attempt, result, cx));
        })
        .detach();
    }

    fn finish_connect(&mut self, ix: usize, attempt: u64, result: Result<String, String>, cx: &mut Context<Self>) {
        let stale = self.sessions.get(ix).map(|s| s.attempt != attempt).unwrap_or(true);
        if stale {
            if let Ok(sid) = result {
                let ctx = self.ctx.clone();
                ops_core::rt::spawn(async move {
                    let _ = ops_ssh::terminal::terminal_disconnect(&ctx.terminals_mgr, sid, None).await;
                });
            }
            return;
        }
        match result {
            Ok(sid) => {
                let ctx = self.ctx.clone();
                let pane = cx.new(|cx| {
                    let mut p = TerminalPane::new(ctx, sid, cx);
                    p.set_broadcast(true);
                    p
                });
                let sub = cx.subscribe(&pane, |this, _, ev: &PaneEvent, cx| {
                    if let PaneEvent::UserInput(data) = ev {
                        this.broadcast(data, cx);
                    }
                });
                let s = &mut self.sessions[ix];
                s.session_id = Some(pane.read(cx).session_id.clone());
                s.pane = Some(pane);
                s._sub = Some(sub);
                s.state = ConnState::Connected;
            }
            Err(e) => {
                let s = &mut self.sessions[ix];
                s.state = ConnState::Error;
                s.error = Some(e);
            }
        }
        cx.notify();
    }

    /// Send input to every connected session.
    pub fn broadcast(&mut self, data: &str, cx: &mut Context<Self>) {
        for s in self.sessions.iter().filter(|s| s.state == ConnState::Connected) {
            if let Some(p) = &s.pane {
                p.read(cx).write_raw(data.to_string());
            }
        }
    }

    fn pane_view(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let s = &self.sessions[ix];
        let (dot, status): (Hsla, Option<(String, Hsla)>) = match s.state {
            ConnState::Connected => (theme.success, None),
            ConnState::Connecting => (theme.primary, Some((t!("batchTerminal.paneConnecting"), theme.primary))),
            ConnState::Error => (theme.danger, Some((t!("batchTerminal.paneFailed"), theme.danger))),
        };
        let body: AnyElement = match (&s.state, &s.pane) {
            (ConnState::Error, _) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_3()
                .child(div().text_size(px(12.)).text_color(theme.danger).child(s.error.clone().filter(|e| !e.is_empty()).unwrap_or_else(|| t!("batchTerminal.connectFailed"))))
                .child(Button::new(("batch-retry", ix)).small().label(t!("terminal.retry")).on_click(cx.listener(move |this, _, _, cx| this.connect(ix, cx))))
                .into_any_element(),
            (_, Some(p)) => div().size_full().p(px(4.)).child(p.clone()).into_any_element(),
            _ => div().into_any_element(),
        };
        v_flex()
            .id(("batch-pane", ix))
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .h(px(26.))
                    .px_2()
                    .gap_1p5()
                    .items_center()
                    .flex_none()
                    .bg(theme.tab_bar)
                    .child(div().size(px(6.)).rounded_full().bg(dot))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(if s.host.name.is_empty() { s.host.ip.clone() } else { s.host.name.clone() }))
                    .when(!s.host.ip.is_empty(), |d| d.child(div().text_size(px(10.)).text_color(theme.muted_foreground).child(s.host.ip.clone())))
                    .when_some(status, |d, (text, color)| d.child(div().px_1p5().rounded_full().bg(Hsla { a: 0.12, ..color }).text_color(color).text_size(px(10.)).child(text))),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

impl Drop for BatchTerminalWindow {
    fn drop(&mut self) {
        // Closing the window hangs up every session it opened.
        for s in &self.sessions {
            if let Some(sid) = s.session_id.clone() {
                let ctx = self.ctx.clone();
                let host = s.host.id.clone();
                ops_core::rt::spawn(async move {
                    let _ = ops_ssh::terminal::terminal_disconnect(&ctx.terminals_mgr, sid, Some(host)).await;
                });
            }
        }
    }
}

impl Render for BatchTerminalWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        if self.loading {
            return div()
                .id("batch-terminal-loading")
                .test_support()
                .size_full()
                .bg(theme.background)
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child(t!("batchTerminal.loading"))
                .into_any_element();
        }
        let (ok, err, pending) = self.stats();
        let total = self.sessions.len();
        let cols = effective_columns(self.columns, total);
        let ixs: Vec<usize> = (0..total).collect();
        let grid_rows = rows(&ixs, cols);
        let n_rows = grid_rows.len().max(1);
        let _ = n_rows;

        let chip = |text: String, color: Hsla| div().px_1p5().rounded_full().bg(Hsla { a: 0.12, ..color }).text_color(color).text_size(px(11.)).child(text);

        let header = h_flex()
            .id("batch-terminal-header")
            .test_support()
            .w_full()
            .pr_2()
            .gap_2()
            .items_center()
            .child(div().font_weight(FontWeight::BOLD).child(t!("batchTerminal.title")))
            .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("batchTerminal.count", count = total)))
            .when(ok > 0, |d| d.child(chip(t!("batchTerminal.connected", count = ok), theme.success)))
            .when(err > 0, |d| d.child(chip(t!("batchTerminal.failed", count = err), theme.danger)))
            .when(pending > 0, |d| d.child(chip(t!("batchTerminal.connecting", count = pending), theme.primary)))
            .child(div().flex_1())
            .child(div().w(px(110.)).child(Select::new(&self.columns_select).small()))
            .child(Button::new("batch-terminal-close").small().label(t!("batchTerminal.close")).on_click(|_, window, _| window.remove_window()));

        let grid = v_flex().id("batch-terminal-grid").test_support().flex_1().min_h_0().children(grid_rows.into_iter().enumerate().map(|(r, row)| {
            let missing = cols - row.len();
            h_flex()
                .id(("batch-row", r))
                .flex_1()
                .min_h_0()
                .children(row.into_iter().map(|ix| self.pane_view(ix, cx)))
                .children((0..missing).map(|_| div().flex_1()))
        }));

        v_flex()
            .id("batch-terminal-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(header))
            .child(grid)
            .into_any_element()
    }
}
