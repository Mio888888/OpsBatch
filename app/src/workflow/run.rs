//! One workflow execution as UI state: log, per-node status, the pending manual confirmation and the stop flag.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_workflow::executor::{create_log_entry, execute_workflow, LogEntry, LogLabels, LogStatus, ProgressEvent};
use ops_workflow::model::{collect_workflow_host_ids, WorkflowRecord};

use super::runtime::{AppRuntime, ConfirmRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Running,
    Success,
    Fail,
}

#[derive(Debug, Clone)]
pub enum RunEvent {
    Finished,
}

struct PendingConfirm {
    name: String,
    description: String,
    reply: Option<futures::channel::oneshot::Sender<bool>>,
}

pub struct WorkflowRun {
    pub workflow_id: Option<String>,
    pub workflow_name: String,
    pub logs: Vec<LogEntry>,
    pub statuses: HashMap<String, NodeStatus>,
    pub running: bool,
    pub collapsed: bool,
    pending: Option<PendingConfirm>,
    cancelled: Arc<AtomicBool>,
    tasks: Vec<Task<()>>,
}

impl EventEmitter<RunEvent> for WorkflowRun {}

fn log_labels() -> LogLabels {
    LogLabels {
        start_execute: t!("wfEditor.startExecute"),
        execute_success: t!("wfEditor.executeSuccess"),
        execute_fail: t!("wfEditor.executeFail"),
        stage_info: Box::new(|level, total, count| t!("wfEditor.stageInfo", level = level, total = total, count = count)),
    }
}

impl Default for WorkflowRun {
    fn default() -> Self {
        Self { workflow_id: None, workflow_name: String::new(), logs: Vec::new(), statuses: HashMap::new(), running: false, collapsed: false, pending: None, cancelled: Arc::new(AtomicBool::new(false)), tasks: Vec::new() }
    }
}

impl WorkflowRun {
    pub fn pending_confirm(&self) -> Option<(&str, &str)> {
        self.pending.as_ref().map(|p| (p.name.as_str(), p.description.as_str()))
    }

    /// Run `workflow` (the caller has checked it is runnable). Resets the log and node statuses.
    pub fn start(&mut self, workflow: &WorkflowRecord, window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.workflow_id = Some(workflow.id.clone());
        self.workflow_name = workflow.name.clone();
        self.logs.clear();
        self.statuses.clear();
        self.collapsed = false;
        self.running = true;
        self.cancelled = Arc::new(AtomicBool::new(false));
        cx.notify();

        let (nodes, connections) = (workflow.nodes.clone(), workflow.connections.clone());
        let host_ids = collect_workflow_host_ids(&nodes);
        let interval = workflow.settings.default_node_interval_seconds;
        let (confirm_tx, confirm_rx) = flume::unbounded::<ConfirmRequest>();
        let runtime = AppRuntime::new(crate::services::ctx(cx), cx.background_executor().clone(), confirm_tx, self.cancelled.clone());

        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(req) = confirm_rx.recv_async().await {
                if this.update_in(cx, |this, window, cx| this.request_confirm(req, window, cx)).is_err() {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            let labels = log_labels();
            execute_workflow(
                &nodes,
                &connections,
                &host_ids,
                &runtime,
                &mut |event| {
                    let _ = this.update(cx, |this, cx| this.on_event(event, &labels, cx));
                },
                interval,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                this.pending = None;
                cx.emit(RunEvent::Finished);
                cx.notify();
            });
        }));
    }

    fn on_event(&mut self, event: ProgressEvent, labels: &LogLabels, cx: &mut Context<Self>) {
        match &event {
            ProgressEvent::NodeStart { node_id, .. } => {
                self.statuses.insert(node_id.clone(), NodeStatus::Running);
            }
            ProgressEvent::NodeComplete { node_id, success, .. } => {
                self.statuses.insert(node_id.clone(), if *success { NodeStatus::Success } else { NodeStatus::Fail });
            }
            _ => {}
        }
        if let Some(entry) = create_log_entry(&event, labels, chrono::Utc::now().timestamp_millis()) {
            self.logs.push(entry);
        }
        cx.notify();
    }

    /// The user pressed "stop": ends the run before the next node (and answers an open confirmation with "no").
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.answer_confirm(false, cx);
    }

    fn request_confirm(&mut self, req: ConfirmRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = Some(PendingConfirm { name: req.node_name.clone(), description: req.description.clone(), reply: Some(req.reply) });
        cx.notify();
        let run = cx.entity();
        let (name, description) = (req.node_name, req.description);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(t!("wfEditor.manualConfirm"))
                .w(px(440.))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(cx_confirm_body(run.clone(), name.clone(), description.clone()))
        });
    }

    /// Resolve the open confirmation (continue = true, terminate = false).
    pub fn answer_confirm(&mut self, ok: bool, cx: &mut Context<Self>) {
        if let Some(mut pending) = self.pending.take() {
            if let Some(reply) = pending.reply.take() {
                let _ = reply.send(ok);
            }
            cx.notify();
        }
    }
}

fn cx_confirm_body(run: Entity<WorkflowRun>, name: String, description: String) -> impl IntoElement {
    ConfirmBody { run, name, description }
}

#[derive(IntoElement)]
struct ConfirmBody {
    run: Entity<WorkflowRun>,
    name: String,
    description: String,
}

impl RenderOnce for ConfirmBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (ok_run, cancel_run) = (self.run.clone(), self.run);
        v_flex()
            .id("wf-confirm")
            .test_support()
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(16.)).child(self.name))
            .when(!self.description.is_empty(), |d| d.child(div().text_color(theme.muted_foreground).child(self.description)))
            .child(div().text_size(px(12.)).child(t!("wfEditor.confirmContinueHint")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("wf-confirm-cancel").danger().label(t!("wfEditor.terminateWorkflow")).on_click(move |_, window, cx| {
                        cancel_run.update(cx, |r, cx| r.answer_confirm(false, cx));
                        window.close_dialog(cx);
                    }))
                    .child(Button::new("wf-confirm-ok").primary().label(t!("wfEditor.confirmContinue")).on_click(move |_, window, cx| {
                        ok_run.update(cx, |r, cx| r.answer_confirm(true, cx));
                        window.close_dialog(cx);
                    })),
            )
    }
}

/// The execution log (collapsible), as used in the run dialog and under the editor.
pub fn log_panel(run: &WorkflowRun, on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let color = |s: Option<LogStatus>| match s {
        Some(LogStatus::Running) => theme.primary,
        Some(LogStatus::Success) => theme.success,
        Some(LogStatus::Fail) => theme.danger,
        _ => theme.muted_foreground,
    };
    let icon = |s: Option<LogStatus>| match s {
        Some(LogStatus::Running) => "●",
        Some(LogStatus::Success) => "✓",
        Some(LogStatus::Fail) => "✗",
        _ => "·",
    };
    v_flex()
        .id("wf-log")
        .test_support()
        .border_1()
        .border_color(theme.border)
        .rounded(px(6.))
        .child(
            h_flex()
                .id("wf-log-header")
                .px_3()
                .py_1p5()
                .gap_2()
                .items_center()
                .cursor_pointer()
                .on_click(on_toggle)
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(t!("wfEditor.execLog")))
                .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(run.logs.len().to_string()))
                .child(div().flex_1())
                .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(if run.collapsed { "▸" } else { "▾" })),
        )
        .when(!run.collapsed, |d| {
            d.child(
                v_flex()
                    .id("wf-log-rows")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .px_3()
                    .pb_2()
                    .gap_0p5()
                    .when(run.logs.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("wfEditor.noLogs"))))
                    .children(run.logs.iter().enumerate().map(|(i, e)| {
                        let time = chrono::DateTime::from_timestamp_millis(e.timestamp_ms).map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).unwrap_or_default();
                        h_flex()
                            .id(ElementId::Name(format!("wf-log-row-{i}").into()))
                            .test_support()
                            .gap_2()
                            .text_size(px(12.))
                            .child(div().text_color(theme.muted_foreground).child(time))
                            .child(div().text_color(color(e.status)).child(icon(e.status)))
                            .when_some(e.node_name.clone(), |d, n| d.child(div().font_weight(FontWeight::MEDIUM).child(n)))
                            .child(div().min_w_0().text_color(color(e.status)).child(e.message.clone()))
                    })),
            )
        })
}

/// Dialog body for "execute from the list": title is the workflow, closing is allowed once it finished.
pub struct RunDialog {
    run: Entity<WorkflowRun>,
    _sub: Subscription,
}

impl RunDialog {
    pub fn open(run: Entity<WorkflowRun>, window: &mut Window, cx: &mut App) {
        let name = run.read(cx).workflow_name.clone();
        let view = cx.new(|cx| {
            let sub = cx.observe(&run, |_, _, cx| cx.notify());
            Self { run: run.clone(), _sub: sub }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(t!("wfList.executeLogTitle", name = name.clone())).w(px(760.)).close_button(false).overlay_closable(false).child(view.clone())
        });
    }
}

impl Render for RunDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let run = self.run.read(cx);
        let running = run.running;
        let this = self.run.clone();
        let toggle_run = self.run.clone();
        v_flex()
            .id("wf-run-dialog")
            .test_support()
            .gap_3()
            .child(log_panel(run, move |_, _, cx| toggle_run.update(cx, |r, cx| {
                r.collapsed = !r.collapsed;
                cx.notify();
            }), cx))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when(running, |d| d.child(Button::new("wf-run-stop").danger().label(t!("wfEditor.terminateWorkflow")).on_click(move |_, _, cx| this.update(cx, |r, cx| r.stop(cx)))))
                    .child(Button::new("wf-run-close").label(t!("common.close")).disabled_when(running).on_click(|_, window, cx| window.close_dialog(cx))),
            )
    }
}

trait DisabledWhen {
    fn disabled_when(self, cond: bool) -> Self;
}
impl DisabledWhen for Button {
    fn disabled_when(self, cond: bool) -> Self {
        use gpui_kit::component::Disableable as _;
        self.disabled(cond)
    }
}
