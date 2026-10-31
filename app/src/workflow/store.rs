//! Workflows, templates and scheduled tasks as UI state (port of `stores/workflow.ts` + the page-level loaders).

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use ops_workflow::model::{
    parse_nodes_payload, stringify_nodes_payload, CanvasNode, Connection, WorkflowRecord,
    WorkflowStatus,
};
use ops_workflow::store as db;
use ops_workflow::store::{ScheduledTask, WorkflowTemplate};

use crate::sftp::store::blocking;

#[derive(Default)]
pub struct WorkflowStore {
    pub workflows: Vec<WorkflowRecord>,
    pub templates: Vec<WorkflowTemplate>,
    pub tasks: Vec<ScheduledTask>,
    pub loading: bool,
}

pub struct WorkflowHandle(pub Entity<WorkflowStore>);
impl Global for WorkflowHandle {}

pub fn workflow_store(cx: &App) -> Entity<WorkflowStore> {
    cx.global::<WorkflowHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let store = cx.new(|_| WorkflowStore::default());
    cx.set_global(WorkflowHandle(store));
}

impl WorkflowStore {
    pub fn workflow(&self, id: &str) -> Option<&WorkflowRecord> {
        self.workflows.iter().find(|w| w.id == id)
    }

    pub fn reload_all(&mut self, cx: &mut Context<Self>) {
        self.reload_workflows(cx);
        self.reload_templates(cx);
        self.reload_tasks(cx);
    }

    pub fn reload_workflows(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let list = blocking(move || db::list_workflows(&ctx.db)).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.workflows = list.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn reload_templates(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let list = blocking(move || db::list_templates(&ctx.db)).await;
            let _ = this.update(cx, |this, cx| {
                this.templates = list.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn reload_tasks(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let list = blocking(move || db::list_scheduled_tasks(&ctx.db)).await;
            let _ = this.update(cx, |this, cx| {
                this.tasks = list.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn create_workflow(
        &mut self,
        name: String,
        description: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<WorkflowRecord, String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let wf = blocking(move || db::create_workflow(&ctx.db, &name, &description)).await?;
            let _ = this.update(cx, |this, cx| this.reload_workflows(cx));
            Ok(wf)
        })
    }

    pub fn save_workflow(
        &mut self,
        wf: WorkflowRecord,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            blocking(move || db::update_workflow(&ctx.db, &wf)).await?;
            let _ = this.update(cx, |this, cx| this.reload_workflows(cx));
            Ok(())
        })
    }

    pub fn delete_workflow(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            blocking(move || db::delete_workflow(&ctx.db, &id)).await?;
            let _ = this.update(cx, |this, cx| this.reload_workflows(cx));
            Ok(())
        })
    }

    pub fn save_as_template(
        &mut self,
        wf: &WorkflowRecord,
        name_suffix: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        let name = format!("{} {}", wf.name, name_suffix);
        let description = wf.description.clone();
        let nodes = stringify_nodes_payload(&wf.nodes, wf.settings);
        let connections = serde_json::to_string(&wf.connections).unwrap_or_else(|_| "[]".into());
        cx.spawn(async move |this, cx| {
            blocking(move || {
                db::save_template(&ctx.db, &name, &description, &nodes, &connections).map(|_| ())
            })
            .await?;
            let _ = this.update(cx, |this, cx| this.reload_templates(cx));
            Ok(())
        })
    }

    /// A new draft workflow filled from `template`. The returned record is shown as ready, like the original editor.
    pub fn create_from_template(
        &mut self,
        template: WorkflowTemplate,
        cx: &mut Context<Self>,
    ) -> Task<Result<WorkflowRecord, String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let payload = parse_nodes_payload(&template.nodes)?;
            let connections: Vec<Connection> =
                serde_json::from_str(if template.connections.is_empty() {
                    "[]"
                } else {
                    &template.connections
                })
                .map_err(|e| e.to_string())?;
            let record = blocking(move || {
                let mut wf = db::create_workflow(&ctx.db, &template.name, &template.description)?;
                wf.nodes = payload.nodes;
                wf.connections = connections;
                wf.settings = payload.settings;
                wf.status = WorkflowStatus::Draft;
                db::update_workflow(&ctx.db, &wf)?;
                wf.status = WorkflowStatus::Ready;
                Ok(wf)
            })
            .await?;
            let _ = this.update(cx, |this, cx| this.reload_workflows(cx));
            Ok(record)
        })
    }

    pub fn delete_template(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            blocking(move || db::delete_template(&ctx.db, &id)).await?;
            let _ = this.update(cx, |this, cx| this.reload_templates(cx));
            Ok(())
        })
    }

    pub fn add_task(
        &mut self,
        name: String,
        rule: String,
        workflow_id: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            blocking(move || {
                db::add_scheduled_task(&ctx.db, &name, &rule, &workflow_id).map(|_| ())
            })
            .await?;
            let _ = this.update(cx, |this, cx| this.reload_tasks(cx));
            Ok(())
        })
    }

    pub fn set_task_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<(), String>>> {
        let t = self.tasks.iter().find(|t| t.id == id)?.clone();
        let ctx = crate::services::ctx(cx);
        Some(cx.spawn(async move |this, cx| {
            blocking(move || {
                db::update_scheduled_task(&ctx.db, &t.id, &t.name, &t.cron, &t.workflow_id, enabled)
            })
            .await?;
            let _ = this.update(cx, |this, cx| this.reload_tasks(cx));
            Ok(())
        }))
    }

    pub fn delete_task(&mut self, id: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            blocking(move || db::delete_scheduled_task(&ctx.db, &id)).await?;
            let _ = this.update(cx, |this, cx| this.reload_tasks(cx));
            Ok(())
        })
    }
}

/// Convenience for the editor: a copy of the record with the edited graph.
pub fn with_graph(
    base: &WorkflowRecord,
    nodes: Vec<CanvasNode>,
    connections: Vec<Connection>,
) -> WorkflowRecord {
    WorkflowRecord {
        nodes,
        connections,
        ..base.clone()
    }
}
