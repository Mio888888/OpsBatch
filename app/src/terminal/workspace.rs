//! Terminal tabs and their connections (port of the state machine in `pages/Terminal/TerminalPage.tsx`).
//!
//! The workspace is app-level state: sessions keep running when the main window is closed to the tray and the
//! rebuilt window shows the same tabs.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Subscription, Task};
use ops_i18n::t;
use ops_ssh::hosts::Host;
use ops_ssh::Ctx;
use ops_terminal::tabs::{active_key_after_close, terminal_tab_close_targets, TabCloseMode};

use super::pane::{PaneEvent, TerminalPane};
use crate::host_form::submitted_secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Connected,
    Error,
}

/// The second pane of a split tab.
pub struct SplitPane {
    pub state: ConnState,
    pub pane: Option<Entity<TerminalPane>>,
    pub error: Option<String>,
    attempt: Option<String>,
    _sub: Option<Subscription>,
}

pub struct TerminalTab {
    pub key: String,
    pub kind: TabKind,
    pub host_id: Option<String>,
    pub host_name: String,
    pub host_ip: String,
    pub state: ConnState,
    pub error: Option<String>,
    pub pane: Option<Entity<TerminalPane>>,
    pub split: Option<SplitPane>,
    attempt: Option<String>,
    _sub: Option<Subscription>,
}

impl TerminalTab {
    pub fn session_id(&self, cx: &App) -> Option<String> {
        self.pane.as_ref().map(|p| p.read(cx).session_id.clone())
    }
}

#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    /// Tabs, states or the active tab changed.
    Changed,
    /// The active tab changed or its terminal just came up: the page should give it keyboard focus.
    Activated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRetryValues {
    pub key_auth: bool,
    pub username: String,
    pub password: String,
    pub private_key: String,
}

pub struct TerminalWorkspace {
    ctx: Arc<Ctx>,
    tabs: Vec<TerminalTab>,
    active: Option<String>,
    /// The first visit to the terminal page opens a local shell (once per app run).
    auto_local_pending: bool,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for TerminalWorkspace {}

/// App-level handle so every window shows the same workspace.
pub struct WorkspaceHandle(pub Entity<TerminalWorkspace>);
impl Global for WorkspaceHandle {}

pub fn workspace(cx: &App) -> Entity<TerminalWorkspace> {
    cx.global::<WorkspaceHandle>().0.clone()
}

pub fn init(ctx: Arc<Ctx>, auto_local: bool, cx: &mut App) {
    let ws = cx.new(|cx| TerminalWorkspace::new(ctx, auto_local, cx));
    cx.set_global(WorkspaceHandle(ws));
}

fn new_key(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

impl TerminalWorkspace {
    pub fn new(ctx: Arc<Ctx>, auto_local: bool, cx: &mut Context<Self>) -> Self {
        let events = ctx.events.subscribe("connection_status_changed");
        let task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                let host = event
                    .payload
                    .get("hostId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let status = event
                    .payload
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if this
                    .update(cx, |this, cx| this.on_connection_status(&host, &status, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            ctx,
            tabs: Vec::new(),
            active: None,
            auto_local_pending: auto_local,
            _tasks: vec![task],
        }
    }

    pub fn set_auto_local(&mut self, enabled: bool) {
        self.auto_local_pending = enabled;
    }

    // ---- reads ----

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn tab(&self, key: &str) -> Option<&TerminalTab> {
        self.tabs.iter().find(|t| t.key == key)
    }

    pub fn active_key(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active_tab(&self) -> Option<&TerminalTab> {
        self.active.as_deref().and_then(|k| self.tab(k))
    }

    fn tab_mut(&mut self, key: &str) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|t| t.key == key)
    }

    pub fn set_active(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.tab(key).is_some() && self.active.as_deref() != Some(key) {
            self.active = Some(key.to_string());
            cx.emit(WorkspaceEvent::Activated);
            cx.emit(WorkspaceEvent::Changed);
            cx.notify();
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::Changed);
        cx.notify();
    }

    // ---- opening ----

    /// Open the terminal page for the first time: a local shell when nothing else was requested.
    pub fn ensure_initial_tab(&mut self, cx: &mut Context<Self>) {
        if self.auto_local_pending && self.tabs.is_empty() {
            self.auto_local_pending = false;
            self.open_local(cx);
        }
    }

    /// Open (or focus) a terminal for a host. Returns false when it was already open.
    pub fn open_host(
        &mut self,
        host_id: &str,
        name: &str,
        ip: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        self.auto_local_pending = false;
        if let Some(existing) = self.tabs.iter().find(|t| {
            t.host_id.as_deref() == Some(host_id)
                && matches!(t.state, ConnState::Connecting | ConnState::Connected)
        }) {
            let key = existing.key.clone();
            self.set_active(&key, cx);
            return false;
        }
        let key = new_key(&format!("remote-{host_id}"));
        let name = if name.is_empty() {
            t!("terminal.defaultName")
        } else {
            name.to_string()
        };
        self.tabs.push(TerminalTab {
            key: key.clone(),
            kind: TabKind::Remote,
            host_id: Some(host_id.to_string()),
            host_name: name,
            host_ip: ip.to_string(),
            state: ConnState::Connecting,
            error: None,
            pane: None,
            split: None,
            attempt: None,
            _sub: None,
        });
        self.start_connect(&key, cx);
        true
    }

    pub fn open_local(&mut self, cx: &mut Context<Self>) -> String {
        self.auto_local_pending = false;
        let key = new_key("local");
        self.tabs.push(TerminalTab {
            key: key.clone(),
            kind: TabKind::Local,
            host_id: None,
            host_name: t!("terminal.localTerminal"),
            host_ip: "127.0.0.1".into(),
            state: ConnState::Connecting,
            error: None,
            pane: None,
            split: None,
            attempt: None,
            _sub: None,
        });
        self.start_connect(&key, cx);
        key
    }

    pub fn retry(&mut self, key: &str, cx: &mut Context<Self>) {
        self.start_connect(key, cx);
    }

    // ---- connecting ----

    fn disconnect_session(&self, session_id: String, host_id: Option<String>) {
        let ctx = self.ctx.clone();
        ops_core::rt::spawn(async move {
            let _ = ops_ssh::terminal::terminal_disconnect(&ctx.terminals_mgr, session_id, host_id)
                .await;
        });
    }

    async fn connect_session(ctx: Arc<Ctx>, host_id: Option<String>) -> Result<String, String> {
        ops_core::rt::run(async move {
            match host_id {
                Some(host_id) => {
                    ops_ssh::terminal::terminal_connect(ctx, host_id, Some(80), Some(24), None)
                        .await
                }
                None => {
                    let mgr = ctx.terminals_mgr.clone();
                    tokio::task::spawn_blocking(move || {
                        ops_ssh::terminal::terminal_connect_local(ctx, &mgr, Some(80), Some(24))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                }
            }
        })
        .await
        .and_then(|r| r)
    }

    fn start_connect(&mut self, key: &str, cx: &mut Context<Self>) {
        let attempt = new_key("attempt");
        let Some(tab) = self.tab_mut(key) else { return };
        let old = tab.pane.take().map(|p| p.read(cx).session_id.clone());
        let host_id = tab.host_id.clone();
        tab._sub = None;
        tab.attempt = Some(attempt.clone());
        tab.state = ConnState::Connecting;
        tab.error = None;
        if let Some(sid) = old {
            self.disconnect_session(sid, host_id.clone());
        }
        self.active = Some(key.to_string());
        self.changed(cx);

        let ctx = self.ctx.clone();
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let result = Self::connect_session(ctx, host_id).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_connect(&key, &attempt, result, cx)
            });
        })
        .detach();
    }

    fn make_pane(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) -> (Entity<TerminalPane>, Subscription) {
        let ctx = self.ctx.clone();
        let pane = cx.new(|cx| TerminalPane::new(ctx, session_id, cx));
        let sub = cx.subscribe(&pane, |_, _, _: &PaneEvent, cx| cx.notify());
        (pane, sub)
    }

    fn finish_connect(
        &mut self,
        key: &str,
        attempt: &str,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .tab(key)
            .is_some_and(|t| t.attempt.as_deref() == Some(attempt));
        if !current {
            // The tab was closed or retried meanwhile: do not leak the session that just came up.
            if let Ok(sid) = result {
                self.disconnect_session(sid, None);
            }
            return;
        }
        match result {
            Ok(session_id) => {
                let (pane, sub) = self.make_pane(session_id, cx);
                let tab = self.tab_mut(key).expect("checked above");
                tab.attempt = None;
                tab.pane = Some(pane);
                tab._sub = Some(sub);
                tab.state = ConnState::Connected;
                tab.error = None;
                if let Some(host_id) = tab.host_id.clone() {
                    // Best effort: warm the SFTP channel once the terminal is interactive.
                    let ctx = self.ctx.clone();
                    ops_core::rt::spawn(async move {
                        if let Err(error) = ops_ssh::sftp::sftp_warmup(ctx, host_id).await {
                            log::warn!("sftp warmup failed: {error}");
                        }
                    });
                }
            }
            Err(error) => {
                let tab = self.tab_mut(key).expect("checked above");
                tab.attempt = None;
                tab.state = ConnState::Error;
                tab.error = Some(error);
            }
        }
        self.active = Some(key.to_string());
        cx.emit(WorkspaceEvent::Activated);
        self.changed(cx);
    }

    // ---- split ----

    pub fn open_split(&mut self, key: &str, cx: &mut Context<Self>) {
        let attempt = new_key("split");
        let host_id = {
            let Some(tab) = self.tab_mut(key) else { return };
            if tab.state != ConnState::Connected || tab.split.is_some() {
                return;
            }
            tab.split = Some(SplitPane {
                state: ConnState::Connecting,
                pane: None,
                error: None,
                attempt: Some(attempt.clone()),
                _sub: None,
            });
            tab.host_id.clone()
        };
        self.changed(cx);
        let ctx = self.ctx.clone();
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let result = Self::connect_session(ctx, host_id).await;
            let _ = this.update(cx, |this, cx| this.finish_split(&key, &attempt, result, cx));
        })
        .detach();
    }

    fn finish_split(
        &mut self,
        key: &str,
        attempt: &str,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .tab(key)
            .and_then(|t| t.split.as_ref())
            .is_some_and(|s| s.attempt.as_deref() == Some(attempt));
        if !current {
            if let Ok(sid) = result {
                self.disconnect_session(sid, None);
            }
            return;
        }
        match result {
            Ok(sid) => {
                let (pane, sub) = self.make_pane(sid, cx);
                if let Some(split) = self.tab_mut(key).and_then(|t| t.split.as_mut()) {
                    split.attempt = None;
                    split.state = ConnState::Connected;
                    split.pane = Some(pane);
                    split._sub = Some(sub);
                }
            }
            Err(error) => {
                if let Some(split) = self.tab_mut(key).and_then(|t| t.split.as_mut()) {
                    split.attempt = None;
                    split.state = ConnState::Error;
                    split.error = Some(error);
                }
            }
        }
        self.changed(cx);
    }

    pub fn close_split(&mut self, key: &str, cx: &mut Context<Self>) {
        let (sid, host) = {
            let Some(tab) = self.tab_mut(key) else { return };
            let Some(split) = tab.split.take() else {
                return;
            };
            (
                split.pane.as_ref().map(|p| p.read(cx).session_id.clone()),
                tab.host_id.clone(),
            )
        };
        if let Some(sid) = sid {
            self.disconnect_session(sid, host);
        }
        self.changed(cx);
    }

    // ---- closing ----

    pub fn close_tabs(
        &mut self,
        targets: &[String],
        preferred_active: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }
        let keys: Vec<String> = self.tabs.iter().map(|t| t.key.clone()).collect();
        let next_active =
            active_key_after_close(&keys, targets, self.active.as_deref(), preferred_active);
        let mut closing = Vec::new();
        self.tabs.retain_mut(|t| {
            if targets.contains(&t.key) {
                closing.push((
                    t.pane.take(),
                    t.split.take().and_then(|s| s.pane),
                    t.host_id.clone(),
                ));
                false
            } else {
                true
            }
        });
        let active_changed = self.active != next_active;
        self.active = next_active;
        if active_changed {
            cx.emit(WorkspaceEvent::Activated);
        }
        for (pane, split, host) in closing {
            if let Some(p) = pane {
                self.disconnect_session(p.read(cx).session_id.clone(), host.clone());
            }
            if let Some(p) = split {
                self.disconnect_session(p.read(cx).session_id.clone(), host.clone());
            }
        }
        self.changed(cx);
    }

    pub fn close_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        self.close_tabs(&[key.to_string()], None, cx);
    }

    pub fn close_by_mode(&mut self, target: &str, mode: TabCloseMode, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.tabs.iter().map(|t| t.key.clone()).collect();
        let targets = terminal_tab_close_targets(&keys, target, mode);
        let preferred = (mode != TabCloseMode::All).then_some(target);
        self.close_tabs(&targets, preferred, cx);
    }

    /// Disconnect every session (application quit).
    pub fn close_all_sessions(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.tabs.iter().map(|t| t.key.clone()).collect();
        self.close_tabs(&keys, None, cx);
    }

    // ---- connection health ----

    fn on_connection_status(&mut self, host_id: &str, status: &str, cx: &mut Context<Self>) {
        if !matches!(status, "idle_disconnected" | "link_down") {
            return;
        }
        let message = if status == "idle_disconnected" {
            t!("terminal.idleDisconnected")
        } else {
            t!("terminal.linkDown")
        };
        let mut changed = false;
        for tab in self
            .tabs
            .iter_mut()
            .filter(|t| t.host_id.as_deref() == Some(host_id) && t.state == ConnState::Connected)
        {
            tab.state = ConnState::Error;
            tab.error = Some(message.clone());
            changed = true;
        }
        if changed {
            self.changed(cx);
        }
    }

    // ---- auth retry ----

    /// The stored host behind a tab, for the "edit authentication" dialog.
    pub fn find_host(
        &self,
        host_id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<Host>, String>> {
        let ctx = self.ctx.clone();
        cx.spawn(async move |_, _| {
            ops_core::rt::run(async move {
                ops_ssh::hosts::list_hosts(&ctx.db)
                    .await
                    .map(|h| h.into_iter().find(|h| h.id == host_id))
            })
            .await
            .and_then(|r| r)
        })
    }

    /// Save new SSH credentials for the tab's host (blank secret = keep stored) and reconnect the tab.
    pub fn save_auth_and_retry(
        &mut self,
        key: &str,
        values: AuthRetryValues,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = self.ctx.clone();
        let key = key.to_string();
        let host_id = self.tab(&key).and_then(|t| t.host_id.clone());
        cx.spawn(async move |this, cx| {
            let Some(host_id) = host_id else {
                return Err(t!("terminal.authRetryHostMissing"));
            };
            let c2 = ctx.clone();
            let updated = ops_core::rt::run(async move {
                let host = ops_ssh::hosts::list_hosts(&c2.db)
                    .await?
                    .into_iter()
                    .find(|h| h.id == host_id)
                    .ok_or_else(|| t!("terminal.authRetryHostMissing"))?;
                if host.os != "linux" {
                    return Err(t!("terminal.authRetryUnsupportedHost"));
                }
                let mut update =
                    crate::stores::assets::update_from_host(&host, host.group_id.clone());
                update.auth_type = if values.key_auth {
                    "key".into()
                } else {
                    "password".into()
                };
                update.username = values.username.trim().to_string();
                update.password = if values.key_auth {
                    None
                } else {
                    submitted_secret(&values.password)
                };
                update.private_key = if values.key_auth {
                    submitted_secret(&values.private_key)
                } else {
                    None
                };
                ops_ssh::hosts::update_host(c2.core.clone(), c2.registry.clone(), update).await
            })
            .await
            .and_then(|r| r)?;
            let _ = this.update(cx, |this, cx| {
                if let Some(tab) = this.tab_mut(&key) {
                    tab.host_name = updated.name.clone();
                    tab.host_ip = updated.ip.clone();
                }
                this.start_connect(&key, cx);
            });
            Ok(())
        })
    }
}
