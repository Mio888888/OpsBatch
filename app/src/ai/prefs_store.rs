//! Shared state of the AI settings tabs: the whole `AiPrefs` document, loaded once and saved after every change.

use gpui_kit::*;
use ops_ai::prefs::{self, AiPrefs, RuntimeLimits};

#[derive(Debug, Clone)]
pub enum PrefsEvent {
    Loaded,
    Saved,
    SaveFailed(String),
}

struct PrefsHandle(Entity<PrefsStore>);
impl Global for PrefsHandle {}

/// The one preferences document of the process: the settings window, the MCP bridge and the approval window all edit
/// this entity, so a stale copy can never overwrite another one's change.
pub fn shared(cx: &mut App) -> Entity<PrefsStore> {
    if let Some(h) = cx.try_global::<PrefsHandle>() {
        return h.0.clone();
    }
    let store = cx.new(PrefsStore::new);
    cx.set_global(PrefsHandle(store.clone()));
    store
}

pub struct PrefsStore {
    prefs: AiPrefs,
    loaded: bool,
    saving: bool,
    dirty: bool,
}

impl EventEmitter<PrefsEvent> for PrefsStore {}

impl PrefsStore {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self { prefs: AiPrefs::default(), loaded: false, saving: false, dirty: false };
        // the keys live in the vault: reading while it is locked would show blanks and saving would erase the keys
        if crate::vault::is_unlocked(cx) {
            this.load(cx);
        } else {
            let weak = cx.entity().downgrade();
            crate::vault::on_unlocked(cx, move |cx| {
                let _ = weak.update(cx, |this, cx| this.load(cx));
            });
        }
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { prefs::load_prefs(&ctx.db) }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(prefs) => this.prefs = prefs,
                    Err(e) => cx.emit(PrefsEvent::SaveFailed(e)),
                }
                this.loaded = true;
                crate::ai::runtime::set(cx, RuntimeLimits::from(this.prefs.clone()));
                cx.emit(PrefsEvent::Loaded);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn prefs(&self) -> &AiPrefs {
        &self.prefs
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// A save is running or queued.
    pub fn saving(&self) -> bool {
        self.saving || self.dirty
    }

    /// Applies `f`, normalizes, tells the chat runtime and saves in the background (saves never overlap; the latest
    /// state always wins).
    pub fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AiPrefs)) {
        if !self.loaded {
            return;
        }
        f(&mut self.prefs);
        self.prefs.normalize();
        crate::ai::runtime::set(cx, RuntimeLimits::from(self.prefs.clone()));
        cx.notify();
        self.persist(cx);
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            self.dirty = true;
            return;
        }
        self.saving = true;
        let snapshot = self.prefs.clone();
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { prefs::save_prefs(&ctx.db, &snapshot) }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => cx.emit(PrefsEvent::Saved),
                    Err(e) => cx.emit(PrefsEvent::SaveFailed(e)),
                }
                if std::mem::take(&mut this.dirty) {
                    this.persist(cx);
                }
            });
        })
        .detach();
    }
}
