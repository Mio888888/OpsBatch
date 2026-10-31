//! Free canvas of the workflow editor (replaces the `@xyflow/react` canvas of `WorkflowEditor.tsx`): nodes dragged
//! around with the mouse, connections drawn from a node's right port to another node's left port, wheel zoom, drag
//! to pan, a mini map, zoom controls and the delete context menu. This file holds the geometry (pure, unit tested)
//! and the view; `WorkflowEditor` owns the state.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_workflow::model::{node_type_entry, CanvasNode};

use super::editor::WorkflowEditor;
use super::node_form::SwitchConfig;
use super::run::NodeStatus;
use crate::theme::ui;

pub const MIN_ZOOM: f32 = 0.3;
pub const MAX_ZOOM: f32 = 2.0;
/// Grid spacing of the dotted background (`<Background gap={20}>`).
const DOT_GAP: f32 = 20.0;
const SWITCH_PALETTE: [u32; 8] = [
    0x52c41a, 0x1677ff, 0xfa8c16, 0xeb2f96, 0x13c2c2, 0x722ed1, 0xff4d4f, 0x8c8c8c,
];

// ---------------------------------------------------------------------------------------------------------------
// geometry
// ---------------------------------------------------------------------------------------------------------------

/// Flow → screen transform: `screen = flow * zoom + (x, y)`, relative to the canvas' top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn to_screen(&self, fx: f32, fy: f32) -> (f32, f32) {
        (fx * self.zoom + self.x, fy * self.zoom + self.y)
    }

    pub fn to_flow(&self, sx: f32, sy: f32) -> (f32, f32) {
        ((sx - self.x) / self.zoom, (sy - self.y) / self.zoom)
    }

    /// Zoom to `zoom` (clamped) keeping the flow point under the screen point `(sx, sy)` where it is.
    pub fn zoom_at(&mut self, sx: f32, sy: f32, zoom: f32) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let (fx, fy) = self.to_flow(sx, sy);
        self.zoom = zoom;
        self.x = sx - fx * zoom;
        self.y = sy - fy * zoom;
    }
}

/// Size and position of a node in flow units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A source port: where it sits relative to the node's top-left corner (flow units) and how it is labelled.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// Connection handle id: `default`, `true`, `false` or `case-N`.
    pub handle: String,
    pub label: Option<String>,
    pub color: Option<u32>,
    pub dx: f32,
    pub dy: f32,
}

const BORDER: f32 = 2.0;
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;
const HEADER_H: f32 = 33.0;
const BRANCH_TOP: f32 = BORDER + PAD_Y + HEADER_H + 4.0;

/// Rough text width: full-width glyphs take the font size, everything else about 0.56 of it.
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                if c.is_ascii_uppercase() {
                    size * 0.64
                } else {
                    size * 0.56
                }
            } else {
                size
            }
        })
        .sum()
}

fn is_pill(kind: &str) -> bool {
    matches!(kind, "start" | "end")
}

pub fn has_target_port(kind: &str) -> bool {
    kind != "start"
}

pub fn has_source_port(kind: &str) -> bool {
    kind != "end"
}

fn switch_cases(config: &str) -> Vec<String> {
    let cases = SwitchConfig::parse(config).cases;
    if cases.is_empty() {
        vec!["default".to_string()]
    } else {
        cases.into_iter().map(|(label, _)| label).collect()
    }
}

/// Source ports of a node, in drawing order.
pub fn source_ports(node: &CanvasNode, geom_w: f32) -> Vec<Port> {
    match node.kind.as_str() {
        "end" => Vec::new(),
        "condition" => vec![
            Port {
                handle: "true".into(),
                label: Some(t!("customNode.yes")),
                color: Some(0x52c41a),
                dx: geom_w - 15.0,
                dy: BRANCH_TOP + 18.0 * 0.3,
            },
            Port {
                handle: "false".into(),
                label: Some(t!("customNode.no")),
                color: Some(0xff4d4f),
                dx: geom_w - 15.0,
                dy: BRANCH_TOP + 22.0 + 18.0 * 0.7,
            },
        ],
        "switch" => switch_cases(&node.config)
            .into_iter()
            .enumerate()
            .map(|(i, label)| Port {
                handle: format!("case-{i}"),
                label: Some(label),
                color: Some(SWITCH_PALETTE[i % SWITCH_PALETTE.len()]),
                dx: geom_w - 15.0,
                dy: BRANCH_TOP + i as f32 * 22.0 + 9.0,
            })
            .collect(),
        _ => vec![Port {
            handle: "default".into(),
            label: None,
            color: None,
            dx: geom_w,
            dy: node_height(node) / 2.0,
        }],
    }
}

fn node_height(node: &CanvasNode) -> f32 {
    let base = BORDER * 2.0 + PAD_Y * 2.0 + HEADER_H;
    match node.kind.as_str() {
        "condition" => base + 4.0 + 18.0 * 2.0 + 4.0,
        "switch" => {
            let n = switch_cases(&node.config).len() as f32;
            base + 4.0 + n * 18.0 + (n - 1.0) * 4.0
        }
        _ => base,
    }
}

pub fn node_geom(node: &CanvasNode, with_status: bool) -> Geom {
    let entry = node_type_entry(&node.kind);
    let type_label = entry
        .map(|e| t!(e.label_key))
        .unwrap_or_else(|| node.kind.clone());
    let text = text_width(&node.name, 12.0).max(text_width(&type_label, 10.0)) * 1.04;
    let header = 8.0 + 6.0 + text + if with_status { 16.0 } else { 0.0 };
    let mut min_w: f32 = if is_pill(&node.kind) { 100.0 } else { 160.0 };
    min_w = min_w.max(header + PAD_X * 2.0 + BORDER + 3.0);
    if node.kind == "switch" {
        let longest = switch_cases(&node.config)
            .iter()
            .map(|c| text_width(c, 10.0).min(80.0))
            .fold(0.0, f32::max);
        min_w = min_w.max(longest + 6.0 + 14.0 + PAD_X * 2.0 + BORDER + 3.0 + 10.0);
    }
    Geom {
        x: node.x as f32,
        y: node.y as f32,
        w: min_w.ceil(),
        h: node_height(node),
    }
}

/// Where the connection point of the target port is (left edge, vertically centred).
pub fn target_point(g: &Geom) -> (f32, f32) {
    (g.x, g.y + g.h / 2.0)
}

/// Polyline of a "smooth step" edge from a source on the right of a node to a target on the left of another one
/// (the corners are rounded when it is drawn). Mirrors xyflow's `getSmoothStepPath` for right → left ports.
pub fn edge_route(src: (f32, f32), dst: (f32, f32)) -> Vec<(f32, f32)> {
    const OFFSET: f32 = 20.0;
    let ((sx, sy), (tx, ty)) = (src, dst);
    if sx + OFFSET <= tx - OFFSET || (sy - ty).abs() < 0.5 && sx < tx {
        let cx = (sx + tx) / 2.0;
        if (sy - ty).abs() < 0.5 {
            return vec![src, dst];
        }
        return vec![src, (cx, sy), (cx, ty), dst];
    }
    // target is left of / too close to the source: leave to the right, go around through the middle, enter from the left
    let cy = (sy + ty) / 2.0;
    vec![
        src,
        (sx + OFFSET, sy),
        (sx + OFFSET, cy),
        (tx - OFFSET, cy),
        (tx - OFFSET, ty),
        dst,
    ]
}

/// Distance from a point to the polyline.
pub fn distance_to_polyline(p: (f32, f32), pts: &[(f32, f32)]) -> f32 {
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 {
                0.0
            } else {
                (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
            };
            let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
            ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
        })
        .fold(f32::MAX, f32::min)
}

pub fn bounding_box(geoms: &[Geom]) -> Option<(f32, f32, f32, f32)> {
    if geoms.is_empty() {
        return None;
    }
    let min_x = geoms.iter().map(|g| g.x).fold(f32::MAX, f32::min);
    let min_y = geoms.iter().map(|g| g.y).fold(f32::MAX, f32::min);
    let max_x = geoms.iter().map(|g| g.x + g.w).fold(f32::MIN, f32::max);
    let max_y = geoms.iter().map(|g| g.y + g.h).fold(f32::MIN, f32::max);
    Some((min_x, min_y, max_x, max_y))
}

/// `fitView` (10% padding, zoom limited to the canvas' limits) for a canvas of `w × h` pixels.
pub fn fit_view(geoms: &[Geom], w: f32, h: f32) -> Viewport {
    let Some((x0, y0, x1, y1)) = bounding_box(geoms) else {
        return Viewport::default();
    };
    let (bw, bh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    let zoom = (w / (bw * 1.1))
        .min(h / (bh * 1.1))
        .clamp(MIN_ZOOM, MAX_ZOOM);
    Viewport {
        zoom,
        x: (w - bw * zoom) / 2.0 - x0 * zoom,
        y: (h - bh * zoom) / 2.0 - y0 * zoom,
    }
}

/// Stroke colour of an edge by branch handle.
pub fn edge_color(handle: Option<&str>) -> u32 {
    match handle {
        Some("true") => 0x52c41a,
        Some("false") => 0xff4d4f,
        _ => 0x8a90a0,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// state of an ongoing mouse interaction
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Drag {
    Pan {
        mouse: (f32, f32),
        view: (f32, f32),
    },
    /// Dragging one node, or all selected nodes (`origins` holds the start position of each).
    Node {
        id: String,
        mouse: (f32, f32),
        origins: Vec<(String, (f64, f64))>,
        moved: bool,
    },
    /// Shift + drag on the background: a selection rectangle (screen coordinates relative to the canvas).
    Marquee {
        start: (f32, f32),
        cursor: (f32, f32),
    },
    Connect {
        from: String,
        handle: String,
        cursor: (f32, f32),
        hover: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuTarget {
    Node(String),
    Edge(String),
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub at: (f32, f32),
    pub target: MenuTarget,
}

/// Look-up data for an edge: its id, route in flow units and colour.
struct RoutedEdge {
    id: String,
    handle: Option<String>,
    route: Vec<(f32, f32)>,
}

impl WorkflowEditor {
    fn geoms(
        &self,
        statuses: &std::collections::HashMap<String, NodeStatus>,
    ) -> Vec<(CanvasNode, Geom)> {
        self.graph()
            .nodes
            .iter()
            .map(|n| (n.clone(), node_geom(n, statuses.contains_key(&n.id))))
            .collect()
    }

    fn routed_edges(&self, geoms: &[(CanvasNode, Geom)]) -> Vec<RoutedEdge> {
        self.graph()
            .connections
            .iter()
            .filter_map(|c| {
                let (from, fg) = geoms.iter().find(|(n, _)| n.id == c.from_id)?;
                let (_, tg) = geoms.iter().find(|(n, _)| n.id == c.to_id)?;
                let ports = source_ports(from, fg.w);
                let port = c
                    .source_handle
                    .as_deref()
                    .and_then(|h| ports.iter().find(|p| p.handle == h))
                    .or_else(|| ports.first())?;
                let src = (fg.x + port.dx, fg.y + port.dy);
                Some(RoutedEdge {
                    id: c.id.clone(),
                    handle: c.source_handle.clone(),
                    route: edge_route(src, target_point(tg)),
                })
            })
            .collect()
    }

    /// Mouse position (window coordinates) relative to the canvas' top-left corner.
    fn rel(&self, p: Point<Pixels>) -> (f32, f32) {
        let b = self.canvas_bounds.get();
        (f32::from(p.x - b.origin.x), f32::from(p.y - b.origin.y))
    }

    fn edge_at(&self, screen: (f32, f32), cx: &App) -> Option<String> {
        let statuses = self.statuses(cx);
        let geoms = self.geoms(&statuses);
        let flow = self.view.to_flow(screen.0, screen.1);
        let reach = 8.0 / self.view.zoom;
        self.routed_edges(&geoms)
            .into_iter()
            .filter(|e| distance_to_polyline(flow, &e.route) <= reach)
            .min_by(|a, b| {
                distance_to_polyline(flow, &a.route)
                    .total_cmp(&distance_to_polyline(flow, &b.route))
            })
            .map(|e| e.id)
    }

    /// The node (other than `from`) whose left port is within reach of the screen point, or which contains it.
    fn connect_target_at(&self, from: &str, screen: (f32, f32), cx: &App) -> Option<String> {
        let statuses = self.statuses(cx);
        let flow = self.view.to_flow(screen.0, screen.1);
        let reach = 20.0 / self.view.zoom;
        self.geoms(&statuses)
            .into_iter()
            .filter(|(n, _)| n.id != from && has_target_port(&n.kind))
            .find_map(|(n, g)| {
                let (px_, py_) = target_point(&g);
                let near = ((flow.0 - px_).powi(2) + (flow.1 - py_).powi(2)).sqrt() <= reach;
                let inside =
                    flow.0 >= g.x && flow.0 <= g.x + g.w && flow.1 >= g.y && flow.1 <= g.y + g.h;
                (near || inside).then_some(n.id)
            })
    }

    // ---- view operations ----

    pub fn view(&self) -> Viewport {
        self.view
    }

    pub fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let b = self.canvas_bounds.get();
        let (cx_, cy_) = (
            f32::from(b.size.width) / 2.0,
            f32::from(b.size.height) / 2.0,
        );
        let zoom = self.view.zoom * factor;
        self.view.zoom_at(cx_, cy_, zoom);
        cx.notify();
    }

    pub fn fit(&mut self, cx: &mut Context<Self>) {
        let b = self.canvas_bounds.get();
        let statuses = self.statuses(cx);
        let geoms: Vec<Geom> = self.geoms(&statuses).into_iter().map(|(_, g)| g).collect();
        self.view = fit_view(&geoms, f32::from(b.size.width), f32::from(b.size.height));
        cx.notify();
    }

    /// Screen position (window coordinates) of a node's top-left corner (tests).
    pub fn node_screen_origin(&self, id: &str) -> Option<Point<Pixels>> {
        let n = self.graph().node(id)?;
        let b = self.canvas_bounds.get();
        let (sx, sy) = self.view.to_screen(n.x as f32, n.y as f32);
        Some(point(b.origin.x + px(sx), b.origin.y + px(sy)))
    }

    /// Move a node to a flow position (tests, and keyboard nudging later).
    pub fn set_node_position(&mut self, id: &str, x: f64, y: f64, cx: &mut Context<Self>) {
        self.graph_mut().set_position(id, x, y);
        cx.notify();
    }

    /// Window position of the middle of an edge (tests).
    pub fn edge_point(&self, edge_id: &str, cx: &App) -> Option<Point<Pixels>> {
        let statuses = self.statuses(cx);
        let geoms = self.geoms(&statuses);
        let edge = self
            .routed_edges(&geoms)
            .into_iter()
            .find(|e| e.id == edge_id)?;
        let total: f32 = edge
            .route
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
            .sum();
        let mut left = total / 2.0;
        for w in edge.route.windows(2) {
            let len = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            if left <= len || len == 0.0 {
                let t = if len == 0.0 { 0.0 } else { left / len };
                let (fx, fy) = (
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                );
                let (sx, sy) = self.view.to_screen(fx, fy);
                let b = self.canvas_bounds.get();
                return Some(point(b.origin.x + px(sx), b.origin.y + px(sy)));
            }
            left -= len;
        }
        None
    }

    pub fn drag_state(&self) -> Option<&Drag> {
        self.drag.as_ref()
    }

    pub fn selected_edge(&self) -> Option<&str> {
        self.selected_edge.as_deref()
    }

    pub fn menu_target(&self) -> Option<&MenuTarget> {
        self.menu.as_ref().map(|m| &m.target)
    }

    // ---- mouse ----

    pub(super) fn pane_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus, cx);
        self.menu = None;
        let at = self.rel(ev.position);
        let edge = self.edge_at(at, cx);
        match ev.button {
            MouseButton::Right => {
                if let Some(edge) = edge {
                    self.select_edge(Some(edge.clone()), window, cx);
                    self.menu = Some(Menu {
                        at,
                        target: MenuTarget::Edge(edge),
                    });
                }
            }
            _ => {
                if let Some(edge) = edge {
                    self.select_edge(Some(edge), window, cx);
                } else {
                    self.select_edge(None, window, cx);
                    if ev.modifiers.shift {
                        self.drag = Some(Drag::Marquee {
                            start: at,
                            cursor: at,
                        });
                    } else {
                        self.select(None, window, cx);
                        self.drag = Some(Drag::Pan {
                            mouse: at,
                            view: (self.view.x, self.view.y),
                        });
                    }
                }
            }
        }
        cx.notify();
    }

    pub(super) fn node_mouse_down(
        &mut self,
        id: &str,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.canvas_focus, cx);
        let at = self.rel(ev.position);
        self.menu = None;
        self.selected_edge = None;
        let additive = ev.modifiers.shift || ev.modifiers.control || ev.modifiers.platform;
        let in_group = self.multi.len() > 1 && self.multi.iter().any(|m| m == id);
        match ev.button {
            MouseButton::Right => {
                if !in_group {
                    self.select(Some(id.to_string()), window, cx);
                }
                self.menu = Some(Menu {
                    at,
                    target: MenuTarget::Node(id.to_string()),
                });
            }
            _ => {
                if additive {
                    // shift / ctrl / cmd + click toggles the node in the selection
                    let mut ids = self.multi.clone();
                    if let Some(pos) = ids.iter().position(|m| m == id) {
                        ids.remove(pos);
                    } else {
                        ids.push(id.to_string());
                    }
                    self.set_multi(ids, window, cx);
                    cx.notify();
                    return;
                }
                if !in_group && self.selected.as_deref() != Some(id) {
                    self.select(Some(id.to_string()), window, cx);
                }
                let moving: Vec<String> = if in_group {
                    self.multi.clone()
                } else {
                    vec![id.to_string()]
                };
                let origins = moving
                    .iter()
                    .filter_map(|m| self.graph().node(m).map(|n| (n.id.clone(), (n.x, n.y))))
                    .collect();
                self.drag = Some(Drag::Node {
                    id: id.to_string(),
                    mouse: at,
                    origins,
                    moved: false,
                });
            }
        }
        cx.notify();
    }

    pub(super) fn port_mouse_down(
        &mut self,
        id: &str,
        handle: &str,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if ev.button != MouseButton::Left {
            return;
        }
        window.focus(&self.canvas_focus, cx);
        self.menu = None;
        let at = self.rel(ev.position);
        self.drag = Some(Drag::Connect {
            from: id.to_string(),
            handle: handle.to_string(),
            cursor: at,
            hover: None,
        });
        cx.notify();
    }

    pub(super) fn canvas_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let at = self.rel(ev.position);
        let Some(drag) = self.drag.clone() else {
            return;
        };
        match drag {
            Drag::Pan { mouse, view } => {
                self.view.x = view.0 + at.0 - mouse.0;
                self.view.y = view.1 + at.1 - mouse.1;
            }
            Drag::Node {
                id, mouse, origins, ..
            } => {
                let z = self.view.zoom as f64;
                let (dx, dy) = ((at.0 - mouse.0) as f64 / z, (at.1 - mouse.1) as f64 / z);
                for (n, origin) in &origins {
                    self.graph_mut()
                        .set_position(n, origin.0 + dx, origin.1 + dy);
                }
                self.drag = Some(Drag::Node {
                    id,
                    mouse,
                    origins,
                    moved: dx != 0.0 || dy != 0.0,
                });
            }
            Drag::Marquee { start, .. } => {
                self.drag = Some(Drag::Marquee { start, cursor: at });
            }
            Drag::Connect { from, handle, .. } => {
                let hover = self.connect_target_at(&from, at, cx);
                self.drag = Some(Drag::Connect {
                    from,
                    handle,
                    cursor: at,
                    hover,
                });
            }
        }
        cx.notify();
    }

    pub(super) fn canvas_mouse_up(
        &mut self,
        ev: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = self.rel(ev.position);
        match self.drag.take() {
            Some(Drag::Connect { from, handle, .. }) => {
                if let Some(to) = self.connect_target_at(&from, at, cx) {
                    self.connect(&from, &to, Some(&handle), window, cx);
                }
            }
            Some(Drag::Marquee { start, cursor }) => {
                // every node touching the rectangle becomes selected
                let (x0, x1) = (start.0.min(cursor.0), start.0.max(cursor.0));
                let (y0, y1) = (start.1.min(cursor.1), start.1.max(cursor.1));
                let statuses = self.statuses(cx);
                let hit: Vec<String> = self
                    .geoms(&statuses)
                    .into_iter()
                    .filter(|(_, g)| {
                        let (sx0, sy0) = self.view.to_screen(g.x, g.y);
                        let (sx1, sy1) = self.view.to_screen(g.x + g.w, g.y + g.h);
                        sx0 <= x1 && sx1 >= x0 && sy0 <= y1 && sy1 >= y0
                    })
                    .map(|(n, _)| n.id)
                    .collect();
                self.set_multi(hit, window, cx);
            }
            _ => {}
        }
        self.drag = None;
        cx.notify();
    }

    pub(super) fn canvas_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let (dy, per_unit) = match ev.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.y), 0.002),
            ScrollDelta::Lines(l) => (l.y, 0.05),
        };
        if dy == 0.0 {
            return;
        }
        let at = self.rel(ev.position);
        // d3-zoom: scale *= 2 ^ (-deltaY * unit * (ctrl ? 10 : 1)); GPUI reports "wheel turned up" as positive
        let boost = if ev.modifiers.control || ev.modifiers.platform {
            10.0
        } else {
            1.0
        };
        let zoom = self.view.zoom * 2f32.powf(dy * per_unit * boost);
        self.view.zoom_at(at.0, at.1, zoom);
        cx.notify();
    }

    pub(super) fn canvas_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev.keystroke.key.as_str() {
            "backspace" | "delete" => {
                if let Some(edge) = self.selected_edge.clone() {
                    self.disconnect(&edge, cx);
                    self.selected_edge = None;
                } else if !self.multi.is_empty() {
                    for id in self.multi.clone() {
                        self.graph_mut().remove_node(&id);
                    }
                    self.select(None, window, cx);
                }
            }
            key @ ("left" | "right" | "up" | "down") if !self.multi.is_empty() => {
                // arrow keys move the selected nodes by 5 flow pixels (20 with shift)
                let step = if ev.keystroke.modifiers.shift {
                    20.0
                } else {
                    5.0
                };
                let (dx, dy) = match key {
                    "left" => (-step, 0.0),
                    "right" => (step, 0.0),
                    "up" => (0.0, -step),
                    _ => (0.0, step),
                };
                for id in self.multi.clone() {
                    if let Some(n) = self.graph().node(&id).cloned() {
                        self.graph_mut().set_position(&id, n.x + dx, n.y + dy);
                    }
                }
            }
            "escape" => {
                self.menu = None;
                self.selected_edge = None;
                self.select(None, window, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn run_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        match menu.target {
            MenuTarget::Node(id) => {
                self.select(Some(id), window, cx);
                self.delete_selected(window, cx);
            }
            MenuTarget::Edge(id) => {
                self.disconnect(&id, cx);
                self.selected_edge = None;
            }
        }
        cx.notify();
    }

    // ---- view ----

    pub(super) fn render_canvas(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = ui(cx);
        let _ = ();
        let statuses = self.statuses(cx);
        let geoms = self.geoms(&statuses);
        let edges = self.routed_edges(&geoms);
        let bounds_cell = self.canvas_bounds.clone();
        let entity = cx.entity();
        let selected_ids = self.multi.clone();
        let selected_edge = self.selected_edge.clone();
        let marquee = match &self.drag {
            Some(Drag::Marquee { start, cursor }) => Some((*start, *cursor)),
            _ => None,
        };
        let marquee_color = ui.primary;
        let hover_target = match &self.drag {
            Some(Drag::Connect { hover, .. }) => hover.clone(),
            _ => None,
        };
        let connecting_from = match &self.drag {
            Some(Drag::Connect {
                from,
                handle,
                cursor,
                ..
            }) => geoms
                .iter()
                .find(|(n, _)| &n.id == from)
                .and_then(|(n, g)| {
                    let ports = source_ports(n, g.w);
                    ports
                        .iter()
                        .find(|p| &p.handle == handle)
                        .map(|p| ((g.x + p.dx, g.y + p.dy), *cursor))
                }),
            _ => None,
        };

        // ---- painted layer: dotted grid, edges, the connection being drawn ----
        let view = self.view;
        let paint_edges: Vec<(Vec<(f32, f32)>, Hsla, bool)> = edges
            .iter()
            .map(|e| {
                let is_sel = selected_edge.as_deref() == Some(e.id.as_str());
                let color: Hsla = if is_sel {
                    rgb(0x555555).into()
                } else {
                    rgb(edge_color(e.handle.as_deref())).into()
                };
                (e.route.clone(), color, is_sel)
            })
            .collect();
        let arrow_colors: Vec<Hsla> = edges
            .iter()
            .map(|e| match e.handle.as_deref() {
                Some("true") => rgb(0x52c41a).into(),
                Some("false") => rgb(0xff4d4f).into(),
                _ => rgb(0x999999).into(),
            })
            .collect();
        let dot_color: Hsla = rgb(0xd9d9d9).into();

        let preview = connecting_from;
        let background = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                // dots
                let gap = DOT_GAP * view.zoom;
                if gap >= 7.0 {
                    let r = view.zoom.max(1.0);
                    let start_x = view.x.rem_euclid(gap);
                    let start_y = view.y.rem_euclid(gap);
                    let mut y = start_y;
                    while y < h {
                        let mut x = start_x;
                        while x < w {
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        point(px(ox + x - r / 2.0), px(oy + y - r / 2.0)),
                                        size(px(r), px(r)),
                                    ),
                                    dot_color,
                                )
                                .corner_radii(px(r / 2.0)),
                            );
                            x += gap;
                        }
                        y += gap;
                    }
                }
                // edges
                for (i, (route, color, selected)) in paint_edges.iter().enumerate() {
                    let pts: Vec<Point<Pixels>> = route
                        .iter()
                        .map(|&(fx, fy)| {
                            let (sx, sy) = view.to_screen(fx, fy);
                            point(px(ox + sx), px(oy + sy))
                        })
                        .collect();
                    stroke_rounded(
                        window,
                        &pts,
                        6.0 * view.zoom,
                        *color,
                        if *selected { 2.0 } else { 1.5 },
                    );
                    if let [.., a, b] = pts.as_slice() {
                        arrow(window, *a, *b, arrow_colors[i], view.zoom);
                    }
                }
                // selection rectangle
                if let Some((a, b)) = marquee {
                    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                    let rect = Bounds::new(
                        point(px(ox + x0), px(oy + y0)),
                        size(px(x1 - x0), px(y1 - y0)),
                    );
                    window.paint_quad(fill(rect, marquee_color.opacity(0.08)));
                    window.paint_quad(outline(
                        rect,
                        marquee_color.opacity(0.6),
                        BorderStyle::Solid,
                    ));
                }
                // connection being drawn: a plain bezier from the port to the cursor
                if let Some(((fx, fy), cursor)) = preview {
                    let (sx, sy) = view.to_screen(fx, fy);
                    let (a, b) = (
                        point(px(ox + sx), px(oy + sy)),
                        point(px(ox + cursor.0), px(oy + cursor.1)),
                    );
                    let dx = (cursor.0 - sx).abs().max(40.0) * 0.5;
                    let mut pb = PathBuilder::stroke(px(1.5));
                    pb.move_to(a);
                    pb.cubic_bezier_to(b, point(a.x + px(dx), a.y), point(b.x - px(dx), b.y));
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, rgb(0xb1b1b7));
                    }
                }
            },
        )
        .absolute()
        .size_full();

        // ---- nodes ----
        let mut nodes: Vec<(CanvasNode, Geom)> = geoms.clone();
        // the selected node is raised above the others
        nodes.sort_by_key(|(n, _)| selected_ids.iter().any(|s| s == &n.id));
        let node_views = nodes
            .into_iter()
            .map(|(node, g)| {
                let status = statuses.get(&node.id).copied();
                self.node_view(
                    &node,
                    g,
                    status,
                    selected_ids.iter().any(|s| s == &node.id),
                    hover_target.as_deref() == Some(node.id.as_str()),
                    cx,
                )
            })
            .collect::<Vec<_>>();

        let empty = self.graph().nodes.is_empty();
        let menu = self.menu.clone();
        let (zoom_in, zoom_out, fit) = (
            Self::control_button(
                "wf-zoom-in",
                IconName::Plus,
                cx.listener(|this, _, _, cx| this.zoom_by(1.2, cx)),
                cx,
            ),
            Self::control_button(
                "wf-zoom-out",
                IconName::Minus,
                cx.listener(|this, _, _, cx| this.zoom_by(1.0 / 1.2, cx)),
                cx,
            ),
            Self::control_button(
                "wf-fit",
                IconName::Maximize,
                cx.listener(|this, _, _, cx| this.fit(cx)),
                cx,
            ),
        );

        // ---- mini map ----
        let mini_nodes: Vec<(Geom, u32)> = geoms
            .iter()
            .map(|(n, g)| {
                (
                    *g,
                    node_type_entry(&n.kind)
                        .map(|e| e.color)
                        .unwrap_or(0xd9d9d9),
                )
            })
            .collect();
        let canvas_size = {
            let b = self.canvas_bounds.get();
            (f32::from(b.size.width), f32::from(b.size.height))
        };
        let mini_bg = crate::widgets::form::mix(ui.card, ui.main, 0.08);
        let mini_mask = ui.card.opacity(0.68);
        let mini_stroke = ui.border_strong;
        let minimap = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (mw, mh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                // view rectangle in flow units
                let (vx0, vy0) = view.to_flow(0.0, 0.0);
                let (vx1, vy1) = view.to_flow(canvas_size.0, canvas_size.1);
                let (mut x0, mut y0, mut x1, mut y1) = (vx0, vy0, vx1, vy1);
                for (g, _) in &mini_nodes {
                    x0 = x0.min(g.x);
                    y0 = y0.min(g.y);
                    x1 = x1.max(g.x + g.w);
                    y1 = y1.max(g.y + g.h);
                }
                let (bw, bh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
                // xyflow pads the bounding box by 2 * offsetScale of the view box
                let scale = (bw / mw).max(bh / mh);
                let map = |fx: f32, fy: f32| {
                    (
                        ox + (mw - bw / scale) / 2.0 + (fx - x0) / scale,
                        oy + (mh - bh / scale) / 2.0 + (fy - y0) / scale,
                    )
                };
                for (g, color) in &mini_nodes {
                    let (sx, sy) = map(g.x, g.y);
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(px(sx), px(sy)),
                                size(px(g.w / scale), px(g.h / scale)),
                            ),
                            rgb(*color),
                        )
                        .corner_radii(px(2.0)),
                    );
                }
                // mask everything outside the visible area
                let (a0, b0) = map(vx0, vy0);
                let (a1, b1) = map(vx1, vy1);
                let (left, top, right, bottom) = (ox, oy, ox + mw, oy + mh);
                let (o0, p0, o1, p1) = (a0, b0, a1, b1);
                let (a0, b0, a1, b1) = (
                    a0.clamp(left, right),
                    b0.clamp(top, bottom),
                    a1.clamp(left, right),
                    b1.clamp(top, bottom),
                );
                let rects = [
                    (left, top, right - left, b0 - top),
                    (left, b1, right - left, bottom - b1),
                    (left, b0, a0 - left, b1 - b0),
                    (a1, b0, right - a1, b1 - b0),
                ];
                for (rx, ry, rw, rh) in rects {
                    if rw > 0.0 && rh > 0.0 {
                        window.paint_quad(fill(
                            Bounds::new(point(px(rx), px(ry)), size(px(rw), px(rh))),
                            mini_mask,
                        ));
                    }
                }
                let _ = (o0, p0, o1, p1, mini_stroke);
            },
        )
        .size_full();

        div()
            .id("wf-canvas")
            .test_support()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(ui.main)
            .track_focus(&self.canvas_focus)
            .key_context("WorkflowCanvas")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.pane_mouse_down(ev, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.pane_mouse_down(ev, window, cx)
                }),
            )
            .on_mouse_move(
                cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.canvas_mouse_move(ev, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.canvas_mouse_up(ev, window, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.canvas_mouse_up(ev, window, cx)
                }),
            )
            .on_scroll_wheel(
                cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.canvas_wheel(ev, cx)),
            )
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.canvas_key(ev, window, cx)),
            )
            // remember where the canvas is (mouse positions are window based) and re-render once when it first gets a size
            .child(
                canvas(
                    move |bounds, window, _| {
                        let old = bounds_cell.get();
                        if old != bounds {
                            bounds_cell.set(bounds);
                            let entity = entity.clone();
                            window
                                .on_next_frame(move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(background)
            .children(node_views)
            .when(empty, |d| {
                d.child(
                    v_flex()
                        .id("wf-canvas-empty")
                        .test_support()
                        .absolute()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .text_color(ui.text_muted)
                        .child(div().text_size(px(36.)).mb(px(8.)).opacity(0.4).child("+"))
                        .child(div().child(t!("wfEditor.canvasEmpty")))
                        .child(
                            div()
                                .text_size(px(12.))
                                .mt(px(4.))
                                .child(t!("wfEditor.canvasEmptyHint")),
                        ),
                )
            })
            // zoom controls (bottom left) and mini map (bottom right), 15px from the edges like xyflow's panels
            .child(
                v_flex()
                    .absolute()
                    .left(px(15.))
                    .bottom(px(15.))
                    .overflow_hidden()
                    .bg(ui.card)
                    .border_1()
                    .border_color(ui.border_strong)
                    .rounded(px(8.))
                    .child(zoom_in)
                    .child(zoom_out)
                    .child(fit),
            )
            .child(
                div()
                    .id("wf-minimap")
                    .test_support()
                    .absolute()
                    .right(px(15.))
                    .bottom(px(15.))
                    .w(px(200.))
                    .h(px(150.))
                    .overflow_hidden()
                    .bg(mini_bg)
                    .border_1()
                    .border_color(ui.border_strong)
                    .rounded(px(8.))
                    .child(minimap),
            )
            .when_some(menu, |d, menu| {
                let label = match menu.target {
                    MenuTarget::Node(_) => t!("wfEditor.deleteNode"),
                    MenuTarget::Edge(_) => t!("wfEditor.deleteConnection"),
                };
                d.child(
                    v_flex()
                        .id("wf-context-menu")
                        .test_support()
                        .absolute()
                        .left(px(menu.at.0))
                        .top(px(menu.at.1))
                        .min_w(px(120.))
                        .py(px(4.))
                        .bg(ui.card)
                        .border_1()
                        .border_color(ui.border)
                        .rounded(px(8.))
                        .shadow(vec![BoxShadow {
                            color: gpui_kit::rgba(0x0000001f).into(),
                            offset: point(px(0.), px(4.)),
                            blur_radius: px(12.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .occlude()
                        .child(
                            div()
                                .id("wf-context-delete")
                                .test_support()
                                .px(px(12.))
                                .py(px(6.))
                                .text_size(px(13.))
                                .text_color(rgb(0xff4d4f))
                                .cursor_pointer()
                                .hover(|d| d.bg(rgba(0xff4d4f14)))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.run_menu(window, cx)),
                                )
                                .child(label),
                        ),
                )
            })
    }

    fn control_button(
        id: &'static str,
        icon: IconName,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> impl IntoElement {
        let ui = ui(cx);
        div()
            .id(id)
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(ui.text_secondary)
            .cursor_pointer()
            .border_b_1()
            .border_color(ui.border)
            .hover(move |d| {
                d.bg(crate::widgets::form::mix(ui.card, ui.primary, 0.08))
                    .text_color(ui.primary)
            })
            .on_click(on_click)
            .child(Icon::new(icon).size(px(12.)))
            .test_support()
    }

    /// One node (`WfNode`): card with a coloured left edge, dot + name + type, status dot and its ports.
    fn node_view(
        &self,
        node: &CanvasNode,
        g: Geom,
        status: Option<NodeStatus>,
        selected: bool,
        hovered_target: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = ui(cx);
        let z = self.view.zoom;
        let entry = node_type_entry(&node.kind);
        let color: Hsla = rgb(entry.map(|e| e.color).unwrap_or(0xd9d9d9)).into();
        let pill = is_pill(&node.kind);
        let (sx, sy) = self.view.to_screen(g.x, g.y);
        let border_color: Hsla = match status {
            Some(NodeStatus::Running) => rgb(0x1677ff).into(),
            Some(NodeStatus::Success) => rgb(0x52c41a).into(),
            Some(NodeStatus::Fail) => rgb(0xff4d4f).into(),
            None if selected => ui.primary,
            None => ui.border,
        };
        let id = node.id.clone();
        let (down_id, port_id) = (id.clone(), id.clone());
        let ports = source_ports(node, g.w);
        let type_label = entry
            .map(|e| t!(e.label_key))
            .unwrap_or_else(|| node.kind.clone());
        let radius = if pill { 20.0 } else { 8.0 };
        let name = node.name.clone();
        let has_target = has_target_port(&node.kind);
        let enabled = node.enabled;
        let host = node.kind == "selectHost";

        let port_dot = |key: String,
                        handle: Option<String>,
                        dx: f32,
                        dy: f32,
                        ring: Hsla,
                        highlight: bool,
                        cx: &mut Context<Self>| {
            let size_px = 10.0 * z;
            let hit = 18.0 * z;
            let mut el = div()
                .id(ElementId::Name(key.into()))
                .absolute()
                .left(px((dx - 9.0) * z - BORDER * z))
                .top(px((dy - 9.0) * z - BORDER * z))
                .size(px(hit))
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::Crosshair)
                .child(
                    div()
                        .size(px(size_px))
                        .rounded_full()
                        .border(px(2.0 * z))
                        .border_color(ring)
                        .bg(if highlight { ui.primary } else { ui.card }),
                );
            if let Some(handle) = handle {
                let (nid, h) = (port_id.clone(), handle);
                el = el.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        this.port_mouse_down(&nid, &h, ev, window, cx)
                    }),
                );
            }
            el.test_support()
        };

        let mut children: Vec<AnyElement> = Vec::new();
        // coloured left edge (3px instead of the 2px border)
        if pill {
            // start / end: a pill whose left edge is a 3px arc in the node colour (CSS `border-left` on a rounded box)
            let (w_px, h_px, r_px) = (g.w * z, g.h * z, (radius * z).min(g.h * z / 2.0));
            let (fill_c, edge_c, accent_c) = (ui.card, border_color, color);
            children.push(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        paint_pill(
                            window,
                            f32::from(bounds.origin.x),
                            f32::from(bounds.origin.y),
                            w_px,
                            h_px,
                            r_px,
                            2.0 * z,
                            3.0 * z,
                            fill_c,
                            edge_c,
                            accent_c,
                        )
                    },
                )
                .absolute()
                .left(px(-BORDER * z))
                .top(px(-BORDER * z))
                .w(px(w_px))
                .h(px(h_px))
                .into_any_element(),
            );
        } else {
            // coloured left edge (3px instead of the 2px border)
            children.push(
                div()
                    .absolute()
                    .left(px(-BORDER * z))
                    .top(px(-BORDER * z))
                    .w(px(3.0 * z))
                    .h(px(g.h * z))
                    .rounded_tl(px(radius * z))
                    .rounded_bl(px(radius * z))
                    .bg(color)
                    .into_any_element(),
            );
        }
        // header: dot, title, type, status
        let status_dot = status.map(|s| {
            let c: Hsla = match s {
                NodeStatus::Running => rgb(0x1677ff).into(),
                NodeStatus::Success => rgb(0x52c41a).into(),
                NodeStatus::Fail => rgb(0xff4d4f).into(),
            };
            div()
                .id(ElementId::Name(format!("wf-status-{id}").into()))
                .test_support()
                .flex_none()
                .size(px(10.0 * z))
                .rounded_full()
                .bg(c)
        });
        children.push(
            h_flex()
                .gap(px(6.0 * z))
                .items_center()
                .when(pill, |d| d.justify_center())
                .h(px(HEADER_H * z))
                .child(div().flex_none().size(px(8.0 * z)).rounded_full().bg(color))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .truncate()
                                .when(pill, |d| d.text_center())
                                .text_size(px(12.0 * z))
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_height(px(18.0 * z))
                                .text_color(ui.text)
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .when(pill, |d| d.text_center())
                                .text_size(px(10.0 * z))
                                .line_height(px(15.0 * z))
                                .text_color(ui.text_muted)
                                .child(type_label),
                        ),
                )
                .children(status_dot)
                .into_any_element(),
        );
        // branch labels
        let labelled: Vec<&Port> = ports.iter().filter(|p| p.label.is_some()).collect();
        if !labelled.is_empty() {
            let (row_h, gap) = (18.0, 4.0);
            children.push(
                v_flex()
                    .mt(px(4.0 * z))
                    .gap(px(gap * z))
                    .children(labelled.iter().map(|p| {
                        let c: Hsla = rgb(p.color.unwrap_or(0x8c8c8c)).into();
                        h_flex().h(px(row_h * z)).items_center().child(
                            div()
                                .text_size(px(10.0 * z))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c)
                                .max_w(px(80.0 * z))
                                .truncate()
                                .child(p.label.clone().unwrap_or_default()),
                        )
                    }))
                    .into_any_element(),
            );
        }
        // ports
        if has_target {
            children.push(
                port_dot(
                    format!("wf-in-{id}"),
                    None,
                    0.0,
                    g.h / 2.0,
                    if hovered_target {
                        ui.primary
                    } else {
                        ui.border_strong
                    },
                    hovered_target,
                    cx,
                )
                .into_any_element(),
            );
        }
        for p in &ports {
            let ring: Hsla = p.color.map(|c| rgb(c).into()).unwrap_or(ui.border_strong);
            let key = if p.handle == "default" {
                format!("wf-out-{id}")
            } else {
                format!("wf-out-{id}-{}", p.handle)
            };
            children.push(
                port_dot(key, Some(p.handle.clone()), p.dx, p.dy, ring, false, cx)
                    .into_any_element(),
            );
        }

        let mut shell = div()
            .id(ElementId::Name(format!("wf-node-{id}").into()))
            .test_support()
            .absolute()
            .left(px(sx))
            .top(px(sy))
            .w(px(g.w * z))
            .h(px(g.h * z))
            .px(px((PAD_X - 0.0) * z))
            .py(px(PAD_Y * z))
            .rounded(px(radius * z))
            .border(px(BORDER * z))
            .border_color(if pill {
                gpui_kit::transparent_black()
            } else {
                border_color
            })
            .when(!pill, |d| d.bg(ui.card))
            .cursor(CursorStyle::PointingHand)
            .shadow(vec![BoxShadow {
                color: if selected {
                    gpui_kit::rgba(0x1677ff4d).into()
                } else {
                    gpui_kit::rgba(0x00000014).into()
                },
                offset: point(px(0.), px(if selected { 2.0 } else { 1.0 } * z)),
                blur_radius: px(if selected { 8.0 } else { 4.0 } * z),
                spread_radius: px(0.),
                inset: false,
            }])
            .when(host, |d| d.border_dashed())
            .when(!enabled, |d| d.opacity(0.5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.node_mouse_down(&down_id, ev, window, cx)
                }),
            );
        let right_id = id.clone();
        shell = shell.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                this.node_mouse_down(&right_id, ev, window, cx)
            }),
        );
        let el = shell.children(children);
        if matches!(status, Some(NodeStatus::Running)) {
            // `@keyframes wf-pulse`: a 1.5s blue ring growing 8px while fading out
            el.with_animation(
                ElementId::Name(format!("wf-pulse-{id}").into()),
                Animation::new(std::time::Duration::from_millis(1500)).repeat(),
                move |el, delta| {
                    el.shadow(vec![BoxShadow {
                        color: Hsla::from(rgb(0x1677ff)).opacity(0.4 * (1.0 - delta)),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(8.0 * delta * z),
                        inset: false,
                    }])
                },
            )
            .into_any_element()
        } else {
            el.into_any_element()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// drawing helpers
// ---------------------------------------------------------------------------------------------------------------

/// Stroke a polyline with rounded corners (quadratic bends through each interior point).
fn stroke_rounded(
    window: &mut Window,
    pts: &[Point<Pixels>],
    radius: f32,
    color: Hsla,
    width: f32,
) {
    if pts.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::stroke(px(width));
    pb.move_to(pts[0]);
    for i in 1..pts.len() - 1 {
        let (prev, cur, next) = (pts[i - 1], pts[i], pts[i + 1]);
        let (d1, d2) = (dist(prev, cur), dist(cur, next));
        let r = radius.min(d1 / 2.0).min(d2 / 2.0);
        if r <= 0.1 {
            pb.line_to(cur);
            continue;
        }
        let a = toward(cur, prev, r);
        let b = toward(cur, next, r);
        pb.line_to(a);
        pb.curve_to(b, cur);
    }
    pb.line_to(pts[pts.len() - 1]);
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn dist(a: Point<Pixels>, b: Point<Pixels>) -> f32 {
    ((f32::from(a.x) - f32::from(b.x)).powi(2) + (f32::from(a.y) - f32::from(b.y)).powi(2)).sqrt()
}

/// The point `r` pixels from `from` on the way to `to`.
fn toward(from: Point<Pixels>, to: Point<Pixels>, r: f32) -> Point<Pixels> {
    let d = dist(from, to).max(0.001);
    point(
        px(f32::from(from.x) + (f32::from(to.x) - f32::from(from.x)) * r / d),
        px(f32::from(from.y) + (f32::from(to.y) - f32::from(from.y)) * r / d),
    )
}

/// Closed arrow head (`MarkerType.ArrowClosed`) at `tip`, pointing along `from → tip`.
fn arrow(window: &mut Window, from: Point<Pixels>, tip: Point<Pixels>, color: Hsla, zoom: f32) {
    let d = dist(from, tip).max(0.001);
    let (ux, uy) = (
        (f32::from(tip.x) - f32::from(from.x)) / d,
        (f32::from(tip.y) - f32::from(from.y)) / d,
    );
    let len = 10.0 * zoom;
    let half = 5.0 * zoom;
    let (bx, by) = (f32::from(tip.x) - ux * len, f32::from(tip.y) - uy * len);
    let (nx, ny) = (-uy, ux);
    let mut pb = PathBuilder::fill();
    pb.move_to(tip);
    pb.line_to(point(px(bx + nx * half), px(by + ny * half)));
    pb.line_to(point(px(bx - nx * half), px(by - ny * half)));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn pt(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

/// Closed rounded rectangle (cubic corners).
fn rounded_rect_path(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
    const K: f32 = 0.5523;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = K * r;
    pb.move_to(pt(x + r, y));
    pb.line_to(pt(x + w - r, y));
    pb.cubic_bezier_to(pt(x + w, y + r), pt(x + w - r + k, y), pt(x + w, y + r - k));
    pb.line_to(pt(x + w, y + h - r));
    pb.cubic_bezier_to(
        pt(x + w - r, y + h),
        pt(x + w, y + h - r + k),
        pt(x + w - r + k, y + h),
    );
    pb.line_to(pt(x + r, y + h));
    pb.cubic_bezier_to(pt(x, y + h - r), pt(x + r - k, y + h), pt(x, y + h - r + k));
    pb.line_to(pt(x, y + r));
    pb.cubic_bezier_to(pt(x + r, y), pt(x, y + r - k), pt(x + r - k, y));
    pb.close();
}

/// Pill-shaped node background: fill, a `border` wide outline and, on the left, an `accent` wide arc that runs from
/// the middle of the top-left corner to the middle of the bottom-left corner.
#[allow(clippy::too_many_arguments)]
fn paint_pill(
    window: &mut Window,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    border: f32,
    accent: f32,
    fill_c: Hsla,
    border_c: Hsla,
    accent_c: Hsla,
) {
    let mut pb = PathBuilder::fill();
    rounded_rect_path(&mut pb, x, y, w, h, r);
    if let Ok(path) = pb.build() {
        window.paint_path(path, fill_c);
    }
    let i = border / 2.0;
    let mut pb = PathBuilder::stroke(px(border));
    rounded_rect_path(&mut pb, x + i, y + i, w - border, h - border, r - i);
    if let Ok(path) = pb.build() {
        window.paint_path(path, border_c);
    }
    // accent arc: second half of the top-left corner, the straight left side, first half of the bottom-left corner
    const K: f32 = 0.5523;
    let i = accent / 2.0;
    let (x, y, w, h) = (x + i, y + i, w - accent, h - accent);
    let _ = w;
    let r = (r - i).min(h / 2.0).max(0.0);
    let k = K * r;
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    // top-left corner: P0 = (x + r, y) .. P3 = (x, y + r)
    let (p0, p1, p2, p3) = ((x + r, y), (x + r - k, y), (x, y + r - k), (x, y + r));
    let (q0, q1, q2) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
    let (r0, r1) = (mid(q0, q1), mid(q1, q2));
    let s = mid(r0, r1);
    // bottom-left corner: P0 = (x, y + h - r) .. P3 = (x + r, y + h)
    let (b0, b1, b2, b3) = (
        (x, y + h - r),
        (x, y + h - r + k),
        (x + r - k, y + h),
        (x + r, y + h),
    );
    let (c0, c1, c2) = (mid(b0, b1), mid(b1, b2), mid(b2, b3));
    let (d0, d1) = (mid(c0, c1), mid(c1, c2));
    let t = mid(d0, d1);
    let mut pb = PathBuilder::stroke(px(accent));
    pb.move_to(pt(s.0, s.1));
    pb.cubic_bezier_to(pt(p3.0, p3.1), pt(r1.0, r1.1), pt(q2.0, q2.1));
    pb.line_to(pt(b0.0, b0.1));
    pb.cubic_bezier_to(pt(t.0, t.1), pt(c0.0, c0.1), pt(d0.0, d0.1));
    if let Ok(path) = pb.build() {
        window.paint_path(path, accent_c);
    }
}

pub fn edge_id(from: &str, to: &str, handle: &str) -> String {
    format!("edge-{from}-{to}-{handle}")
}

#[cfg(test)]
mod tests {
    use super::{
        bounding_box, distance_to_polyline, edge_route, fit_view, has_source_port, has_target_port,
        node_geom, source_ports, Geom, Viewport, MAX_ZOOM, MIN_ZOOM,
    };
    use ops_workflow::model::CanvasNode;

    fn node(kind: &str, name: &str, x: f64, y: f64) -> CanvasNode {
        let mut n = CanvasNode::new("n", kind, name, "");
        n.x = x;
        n.y = y;
        n
    }

    #[test]
    fn viewport_zoom_keeps_the_point_under_the_cursor() {
        let mut v = Viewport {
            x: 30.0,
            y: -20.0,
            zoom: 1.0,
        };
        let before = v.to_flow(200.0, 150.0);
        v.zoom_at(200.0, 150.0, 1.5);
        let after = v.to_flow(200.0, 150.0);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        v.zoom_at(0.0, 0.0, 10.0);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_at(0.0, 0.0, 0.01);
        assert_eq!(v.zoom, MIN_ZOOM);
    }

    #[test]
    fn nodes_have_the_original_sizes_and_ports() {
        let g = node_geom(&node("command", "ls", 10.0, 20.0), false);
        assert_eq!((g.w, g.h), (160.0, 53.0));
        assert_eq!(node_geom(&node("start", "开始", 0.0, 0.0), false).w, 100.0);
        // long names widen the node instead of being cut
        assert!(
            node_geom(
                &node("command", &"very long node name ".repeat(3), 0.0, 0.0),
                false
            )
            .w > 160.0
        );
        let cond = node("condition", "是否成功", 0.0, 0.0);
        let g = node_geom(&cond, false);
        assert_eq!(g.h, 53.0 + 4.0 + 36.0 + 4.0);
        let ports = source_ports(&cond, g.w);
        assert_eq!(
            ports.iter().map(|p| p.handle.as_str()).collect::<Vec<_>>(),
            ["true", "false"]
        );
        assert!(ports[0].dy < ports[1].dy);
        assert_eq!(source_ports(&{ let mut n = node("switch", "s", 0.0, 0.0); n.config = r#"{"expression":"","cases":[{"label":"a","value":"1"},{"label":"b","value":"*"}]}"#.into(); n }, 160.0).iter().map(|p| p.dy).collect::<Vec<_>>().windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>(), vec![22.0], "switch rows are 18px + 4px gap");
        let mut sw = node("switch", "分支", 0.0, 0.0);
        sw.config = r#"{"expression":"","cases":[{"label":"a","value":"1"},{"label":"b","value":"2"},{"label":"c","value":"*"}]}"#.into();
        assert_eq!(
            source_ports(&sw, 160.0)
                .iter()
                .map(|p| p.handle.clone())
                .collect::<Vec<_>>(),
            ["case-0", "case-1", "case-2"]
        );
        assert!(source_ports(&node("end", "结束", 0.0, 0.0), 100.0).is_empty());
        assert!(!has_target_port("start") && has_target_port("command") && !has_source_port("end"));
        let plain = source_ports(&node("command", "x", 0.0, 0.0), 160.0);
        assert_eq!(
            (plain[0].handle.as_str(), plain[0].dx, plain[0].dy),
            ("default", 160.0, 26.5)
        );
    }

    #[test]
    fn edges_route_like_a_smooth_step_path() {
        // target to the right: horizontal, vertical in the middle, horizontal
        assert_eq!(
            edge_route((0.0, 0.0), (100.0, 40.0)),
            vec![(0.0, 0.0), (50.0, 0.0), (50.0, 40.0), (100.0, 40.0)]
        );
        // same height: a straight line
        assert_eq!(
            edge_route((0.0, 10.0), (100.0, 10.0)),
            vec![(0.0, 10.0), (100.0, 10.0)]
        );
        // target behind the source: goes around
        let back = edge_route((200.0, 0.0), (100.0, 60.0));
        assert_eq!(back.len(), 6);
        assert_eq!(back[1], (220.0, 0.0));
        assert_eq!(back[4], (80.0, 60.0));
    }

    #[test]
    fn polyline_hit_testing_and_fit_view() {
        let pts = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
        assert!((distance_to_polyline((50.0, 5.0), &pts) - 5.0).abs() < 1e-4);
        assert!((distance_to_polyline((110.0, 50.0), &pts) - 10.0).abs() < 1e-4);
        let geoms = [
            Geom {
                x: 0.0,
                y: 0.0,
                w: 160.0,
                h: 53.0,
            },
            Geom {
                x: 400.0,
                y: 300.0,
                w: 160.0,
                h: 53.0,
            },
        ];
        let v = fit_view(&geoms, 800.0, 600.0);
        let (x0, y0) = v.to_screen(0.0, 0.0);
        let (x1, y1) = v.to_screen(560.0, 353.0);
        assert!(
            x0 > 0.0 && y0 > 0.0 && x1 < 800.0 && y1 < 600.0,
            "everything is inside the canvas"
        );
        assert!(
            ((x0 + x1) / 2.0 - 400.0).abs() < 1.0 && ((y0 + y1) / 2.0 - 300.0).abs() < 1.0,
            "and centred"
        );
        // a single small node does not zoom beyond the limit
        assert_eq!(fit_view(&geoms[..1], 800.0, 600.0).zoom, MAX_ZOOM);
        assert_eq!(fit_view(&[], 800.0, 600.0), Viewport::default());
        assert!(bounding_box(&[]).is_none());
    }
}
