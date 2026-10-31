//! The "Workflow" mode page (port of `WorkflowPage.tsx`, `WorkflowList.tsx`, `TemplateManager.tsx` and
//! `ScheduledTaskManager.tsx`): workflows, templates and scheduled tasks, creating, running and editing.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_workflow::model::{collect_workflow_host_ids, WorkflowRecord, WorkflowStatus};
use ops_workflow::store::WorkflowTemplate;

use super::editor::{EditorEvent, WorkflowEditor};
use super::run::{RunDialog, RunEvent, WorkflowRun};
use super::store::{workflow_store, WorkflowStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::field;
use crate::widgets::toast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTab {
    List,
    Templates,
    Scheduled,
}

pub struct WorkflowPage {
    store: Entity<WorkflowStore>,
    tab: PageTab,
    editor: Option<Entity<WorkflowEditor>>,
    editor_sub: Option<Subscription>,
    run: Entity<WorkflowRun>,
    /// The current run was started from the list (its result is announced with a toast).
    list_run: bool,
    _subs: Vec<Subscription>,
}

impl WorkflowPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = workflow_store(cx);
        let run = cx.new(|_| WorkflowRun::default());
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&run, |_, _, cx| cx.notify()),
            cx.subscribe_in(&run, window, |this, _, _: &RunEvent, window, cx| {
                if std::mem::take(&mut this.list_run) {
                    toast::success(window, cx, t!("wfList.executeCompleted"));
                }
            }),
        ];
        store.update(cx, |s, cx| s.reload_all(cx));
        // The editor's host picker needs the host list.
        cx.global::<crate::ui_state::UiState>()
            .assets
            .clone()
            .update(cx, |a, cx| a.reload(cx));
        Self {
            store,
            tab: PageTab::List,
            editor: None,
            editor_sub: None,
            run,
            list_run: false,
            _subs: subs,
        }
    }

    pub fn tab(&self) -> PageTab {
        self.tab
    }
    pub fn editor(&self) -> Option<&Entity<WorkflowEditor>> {
        self.editor.as_ref()
    }
    pub fn run(&self) -> &Entity<WorkflowRun> {
        &self.run
    }
    pub fn set_tab(&mut self, tab: PageTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    fn running(&self, cx: &App) -> bool {
        self.run.read(cx).running
    }

    // ---- actions ----

    pub fn open_editor(
        &mut self,
        record: WorkflowRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run = self.run.clone();
        let editor = cx.new(|cx| WorkflowEditor::new(record, run, window, cx));
        self.editor_sub = Some(
            cx.subscribe(&editor, |this, _, ev: &EditorEvent, cx| match ev {
                EditorEvent::Back => {
                    this.editor = None;
                    this.editor_sub = None;
                    this.store.update(cx, |s, cx| s.reload_workflows(cx));
                    cx.notify();
                }
            }),
        );
        self.editor = Some(editor);
        cx.notify();
    }

    /// Run a workflow from the list, with the checks and messages of the original.
    pub fn execute_from_list(
        &mut self,
        workflow: &WorkflowRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = self.run.read(cx);
        if running.running {
            let same = running.workflow_id.as_deref() == Some(workflow.id.as_str());
            toast::warning(
                window,
                cx,
                if same {
                    t!("wfList.workflowAlreadyExecuting")
                } else {
                    t!("wfList.anotherWorkflowExecuting")
                },
            );
            return;
        }
        if workflow.nodes.is_empty() {
            toast::warning(window, cx, t!("wfList.noNodesToExecute"));
            return;
        }
        if collect_workflow_host_ids(&workflow.nodes).is_empty() {
            toast::warning(window, cx, t!("wfList.noTargetHosts"));
            return;
        }
        self.list_run = true;
        self.run.update(cx, |r, cx| r.start(workflow, window, cx));
        RunDialog::open(self.run.clone(), window, cx);
        toast::success(window, cx, t!("wfList.executeStarted"));
    }

    fn create_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity();
        let view = cx.new(|cx| CreateWorkflowDialog::new(page, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(t!("workflow.createWorkflowModal"))
                .w(px(460.))
                .child(view.clone())
        });
    }

    fn template_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity();
        let store = self.store.clone();
        let view = cx.new(|cx| TemplatePicker::new(page, store, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(t!("workflow.createTemplateModal"))
                .w(px(700.))
                .child(view.clone())
        });
    }

    pub fn use_template(
        &mut self,
        template: WorkflowTemplate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = self
            .store
            .update(cx, |s, cx| s.create_from_template(template, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(record) => {
                    window.close_dialog(cx);
                    toast::success(window, cx, t!("workflow.createdFromTemplate"));
                    this.open_editor(record, window, cx);
                }
                Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
            });
        })
        .detach();
    }

    fn confirm_delete(
        &mut self,
        title: String,
        description: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_ok: impl Fn(&mut Window, &mut App) + 'static,
    ) {
        let on_ok = std::rc::Rc::new(on_ok);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let on_ok = on_ok.clone();
            alert
                .confirm()
                .title(title.clone())
                .description(description.clone())
                .show_cancel(true)
                .ok_text(t!("common.delete"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, window, cx| {
                    on_ok(window, cx);
                    true
                })
        });
    }

    fn delete_workflow(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        self.confirm_delete(
            t!("wfList.confirmDelete"),
            String::new(),
            window,
            cx,
            move |window, cx| {
                let task = store.update(cx, |s, cx| s.delete_workflow(id.clone(), cx));
                window
                    .spawn(cx, async move |cx| {
                        let r = task.await;
                        let _ = cx.update(|window, cx| match r {
                            Ok(()) => toast::success(window, cx, t!("library.deleted")),
                            Err(e) => {
                                toast::error(window, cx, t!("common.deleteFailed", error = e))
                            }
                        });
                    })
                    .detach();
            },
        );
    }

    fn save_as_template(
        &mut self,
        wf: WorkflowRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = self.store.update(cx, |s, cx| {
            s.save_as_template(&wf, &t!("workflow.templateTab"), cx)
        });
        cx.spawn_in(window, async move |_, cx| {
            let r = task.await;
            let _ = cx.update(|window, cx| match r {
                Ok(()) => toast::success(window, cx, t!("wfList.savedAsTemplate")),
                Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
            });
        })
        .detach();
    }

    fn delete_template(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        self.confirm_delete(
            t!("workflow.confirmDeleteTemplate"),
            String::new(),
            window,
            cx,
            move |window, cx| {
                let task = store.update(cx, |s, cx| s.delete_template(id.clone(), cx));
                window
                    .spawn(cx, async move |cx| {
                        let r = task.await;
                        let _ = cx.update(|window, cx| match r {
                            Ok(()) => toast::success(window, cx, t!("workflow.templateDeleted")),
                            Err(e) => {
                                toast::error(window, cx, t!("common.deleteFailed", error = e))
                            }
                        });
                    })
                    .detach();
            },
        );
    }

    fn task_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let view = cx.new(|cx| TaskDialog::new(store, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(t!("scheduledTask.createModal"))
                .w(px(480.))
                .child(view.clone())
        });
    }

    fn delete_task(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        self.confirm_delete(
            t!("scheduledTask.confirmDeleteTask"),
            String::new(),
            window,
            cx,
            move |window, cx| {
                let task = store.update(cx, |s, cx| s.delete_task(id.clone(), cx));
                window
                    .spawn(cx, async move |cx| {
                        let r = task.await;
                        let _ = cx.update(|window, cx| match r {
                            Ok(()) => toast::success(window, cx, t!("scheduledTask.deleted")),
                            Err(e) => {
                                toast::error(window, cx, t!("common.deleteFailed", error = e))
                            }
                        });
                    })
                    .detach();
            },
        );
    }

    // ---- rendering ----

    fn section_header(
        &self,
        eyebrow: String,
        title: String,
        desc: String,
        actions: impl IntoElement,
        cx: &App,
    ) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let line = crate::widgets::form::dash(cx).line;
        // `.workflow-section-header`: tinted panel, 14/15 padding, 10px corners
        h_flex()
            .flex_none()
            .gap(px(14.))
            .px(px(15.))
            .py(px(14.))
            .justify_between()
            .items_start()
            .rounded(px(10.))
            .border_1()
            .border_color(line)
            .bg(crate::widgets::form::dash(cx)
                .panel
                .blend(ui.primary.opacity(0.1)))
            .shadow(crate::widgets::form::dash_shadow(
                &crate::widgets::form::dash(cx),
            ))
            .child(
                v_flex()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .line_height(relative(1.2))
                            .text_color(ui.primary)
                            .child(eyebrow.to_uppercase()),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .line_height(relative(1.2))
                            .text_color(ui.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(7.))
                            .max_w(px(520.))
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(ui.text_muted)
                            .child(desc),
                    ),
            )
            .child(actions)
    }

    fn empty_card(
        &self,
        id: &'static str,
        title: String,
        desc: String,
        actions: impl IntoElement,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id(id)
            .test_support()
            .items_center()
            .gap_2()
            .p_8()
            .rounded(px(10.))
            .border_1()
            .border_color(theme.border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(desc),
            )
            .child(actions)
    }

    fn list_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let running = self.running(cx);
        let run = self.run.read(cx);
        let running_id = run.running.then(|| run.workflow_id.clone()).flatten();
        let workflows = self.store.read(cx).workflows.clone();
        let actions = h_flex()
            .gap_2()
            .items_center()
            .when(running, |d| {
                d.child(
                    div()
                        .id("wf-running-pill")
                        .test_support()
                        .text_size(px(12.))
                        .text_color(theme.warning)
                        .child(t!("workflow.runningCount", count = 1)),
                )
            })
            .child(
                Button::new("wf-create-from-template")
                    .label(t!("workflow.createFromTemplate"))
                    .disabled(running)
                    .on_click(cx.listener(|this, _, window, cx| this.template_dialog(window, cx))),
            )
            .child(
                Button::new("wf-create")
                    .primary()
                    .icon(Icon::new(IconName::Plus))
                    .label(t!("workflow.createWorkflow"))
                    .disabled(running)
                    .on_click(cx.listener(|this, _, window, cx| this.create_dialog(window, cx))),
            );
        let header = self.section_header(
            t!("workflow.workflowTab"),
            t!("workflow.workflowListTitle"),
            t!("workflow.workflowListDesc"),
            actions,
            cx,
        );

        let body: AnyElement = if workflows.is_empty() {
            self.empty_card(
                "wf-empty",
                t!("wfList.noWorkflows"),
                t!("wfList.emptyDesc"),
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("wf-empty-template")
                            .label(t!("workflow.createFromTemplate"))
                            .disabled(running)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.template_dialog(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("wf-empty-create")
                            .primary()
                            .label(t!("workflow.createWorkflow"))
                            .disabled(running)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.create_dialog(window, cx)),
                            ),
                    ),
                cx,
            )
            .into_any_element()
        } else {
            let ui = crate::theme::ui(cx);
            let dash = crate::widgets::form::dash(cx);
            let (line, panel, strong) = (dash.line, dash.panel, dash.strong);
            let amber: Hsla = gpui_kit::rgb(0xfaad14).into();
            let green: Hsla = gpui_kit::rgb(0x52c41a).into();
            div()
                .id("wf-grid")
                .test_support()
                .grid()
                .grid_cols(2)
                .gap(px(10.))
                .children(workflows.into_iter().enumerate().map(|(i, wf)| {
                    let host_ids = collect_workflow_host_ids(&wf.nodes);
                    let executing = running_id.as_deref() == Some(wf.id.as_str());
                    let can_execute = !wf.nodes.is_empty() && !host_ids.is_empty() && !running;
                    let ready = wf.status == WorkflowStatus::Ready;
                    let (exec_wf, edit_wf, tpl_wf, del_id) =
                        (wf.clone(), wf.clone(), wf.clone(), wf.id.clone());
                    let meta = |n: usize, label: String, accent: bool| {
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .px(px(9.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(line.opacity(0.68))
                            .bg(strong.opacity(0.7))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .line_height(relative(1.))
                                    .font_family("Menlo")
                                    .text_color(if accent {
                                        crate::widgets::form::mix(ui.text, ui.primary, 0.82)
                                    } else {
                                        ui.text
                                    })
                                    .child(n.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(ui.text_muted)
                                    .child(label),
                            )
                    };
                    // `.workflow-card`: 3px bar (amber for drafts, green when ready), 14px padding, 12px rows
                    v_flex()
                        .id(ElementId::Name(format!("wf-card-{i}").into()))
                        .test_support()
                        .relative()
                        .overflow_hidden()
                        .min_w_0()
                        .gap(px(12.))
                        .p(px(14.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(line)
                        .bg(panel)
                        .shadow(crate::widgets::form::dash_shadow(&dash))
                        .child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(
                            if ready {
                                green.opacity(0.65)
                            } else {
                                amber.opacity(0.62)
                            },
                        ))
                        .child(
                            h_flex()
                                .justify_between()
                                .gap(px(12.))
                                .items_center()
                                .child(
                                    h_flex()
                                        .gap(px(11.))
                                        .min_w_0()
                                        .items_center()
                                        .child(
                                            div()
                                                .size(px(34.))
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(8.))
                                                .border_1()
                                                .border_color(ui.primary.opacity(0.21))
                                                .bg(ui.primary.opacity(0.12))
                                                .text_color(ui.primary)
                                                .child(Icon::new(IconName::Archive).size_4()),
                                        )
                                        .child(
                                            v_flex()
                                                .min_w_0()
                                                .gap(px(3.))
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .line_height(relative(1.35))
                                                        .text_color(ui.text)
                                                        .child(wf.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .font_family("Menlo")
                                                        .text_size(px(11.))
                                                        .text_color(ui.text_muted)
                                                        .child(t!(
                                                            "wfList.updatedAt",
                                                            time = if wf.updated_at.is_empty() {
                                                                "-".to_string()
                                                            } else {
                                                                wf.updated_at.clone()
                                                            }
                                                        )),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .flex_none()
                                        .gap(px(6.))
                                        .items_center()
                                        .px(px(8.))
                                        .py(px(4.))
                                        .rounded(px(7.))
                                        .border_1()
                                        .border_color(line.opacity(0.86))
                                        .bg(strong.opacity(0.78))
                                        .text_size(px(12.))
                                        .text_color(ui.text_muted)
                                        .child(div().size(px(6.)).rounded_full().bg(if ready {
                                            green
                                        } else {
                                            amber
                                        }))
                                        .child(if ready {
                                            t!("wfList.ready")
                                        } else {
                                            t!("wfList.draft")
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .min_h(px(42.))
                                .text_size(px(12.))
                                .line_height(relative(1.5))
                                .text_color(ui.text_muted)
                                .child(if wf.description.is_empty() {
                                    t!("wfList.noDescription")
                                } else {
                                    wf.description.clone()
                                }),
                        )
                        .child(
                            h_flex()
                                .gap(px(10.))
                                .child(meta(wf.nodes.len(), t!("wfList.nodeCount"), false))
                                .child(meta(
                                    wf.connections.len(),
                                    t!("wfList.connectionCount"),
                                    false,
                                ))
                                .child(meta(host_ids.len(), t!("wfList.targetHosts"), true)),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .gap(px(10.))
                                .items_center()
                                .flex_wrap()
                                .pt(px(11.))
                                .border_t_1()
                                .border_color(line.opacity(0.62))
                                .child({
                                    let ok = can_execute || executing;
                                    crate::widgets::form::ui_tag(
                                        if ok {
                                            t!("wfList.executable")
                                        } else {
                                            t!("wfList.needsConfig")
                                        },
                                        Some(if ok {
                                            crate::widgets::form::TagColor::Green
                                        } else {
                                            crate::widgets::form::TagColor::Orange
                                        }),
                                        cx,
                                    )
                                    .id(ElementId::Name(format!("wf-state-{i}").into()))
                                    .test_support()
                                })
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .child(
                                            Button::new(ElementId::Name(
                                                format!("wf-exec-{i}").into(),
                                            ))
                                            .primary()
                                            .small()
                                            .icon(Icon::new(IconName::CirclePlay))
                                            .label(if executing {
                                                t!("wfList.executing")
                                            } else {
                                                t!("wfList.execute")
                                            })
                                            .loading(executing)
                                            .disabled(!can_execute)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.execute_from_list(&exec_wf, window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(ElementId::Name(
                                                format!("wf-edit-{i}").into(),
                                            ))
                                            .small()
                                            .icon(Icon::new(IconName::SquarePen))
                                            .label(t!("wfList.edit"))
                                            .disabled(running)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_editor(edit_wf.clone(), window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(ElementId::Name(
                                                format!("wf-tpl-{i}").into(),
                                            ))
                                            .small()
                                            .icon(Icon::new(IconName::Save))
                                            .label(t!("wfList.saveAsTemplate"))
                                            .disabled(running)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.save_as_template(tpl_wf.clone(), window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(ElementId::Name(
                                                format!("wf-del-{i}").into(),
                                            ))
                                            .small()
                                            .danger()
                                            .icon(Icon::new(IconName::Trash))
                                            .disabled(running)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.delete_workflow(del_id.clone(), window, cx)
                                            })),
                                        ),
                                ),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .gap(px(12.))
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn templates_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let templates = self.store.read(cx).templates.clone();
        let header = self.section_header(
            t!("workflow.templateTab"),
            t!("workflow.workflowTemplates"),
            t!("workflow.templateDesc"),
            div(),
            cx,
        );
        let body: AnyElement = if templates.is_empty() {
            self.empty_card(
                "wf-tpl-empty",
                t!("workflow.noTemplatesTitle"),
                t!("workflow.noTemplates"),
                div(),
                cx,
            )
            .into_any_element()
        } else {
            v_flex()
                .id("wf-tpl-table")
                .test_support()
                .border_1()
                .border_color(theme.border)
                .rounded(px(8.))
                .children(templates.into_iter().enumerate().map(|(i, tpl)| {
                    let (use_tpl, del_id) = (tpl.clone(), tpl.id.clone());
                    h_flex()
                        .id(ElementId::Name(format!("wf-tpl-row-{i}").into()))
                        .test_support()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .w(px(220.))
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(tpl.name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(tpl.description.clone()),
                        )
                        .child(
                            div()
                                .w(px(160.))
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(tpl.created_at.clone()),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("wf-tpl-use-{i}").into()))
                                .primary()
                                .small()
                                .icon(Icon::new(IconName::Plus))
                                .label(t!("workflow.createFrom"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.use_template(use_tpl.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("wf-tpl-del-{i}").into()))
                                .small()
                                .danger()
                                .icon(Icon::new(IconName::Trash))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.delete_template(del_id.clone(), window, cx)
                                })),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .gap(px(12.))
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn scheduled_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (tasks, workflows) = {
            let s = self.store.read(cx);
            (s.tasks.clone(), s.workflows.clone())
        };
        let header = self.section_header(
            t!("workflow.scheduledTab"),
            t!("scheduledTask.title"),
            t!("scheduledTask.description"),
            Button::new("wf-task-create")
                .primary()
                .icon(Icon::new(IconName::Plus))
                .label(t!("scheduledTask.createTask"))
                .on_click(cx.listener(|this, _, window, cx| this.task_dialog(window, cx))),
            cx,
        );
        let body: AnyElement =
            if tasks.is_empty() {
                self.empty_card(
                    "wf-task-empty",
                    t!("scheduledTask.noTasks"),
                    t!("scheduledTask.emptyDesc"),
                    Button::new("wf-task-empty-create")
                        .primary()
                        .label(t!("scheduledTask.createTaskBtn"))
                        .on_click(cx.listener(|this, _, window, cx| this.task_dialog(window, cx))),
                    cx,
                )
                .into_any_element()
            } else {
                v_flex()
                    .id("wf-task-table")
                    .test_support()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.))
                    .children(tasks.into_iter().enumerate().map(|(i, task)| {
                        let wf_name = workflows
                            .iter()
                            .find(|w| w.id == task.workflow_id)
                            .map(|w| w.name.clone());
                        let (toggle_id, del_id) = (task.id.clone(), task.id.clone());
                        let this = cx.entity();
                        h_flex()
                            .id(ElementId::Name(format!("wf-task-row-{i}").into()))
                            .test_support()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .w(px(200.))
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(task.name.clone()),
                            )
                            .child(
                                div()
                                    .w(px(160.))
                                    .font_family("Menlo")
                                    .text_size(px(12.))
                                    .child(task.cron.clone()),
                            )
                            .child(div().w(px(180.)).truncate().text_size(px(12.)).child(
                                wf_name.unwrap_or_else(|| t!("scheduledTask.deletedWorkflow")),
                            ))
                            .child(
                                Switch::new(ElementId::Name(format!("wf-task-enabled-{i}").into()))
                                    .checked(task.enabled)
                                    .on_click(move |v, _, cx| {
                                        let enabled = *v;
                                        let id = toggle_id.clone();
                                        this.update(cx, |this, cx| {
                                            if let Some(t) = this.store.update(cx, |s, cx| {
                                                s.set_task_enabled(&id, enabled, cx)
                                            }) {
                                                cx.spawn(async move |_, _| {
                                                    let _ = t.await;
                                                })
                                                .detach();
                                            }
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.))
                                    .text_color(theme.muted_foreground)
                                    .child(task.last_run_at.clone().unwrap_or_else(|| "-".into())),
                            )
                            .child(
                                Button::new(ElementId::Name(format!("wf-task-del-{i}").into()))
                                    .small()
                                    .danger()
                                    .icon(Icon::new(IconName::Trash))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete_task(del_id.clone(), window, cx)
                                    })),
                            )
                    }))
                    .into_any_element()
            };
        v_flex()
            .gap(px(12.))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

impl Render for WorkflowPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(editor) = self.editor.clone() {
            return div()
                .id("workflow-page")
                .test_support()
                .size_full()
                .child(editor)
                .into_any_element();
        }
        let theme = cx.theme().clone();
        let (n_wf, n_tpl, n_task) = {
            let s = self.store.read(cx);
            (s.workflows.len(), s.templates.len(), s.tasks.len())
        };
        let ui = crate::theme::ui(cx);
        let line = ui.border_strong.blend(ui.primary.opacity(0.09));
        let tab_button = |id: &'static str,
                          label: String,
                          count: usize,
                          tab: PageTab,
                          cx: &mut Context<Self>| {
            // `.workflow-tabs .ui-tabs-trigger`: 32px, 8px corners; the active one is tinted and ringed
            let active = self.tab == tab;
            h_flex()
                .id(id)
                .test_support()
                .h(px(32.))
                .px(px(10.))
                .gap(px(7.))
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(14.))
                .text_color(if active { ui.primary } else { ui.text_muted })
                .when(active, |d| {
                    d.bg(ui.card.blend(ui.primary.opacity(0.10)))
                        .border_1()
                        .border_color(ui.primary.opacity(0.18))
                })
                .when(!active, |d| {
                    d.hover(move |d| {
                        d.text_color(ui.text)
                            .bg(ui.card.blend(ui.primary.opacity(0.07)))
                    })
                })
                .child(label)
                .child(
                    div()
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded_full()
                        .bg(ui.text_muted.opacity(0.09))
                        .text_center()
                        .font_family("Menlo")
                        .text_size(px(11.))
                        .child(count.to_string()),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
        };
        let tabs = h_flex()
            .flex_none()
            .gap(px(6.))
            .p(px(4.))
            .rounded(px(10.))
            .border_1()
            .border_color(line)
            .bg(ui.card.blend(ui.primary.opacity(0.03)).opacity(0.76))
            .child(tab_button(
                "wf-tab-list",
                t!("workflow.workflowTab"),
                n_wf,
                PageTab::List,
                cx,
            ))
            .child(tab_button(
                "wf-tab-templates",
                t!("workflow.templateTab"),
                n_tpl,
                PageTab::Templates,
                cx,
            ))
            .child(tab_button(
                "wf-tab-scheduled",
                t!("workflow.scheduledTab"),
                n_task,
                PageTab::Scheduled,
                cx,
            ));
        let content = match self.tab {
            PageTab::List => self.list_tab(cx),
            PageTab::Templates => self.templates_tab(cx),
            PageTab::Scheduled => self.scheduled_tab(cx),
        };
        let _ = theme;
        crate::widgets::form::page_container(cx)
            .bg(crate::widgets::form::dashboard_bg(cx))
            .id("workflow-page")
            .test_support()
            .relative()
            .gap(px(12.))
            .child(crate::widgets::form::dashboard_backdrop(cx))
            .child(tabs)
            .child(
                div()
                    .id("workflow-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

struct CreateWorkflowDialog {
    page: Entity<WorkflowPage>,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    name_error: bool,
    busy: bool,
}

impl CreateWorkflowDialog {
    fn new(page: Entity<WorkflowPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("workflow.workflowNamePlaceholder"))
        });
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("workflow.descriptionPlaceholder"))
                .auto_grow(2, 5)
        });
        // The dialog takes focus when it opens; focus the name once that has happened.
        let focus = name.clone();
        window.defer(cx, move |window, cx| {
            focus.update(cx, |s, cx| s.focus(window, cx))
        });
        Self {
            page,
            name,
            description,
            name_error: false,
            busy: false,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        self.name_error = name.is_empty();
        if self.name_error || self.busy {
            cx.notify();
            return;
        }
        self.busy = true;
        let description = self.description.read(cx).value().to_string();
        let store = self.page.read(cx).store.clone();
        let task = store.update(cx, |s, cx| s.create_workflow(name, description, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(record) => {
                        window.close_dialog(cx);
                        toast::success(window, cx, t!("workflow.workflowCreated"));
                        this.page
                            .update(cx, |p, cx| p.open_editor(record, window, cx));
                    }
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CreateWorkflowDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("wf-create-dialog")
            .test_support()
            .gap_3()
            .child(field(
                t!("workflow.workflowName"),
                Input::new(&self.name),
                self.name_error.then(|| t!("common.required").into()),
                cx,
            ))
            .child(field(
                t!("workflow.description"),
                Textarea::new(&self.description),
                None,
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("wf-create-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("wf-create-ok")
                            .primary()
                            .label(t!("common.ok"))
                            .loading(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

struct TemplatePicker {
    page: Entity<WorkflowPage>,
    store: Entity<WorkflowStore>,
    _sub: Subscription,
}

impl TemplatePicker {
    fn new(
        page: Entity<WorkflowPage>,
        store: Entity<WorkflowStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            page,
            store,
            _sub: sub,
        }
    }
}

impl Render for TemplatePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let templates = self.store.read(cx).templates.clone();
        let busy = self.page.read(cx).running(cx);
        v_flex()
            .id("wf-template-picker")
            .test_support()
            .gap_2()
            .when(templates.is_empty(), |d| {
                d.child(
                    div()
                        .p_6()
                        .text_color(theme.muted_foreground)
                        .child(t!("workflow.noTemplates")),
                )
            })
            .children(templates.into_iter().enumerate().map(|(i, tpl)| {
                let pick = tpl.clone();
                let page = self.page.clone();
                h_flex()
                    .id(ElementId::Name(format!("wf-pick-row-{i}").into()))
                    .gap_3()
                    .items_center()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w(px(200.)).truncate().child(tpl.name.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(tpl.description.clone()),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("wf-use-tpl-{i}").into()))
                            .primary()
                            .small()
                            .label(t!("workflow.useTemplate"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let pick = pick.clone();
                                page.update(cx, |p, cx| p.use_template(pick, window, cx));
                            }),
                    )
            }))
    }
}

struct TaskDialog {
    store: Entity<WorkflowStore>,
    name: Entity<InputState>,
    rule: Entity<InputState>,
    workflow: Entity<ChoiceSelect>,
    workflow_id: Option<String>,
    errors: (bool, bool, bool),
    _sub: Subscription,
}

impl TaskDialog {
    fn new(store: Entity<WorkflowStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let choices: Vec<Choice> = store
            .read(cx)
            .workflows
            .iter()
            .map(|w| Choice::new(w.id.clone(), w.name.clone()))
            .collect();
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("scheduledTask.taskNamePlaceholder"))
        });
        let rule = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("scheduledTask.schedulePlaceholder"))
        });
        let workflow = cx.new(|cx| new_select(choices, "", window, cx));
        let sub = cx.subscribe(&workflow, |this, _, ev, cx| {
            if let Some(v) = confirmed_value(ev) {
                this.workflow_id = Some(v);
                cx.notify();
            }
        });
        Self {
            store,
            name,
            rule,
            workflow,
            workflow_id: None,
            errors: (false, false, false),
            _sub: sub,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        let rule = self.rule.read(cx).value().trim().to_string();
        self.errors = (name.is_empty(), self.workflow_id.is_none(), rule.is_empty());
        if self.errors != (false, false, false) {
            cx.notify();
            return;
        }
        let workflow_id = self.workflow_id.clone().unwrap_or_default();
        let task = self
            .store
            .update(cx, |s, cx| s.add_task(name, rule, workflow_id, cx));
        cx.spawn_in(window, async move |_, cx| {
            let r = task.await;
            let _ = cx.update(|window, cx| match r {
                Ok(()) => {
                    window.close_dialog(cx);
                    toast::success(window, cx, t!("scheduledTask.created"));
                }
                Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
            });
        })
        .detach();
    }
}

impl Render for TaskDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let req = |e: bool| e.then(|| t!("common.required").into());
        v_flex()
            .id("wf-task-dialog")
            .test_support()
            .gap_3()
            .child(field(
                t!("scheduledTask.taskName"),
                Input::new(&self.name),
                req(self.errors.0),
                cx,
            ))
            .child(field(
                t!("scheduledTask.linkedWorkflow"),
                Select::new(&self.workflow).placeholder(t!("scheduledTask.selectWorkflow")),
                req(self.errors.1),
                cx,
            ))
            .child(field(
                t!("scheduledTask.scheduleRule"),
                Input::new(&self.rule),
                req(self.errors.2),
                cx,
            ))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child(t!("scheduledTask.scheduleRuleExtra")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("wf-task-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("wf-task-ok")
                            .primary()
                            .label(t!("common.ok"))
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
