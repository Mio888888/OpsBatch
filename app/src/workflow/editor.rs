//! Workflow editor (port of `WorkflowEditor.tsx`): a node palette on the left, the free canvas (`canvas.rs`) in the
//! middle and the selected node's settings on the right. The saved JSON is exactly what the canvas editor wrote.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_workflow::model::{
    collect_workflow_host_ids, WorkflowRecord, WorkflowSettings, WorkflowStatus, NODE_GROUPS,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use super::canvas::{Drag, Menu, Viewport};
use super::graph::Graph;
use super::node_form::{NodeForm, NodeFormEvent};
use super::run::{log_panel, NodeStatus, WorkflowRun};
use super::store::{with_graph, workflow_store};
use crate::widgets::toast;

#[derive(Debug, Clone)]
pub enum EditorEvent {
    Back,
}

pub struct WorkflowEditor {
    record: WorkflowRecord,
    graph: Graph,
    interval: Entity<InputState>,
    palette_search: Entity<InputState>,
    name_input: Entity<InputState>,
    pub(super) selected: Option<String>,
    /// every selected node (the edit panel shows when it is exactly one)
    pub(super) multi: Vec<String>,
    form: Option<Entity<NodeForm>>,
    run: Entity<WorkflowRun>,
    // canvas state (see canvas.rs)
    pub(super) view: Viewport,
    pub(super) canvas_bounds: Rc<Cell<Bounds<Pixels>>>,
    fitted: bool,
    pub(super) drag: Option<Drag>,
    pub(super) selected_edge: Option<String>,
    pub(super) menu: Option<Menu>,
    pub(super) canvas_focus: FocusHandle,
    _subs: Vec<Subscription>,
    _form_sub: Option<Subscription>,
}

impl EventEmitter<EditorEvent> for WorkflowEditor {}

fn new_id() -> String {
    format!(
        "node-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        &uuid::Uuid::new_v4().simple().to_string()[..4]
    )
}

impl WorkflowEditor {
    pub fn new(
        record: WorkflowRecord,
        run: Entity<WorkflowRun>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let interval = cx.new(|cx| {
            InputState::new(window, cx).default_value(format_interval(
                record.settings.default_node_interval_seconds,
            ))
        });
        let palette_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("nodePalette.search")));
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("wfEditor.nodeName")));
        let subs = vec![
            cx.observe(&palette_search, |_, _, cx| cx.notify()),
            cx.observe(&run, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &name_input,
                window,
                |this, input, ev: &InputEvent, _, cx| {
                    if let (InputEvent::Change, Some(id)) = (ev, this.selected.clone()) {
                        let name = input.read(cx).value().to_string();
                        this.graph.set_name(&id, &name);
                        cx.notify();
                    }
                },
            ),
        ];
        let graph = Graph::new(record.nodes.clone(), record.connections.clone());
        Self {
            record,
            graph,
            interval,
            palette_search,
            name_input,
            selected: None,
            multi: Vec::new(),
            form: None,
            run,
            view: Viewport::default(),
            canvas_bounds: Rc::new(Cell::new(Bounds::default())),
            fitted: false,
            drag: None,
            selected_edge: None,
            menu: None,
            canvas_focus: cx.focus_handle(),
            _subs: subs,
            _form_sub: None,
        }
    }

    // ---- accessors (tests, canvas) ----

    pub fn graph(&self) -> &Graph {
        &self.graph
    }
    pub(super) fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }
    pub fn form(&self) -> Option<&Entity<NodeForm>> {
        self.form.as_ref()
    }
    pub fn interval_input(&self) -> &Entity<InputState> {
        &self.interval
    }
    pub fn workflow_id(&self) -> &str {
        &self.record.id
    }
    pub fn executing(&self, cx: &App) -> bool {
        let run = self.run.read(cx);
        run.running && run.workflow_id.as_deref() == Some(self.record.id.as_str())
    }

    /// Node statuses of the current run of this workflow (empty when another workflow ran last).
    pub(super) fn statuses(&self, cx: &App) -> HashMap<String, NodeStatus> {
        let run = self.run.read(cx);
        if run.workflow_id.as_deref() == Some(self.record.id.as_str()) {
            run.statuses.clone()
        } else {
            HashMap::new()
        }
    }

    fn interval_seconds(&self, cx: &App) -> f64 {
        self.interval
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.0)
    }

    /// The record as it would be saved right now.
    pub fn current_record(&self, cx: &App) -> WorkflowRecord {
        let mut rec = with_graph(
            &self.record,
            self.graph.nodes.clone(),
            self.graph.connections.clone(),
        );
        rec.settings = WorkflowSettings {
            default_node_interval_seconds: self.interval_seconds(cx),
        };
        rec
    }

    // ---- editing ----

    /// Add a node of `kind` near the middle of what is visible (like the original's palette click).
    pub fn add_node(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = new_id();
        let b = self.canvas_bounds.get();
        let (w, h) = if f32::from(b.size.width) > 0.0 {
            (f32::from(b.size.width), f32::from(b.size.height))
        } else {
            (800.0, 600.0)
        };
        let seed = uuid::Uuid::new_v4().as_bytes().to_vec();
        let (jx, jy) = (
            (seed[0] as f32 / 255.0 - 0.5) * 100.0,
            (seed[1] as f32 / 255.0 - 0.5) * 80.0,
        );
        let (fx, fy) = self.view.to_flow(w / 2.0 - 80.0 + jx, h / 2.0 - 28.0 + jy);
        self.graph.add_node(kind, id.clone());
        self.graph.set_position(&id, fx as f64, fy as f64);
        self.select(Some(id), window, cx);
    }

    pub fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id.filter(|id| self.graph.node(id).is_some());
        self.multi = self.selected.iter().cloned().collect();
        self.form = None;
        self._form_sub = None;
        if let Some(node) = self
            .selected
            .as_deref()
            .and_then(|id| self.graph.node(id))
            .cloned()
        {
            self.selected_edge = None;
            let name = node.name.clone();
            self.name_input
                .update(cx, |s, cx| s.set_value(name, window, cx));
            let assets = cx.global::<crate::ui_state::UiState>().assets.clone();
            let form = cx.new(|cx| NodeForm::new(&node.kind, &node.config, assets, window, cx));
            let node_id = node.id.clone();
            self._form_sub = Some(cx.subscribe(&form, move |this, _, ev: &NodeFormEvent, cx| {
                let NodeFormEvent::Config(config) = ev;
                this.graph.set_config(&node_id, config);
                cx.notify();
            }));
            self.form = Some(form);
        }
        cx.notify();
    }

    /// Replace the selection; the edit panel shows only for a single node.
    pub fn set_multi(&mut self, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| self.graph.node(id).is_some())
            .collect();
        if ids.len() == 1 {
            self.select(Some(ids[0].clone()), window, cx);
            return;
        }
        self.select(None, window, cx);
        self.multi = ids;
        cx.notify();
    }

    pub fn selected_nodes(&self) -> &[String] {
        &self.multi
    }

    pub fn select_edge(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if id.is_some() && self.selected.is_some() {
            self.select(None, window, cx);
        }
        self.selected_edge = id;
        cx.notify();
    }

    pub fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            self.graph.remove_node(&id);
            self.select(None, window, cx);
        }
    }

    pub fn toggle_enabled(&mut self, id: &str, cx: &mut Context<Self>) {
        let enabled = self.graph.node(id).map(|n| !n.enabled).unwrap_or(true);
        self.graph.set_enabled(id, enabled);
        cx.notify();
    }

    /// Connect `from` (at the port `handle`, `default` for ordinary nodes) to `to`. Self loops and duplicates are ignored.
    pub fn connect(
        &mut self,
        from: &str,
        to: &str,
        handle: Option<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let handle = handle.unwrap_or("default");
        let id = super::canvas::edge_id(from, to, handle);
        let duplicate = self.graph.connections.iter().any(|c| {
            c.from_id == from
                && c.to_id == to
                && c.source_handle.as_deref().unwrap_or("default") == handle
        });
        let ok = !duplicate && self.graph.connect(from, to, Some(handle));
        if ok {
            if let Some(c) = self.graph.connections.last_mut() {
                c.id = id;
            }
        }
        cx.notify();
        ok
    }

    pub fn disconnect(&mut self, edge_id: &str, cx: &mut Context<Self>) {
        self.graph.disconnect(edge_id);
        cx.notify();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut rec = self.current_record(cx);
        rec.status = WorkflowStatus::Ready;
        self.record = rec.clone();
        let task = workflow_store(cx).update(cx, |s, cx| s.save_workflow(rec, cx));
        cx.spawn_in(window, async move |_, cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| match result {
                Ok(()) => toast::success(window, cx, t!("wfEditor.saved")),
                Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
            });
        })
        .detach();
    }

    pub fn execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rec = self.current_record(cx);
        if collect_workflow_host_ids(&rec.nodes).is_empty() {
            toast::warning(window, cx, t!("wfEditor.addHostNode"));
            return;
        }
        self.run.update(cx, |r, cx| r.start(&rec, window, cx));
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.run.update(cx, |r, cx| r.stop(cx));
    }

    // ---- rendering ----

    /// `.wf-palette`: 200px, soft background, header / search / grouped list.
    fn palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let q = self.palette_search.read(cx).value().trim().to_lowercase();
        v_flex()
            .id("wf-palette")
            .test_support()
            .w(px(200.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(ui.border)
            .bg(ui.main)
            .child(
                div()
                    .px(px(12.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(ui.border)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child(t!("nodePalette.title")),
            )
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(ui.border)
                    .child(Input::new(&self.palette_search).cleanable(true)),
            )
            .child(
                v_flex()
                    .id("wf-palette-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py(px(4.))
                    .children(NODE_GROUPS.iter().filter_map(|group| {
                        let types: Vec<_> = group
                            .types
                            .iter()
                            .filter(|ty| {
                                q.is_empty()
                                    || t!(ty.label_key).to_lowercase().contains(&q)
                                    || ty.fallback_label.to_lowercase().contains(&q)
                                    || ty.value.to_lowercase().contains(&q)
                            })
                            .collect();
                        if types.is_empty() {
                            return None;
                        }
                        Some(
                            v_flex()
                                .pb(px(4.))
                                .child(
                                    div()
                                        .px(px(12.))
                                        .pt(px(6.))
                                        .pb(px(2.))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(ui.text_muted)
                                        .child(t!(group.label_key)),
                                )
                                .children(types.into_iter().map(|ty| {
                                    let kind = ty.value;
                                    h_flex()
                                        .id(ElementId::Name(format!("wf-palette-{kind}").into()))
                                        .test_support()
                                        .gap(px(8.))
                                        .items_center()
                                        .px(px(12.))
                                        .h(px(30.))
                                        .cursor_pointer()
                                        .hover(|d| d.bg(rgba(0x1677ff14)))
                                        .child(
                                            div()
                                                .flex_none()
                                                .size(px(8.))
                                                .rounded_full()
                                                .bg(rgb(ty.color)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(ui.text)
                                                .child(t!(ty.label_key)),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.add_node(kind, window, cx)
                                        }))
                                })),
                        )
                    })),
            )
    }

    /// `.wf-edit-panel`: 280px, name, enabled checkbox, node configuration and the delete button.
    fn side_panel(
        &self,
        node: &ops_workflow::model::CanvasNode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui(cx);
        let id = node.id.clone();
        v_flex()
            .id("wf-edit-panel")
            .test_support()
            .w(px(280.))
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(ui.border)
            .bg(ui.card)
            .child(
                h_flex()
                    .px(px(12.))
                    .py(px(10.))
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(ui.border)
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text)
                            .child(t!("wfEditor.editNode")),
                    )
                    .child(
                        Button::new("wf-edit-close")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::X))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.select(None, window, cx)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("wf-edit-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(12.))
                    .gap(px(12.))
                    .child(
                        v_flex()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ui.text_muted)
                                    .child(t!("wfEditor.nodeName")),
                            )
                            .child(Input::new(&self.name_input)),
                    )
                    .child(
                        h_flex()
                            .id("wf-edit-enabled-row")
                            .gap(px(6.))
                            .items_center()
                            .cursor_pointer()
                            .on_click({
                                let id = id.clone();
                                cx.listener(move |this, _, _, cx| this.toggle_enabled(&id, cx))
                            })
                            .child(Checkbox::new("wf-edit-enabled").checked(node.enabled))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(ui.text)
                                    .child(t!("wfEditor.enableNode")),
                            )
                            .test_support(),
                    )
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .pb(px(2.))
                                    .border_b_1()
                                    .border_color(ui.border)
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ui.text_muted)
                                    .child(t!("wfEditor.config")),
                            )
                            .when_some(self.form.clone(), |d, f| d.child(f)),
                    )
                    .child(
                        div()
                            .mt_auto()
                            .pt(px(12.))
                            .border_t_1()
                            .border_color(ui.border)
                            .child(
                                Button::new("wf-delete-node")
                                    .danger()
                                    .label(t!("wfEditor.deleteNode"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_selected(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn format_interval(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        v.to_string()
    }
}

impl Render for WorkflowEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let executing = self.executing(cx);
        let rec_for_hosts = collect_workflow_host_ids(&self.graph.nodes);
        let selected_node = self
            .selected
            .as_deref()
            .and_then(|id| self.graph.node(id))
            .cloned();
        let show_log = {
            let run = self.run.read(cx);
            run.workflow_id.as_deref() == Some(self.record.id.as_str()) && !run.logs.is_empty()
        };
        let run_entity = self.run.clone();
        // `fitView` once the canvas has a size
        if !self.fitted && f32::from(self.canvas_bounds.get().size.width) > 0.0 {
            self.fitted = true;
            if !self.graph.nodes.is_empty() {
                let b = self.canvas_bounds.get();
                let statuses = self.statuses(cx);
                let geoms: Vec<_> = self
                    .graph
                    .nodes
                    .iter()
                    .map(|n| super::canvas::node_geom(n, statuses.contains_key(&n.id)))
                    .collect();
                self.view = super::canvas::fit_view(
                    &geoms,
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                );
            }
        }
        let canvas = self.render_canvas(cx);
        use crate::widgets::form::{ui_tag, TagColor};

        v_flex()
            .id("wf-editor")
            .test_support()
            .size_full()
            .bg(ui.card)
            .child(
                // `.workflow-editor-header`
                h_flex()
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .bg(ui.card)
                    .border_b_1()
                    .border_color(ui.border)
                    .child(
                        Button::new("wf-back")
                            .label(t!("wfEditor.backToList"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Back))),
                    )
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ui.text)
                            .child(self.record.name.clone()),
                    )
                    .when(self.record.status == WorkflowStatus::Ready, |d| {
                        d.child(ui_tag(t!("wfList.ready"), Some(TagColor::Green), cx))
                    })
                    .when(!rec_for_hosts.is_empty(), |d| {
                        d.child(
                            ui_tag(
                                t!("wfEditor.hostSelected", count = rec_for_hosts.len()),
                                Some(TagColor::Blue),
                                cx,
                            )
                            .id("wf-host-tag")
                            .test_support(),
                        )
                    })
                    .when(executing, |d| {
                        d.child(
                            ui_tag(t!("wfEditor.executing"), Some(TagColor::Orange), cx)
                                .id("wf-executing-tag")
                                .test_support(),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .text_size(px(12.))
                            .text_color(ui.text_secondary)
                            .child(t!("wfEditor.defaultNodeInterval"))
                            .child(
                                h_flex()
                                    .w(px(86.))
                                    .h(px(30.))
                                    .overflow_hidden()
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(ui.input_border)
                                    .bg(ui.input_bg)
                                    .child(
                                        div().flex_1().min_w_0().child(crate::widgets::form::spin(
                                            Input::new(&self.interval)
                                                .small()
                                                .appearance(false)
                                                .disabled(executing),
                                            "wf-interval",
                                            &self.interval,
                                            0,
                                            None,
                                            ui,
                                        )),
                                    )
                                    .child(
                                        div()
                                            .h_full()
                                            .px(px(8.))
                                            .flex()
                                            .items_center()
                                            .border_l_1()
                                            .border_color(ui.input_border)
                                            .bg(if ui.is_dark {
                                                white().opacity(0.04)
                                            } else {
                                                rgb(0xf5f5f5).into()
                                            })
                                            .text_color(if ui.is_dark {
                                                ui.text_muted
                                            } else {
                                                rgb(0x7c8794).into()
                                            })
                                            .child("s"),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("wf-save")
                            .icon(Icon::new(IconName::Save))
                            .label(t!("wfEditor.save"))
                            .disabled(executing)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .when(executing, |d| {
                        d.child(
                            Button::new("wf-stop")
                                .danger()
                                .label(t!("wfEditor.terminateWorkflow"))
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        )
                    })
                    .child(
                        Button::new("wf-run")
                            .primary()
                            .icon(Icon::new(IconName::CirclePlay))
                            .label(t!("wfEditor.executeWorkflow"))
                            .disabled(self.graph.nodes.is_empty() || executing)
                            .on_click(cx.listener(|this, _, window, cx| this.execute(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.palette(cx))
                    .child(canvas)
                    .when_some(selected_node, |d, node| d.child(self.side_panel(&node, cx))),
            )
            .when(show_log, |d| {
                let toggle = run_entity.clone();
                d.child(div().p_2().child(log_panel(
                    run_entity.read(cx),
                    move |_, _, cx| {
                        toggle.update(cx, |r, cx| {
                            r.collapsed = !r.collapsed;
                            cx.notify();
                        })
                    },
                    cx,
                )))
            })
    }
}
