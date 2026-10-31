//! Command / script / quick-action state (port of `stores/library.ts`). Every write reloads the affected list.

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use ops_library::{library as lib, quick_actions as qa};

use super::model::{CommandEntry, QuickActionEntry, ScriptEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryEvent {
    Loaded,
}

#[derive(Default)]
pub struct LibraryStore {
    pub commands: Vec<CommandEntry>,
    pub scripts: Vec<ScriptEntry>,
    pub actions: Vec<QuickActionEntry>,
}

impl EventEmitter<LibraryEvent> for LibraryStore {}

pub struct LibraryHandle(pub Entity<LibraryStore>);
impl Global for LibraryHandle {}

pub fn library_store(cx: &App) -> Entity<LibraryStore> {
    cx.global::<LibraryHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let store = cx.new(|_| LibraryStore::default());
    cx.set_global(LibraryHandle(store));
}

type Db = std::sync::Arc<ops_ssh::Ctx>;

impl LibraryStore {
    pub fn load_commands(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let rows = ops_core::rt::run(async move { lib::list_commands(&ctx.db).await })
                .await
                .and_then(|r| r)
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.commands = rows.iter().map(CommandEntry::from_row).collect();
                cx.emit(LibraryEvent::Loaded);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn load_scripts(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let rows = ops_core::rt::run(async move { lib::list_scripts(&ctx.db).await })
                .await
                .and_then(|r| r)
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.scripts = rows.iter().map(ScriptEntry::from_row).collect();
                cx.emit(LibraryEvent::Loaded);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn load_actions(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let rows = ops_core::rt::run(async move { qa::list_quick_actions(&ctx.db).await })
                .await
                .and_then(|r| r)
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.actions = rows.iter().map(QuickActionEntry::from_row).collect();
                cx.emit(LibraryEvent::Loaded);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn load_all(&mut self, cx: &mut Context<Self>) {
        self.load_commands(cx);
        self.load_scripts(cx);
        self.load_actions(cx);
    }

    /// Run a write on the runtime, then reload with `reload`.
    fn write<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        reload: fn(&mut Self, &mut Context<Self>),
        op: impl FnOnce(
            Db,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, String>> + Send>,
        >,
    ) -> Task<Result<T, String>> {
        let ctx = crate::services::ctx(cx);
        let fut = op(ctx);
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(fut).await.and_then(|r| r);
            if result.is_ok() {
                let _ = this.update(cx, |this, cx| reload(this, cx));
            }
            result
        })
    }

    // ---- commands ----

    pub fn add_command(
        &mut self,
        entry: CommandEntry,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        let new = entry.to_new();
        self.write(cx, Self::load_commands, move |ctx| {
            Box::pin(async move { lib::add_command(&ctx.db, new).await })
        })
    }

    pub fn delete_command(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.write(cx, Self::load_commands, move |ctx| {
            Box::pin(async move { lib::delete_command(&ctx.db, id).await })
        })
    }

    pub fn toggle_star_command(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.write(cx, Self::load_commands, move |ctx| {
            Box::pin(async move { lib::toggle_star_command(&ctx.db, id).await })
        })
    }

    // ---- scripts ----

    pub fn add_script(
        &mut self,
        entry: ScriptEntry,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        let new = entry.to_new();
        self.write(cx, Self::load_scripts, move |ctx| {
            Box::pin(async move { lib::add_script(&ctx.db, new).await })
        })
    }

    pub fn update_script(
        &mut self,
        entry: ScriptEntry,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let row = entry.to_row();
        self.write(cx, Self::load_scripts, move |ctx| {
            Box::pin(async move { lib::update_script(&ctx.db, row).await })
        })
    }

    pub fn delete_script(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.write(cx, Self::load_scripts, move |ctx| {
            Box::pin(async move { lib::delete_script(&ctx.db, id).await })
        })
    }

    pub fn toggle_star_script(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.write(cx, Self::load_scripts, move |ctx| {
            Box::pin(async move { lib::toggle_star_script(&ctx.db, id).await })
        })
    }

    pub fn save_script_version(
        &mut self,
        script_id: String,
        content: String,
        label: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |_, _| {
            ops_core::rt::run(async move {
                lib::save_script_version(&ctx.db, script_id, content, label).await
            })
            .await
            .and_then(|r| r)
        })
    }

    pub fn list_script_versions(
        &self,
        script_id: String,
        cx: &mut Context<Self>,
    ) -> Task<Vec<lib::ScriptVersion>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |_, _| {
            ops_core::rt::run(async move { lib::list_script_versions(&ctx.db, script_id).await })
                .await
                .and_then(|r| r)
                .unwrap_or_default()
        })
    }

    // ---- quick actions ----

    pub fn add_action(
        &mut self,
        entry: QuickActionEntry,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        let new = entry.to_new();
        self.write(cx, Self::load_actions, move |ctx| {
            Box::pin(async move { qa::add_quick_action(&ctx.db, new).await })
        })
    }

    pub fn update_action(
        &mut self,
        entry: QuickActionEntry,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let row = entry.to_row();
        self.write(cx, Self::load_actions, move |ctx| {
            Box::pin(async move { qa::update_quick_action(&ctx.db, row).await })
        })
    }

    pub fn delete_action(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.write(cx, Self::load_actions, move |ctx| {
            Box::pin(async move { qa::delete_quick_action(&ctx.db, id).await })
        })
    }
}
