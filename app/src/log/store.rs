//! Log entries of the whole app, newest first (port of `stores/log.ts`).

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use ops_core::log::{get_log_history, AppLogEntry, GLOBAL_LOG_TOPIC};
use ops_i18n::t;

use crate::sftp::store::blocking;

pub const MAX_LOG_ENTRIES: usize = 600;
const HISTORY_LIMIT: u32 = 200;

pub struct LogStore {
    /// Newest first.
    pub logs: Vec<AppLogEntry>,
    pub loading: bool,
    started: bool,
    _tasks: Vec<Task<()>>,
}

pub struct LogHandle(pub Entity<LogStore>);
impl Global for LogHandle {}

pub fn log_store(cx: &App) -> Entity<LogStore> {
    cx.global::<LogHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let store = cx.new(|cx| LogStore::new(cx));
    cx.set_global(LogHandle(store));
}

impl LogStore {
    fn new(cx: &mut Context<Self>) -> Self {
        // Subscribe immediately so nothing logged before the window opens is lost.
        let events = crate::services::ctx(cx).events.subscribe(GLOBAL_LOG_TOPIC);
        let task = cx.spawn(async move |this, cx| {
            while let Ok(ev) = events.recv_async().await {
                let Some(entry) = ev.parse::<AppLogEntry>() else { continue };
                if this.update(cx, |this, cx| this.push(entry, cx)).is_err() {
                    break;
                }
            }
        });
        Self { logs: Vec::new(), loading: true, started: false, _tasks: vec![task] }
    }

    fn push(&mut self, entry: AppLogEntry, cx: &mut Context<Self>) {
        self.logs.insert(0, entry);
        self.logs.truncate(MAX_LOG_ENTRIES);
        cx.notify();
    }

    /// Load the stored history once and announce the window (like `init()` of the Tauri store).
    pub fn ensure_started(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let core = ctx.core.clone();
            let history = blocking(move || get_log_history(&core, Some(HISTORY_LIMIT))).await;
            let _ = this.update(cx, |this, cx| {
                // History is chronological; live entries that arrived meanwhile stay on top.
                let live = std::mem::take(&mut this.logs);
                this.logs = live.into_iter().chain(history.unwrap_or_default().into_iter().rev()).take(MAX_LOG_ENTRIES).collect();
                this.loading = false;
                cx.notify();
            });
            ops_core::log::emit_log(&ctx.core, "info", "system", &t!("log.connectedMessage"), "frontend");
        })
        .detach();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self.logs.clear();
        cx.notify();
        cx.spawn(async move |_, _| {
            let core = ctx.core.clone();
            let _ = blocking(move || ops_core::log::clear_log_history(&core)).await;
        })
        .detach();
    }
}
