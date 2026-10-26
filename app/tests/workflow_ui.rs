//! Workflow page and editor through the real window, with the executor running against a private sshd.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use std::time::Duration;

use gpui_kit::test::{TestEventExt as _, TestWindowExt};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use ops_ssh::hosts::NewHost;
use ops_workflow::model::{CanvasNode, Connection, WorkflowSettings, WorkflowStatus};
use ops_workflow::store as db;
use opsbatch_app::workflow::editor::WorkflowEditor;
use opsbatch_app::workflow::page::{PageTab, WorkflowPage};
use opsbatch_app::workflow::store::workflow_store;

use common::{serial, wait_until, Fixture};

fn win<R>(cx: &mut TestAppContext, fx: &Fixture, f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R) -> R {
    cx.update_window(fx.handle.into(), |_, w, cx| f(w, cx)).unwrap()
}

fn eid(s: String) -> gpui_kit::ElementId {
    gpui_kit::ElementId::Name(s.into())
}

fn open_page(cx: &mut TestAppContext, fx: &Fixture) -> Entity<WorkflowPage> {
    win(cx, fx, |w, cx| {
        w.click("mode-workflow", cx);
        w.render_frame(cx);
    });
    cx.update(|cx| fx.workbench.read(cx).workflow_page().expect("workflow page").clone())
}

fn workflows(fx: &Fixture) -> Vec<ops_workflow::model::WorkflowRecord> {
    let ctx = fx.ctx.clone();
    db::list_workflows(&ctx.db).unwrap()
}

fn node(id: &str, kind: &str, name: &str, config: &str) -> CanvasNode {
    CanvasNode::new(id, kind, name, config)
}

fn edge(from: &str, to: &str, handle: Option<&str>) -> Connection {
    Connection { id: format!("{from}-{to}"), from_id: from.into(), to_id: to.into(), source_handle: handle.map(str::to_string) }
}

fn seed_workflow(fx: &Fixture, name: &str, nodes: Vec<CanvasNode>, connections: Vec<Connection>) -> String {
    let mut wf = db::create_workflow(&fx.ctx.db, name, "desc").unwrap();
    wf.nodes = nodes;
    wf.connections = connections;
    wf.status = WorkflowStatus::Ready;
    db::update_workflow(&fx.ctx.db, &wf).unwrap();
    wf.id
}

fn log_text(cx: &mut TestAppContext, page: &Entity<WorkflowPage>) -> String {
    let run = page.read_with(cx, |p, _| p.run().clone());
    run.read_with(cx, |r, _| r.logs.iter().map(|l| format!("{}|{}", l.node_name.clone().unwrap_or_default(), l.message)).collect::<Vec<_>>().join("\n"))
}

fn add_ssh_host(core: &ops_core::Core, port: i32, user: &str, key: &str) -> String {
    ops_core::rt::block_on(ops_ssh::hosts::add_host(
        &core.db,
        NewHost {
            name: "ssh-box".into(),
            ip: "127.0.0.1".into(),
            port: Some(port),
            auth_type: Some("key".into()),
            username: Some(user.into()),
            password: None,
            private_key: Some(key.into()),
            os: Some("linux".into()),
            tags: None,
            group_id: None,
            remark: None,
            jump_chain: None,
            rdp_settings: None,
            proxy_settings: None,
        },
    ))
    .unwrap()
    .id
}

// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn the_page_starts_empty_and_creating_a_workflow_opens_the_editor(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let page = open_page(cx, &fx);
    wait_until(cx, "loaded", |cx| cx.update(|cx| !workflow_store(cx).read(cx).loading));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-empty").is_some(), "empty card");
        assert!(w.try_find("wf-tab-list").is_some() && w.try_find("wf-tab-templates").is_some() && w.try_find("wf-tab-scheduled").is_some());
        w.click("wf-create", cx);
        w.render_frame(cx);
        assert!(w.try_find("wf-create-dialog").is_some());
        // Empty names are refused.
        w.click("wf-create-ok", cx);
    });
    cx.run_until_parked();
    assert!(workflows(&fx).is_empty());
    win(cx, &fx, |w, cx| {
        w.input("日常巡检", cx);
        w.click("wf-create-ok", cx);
    });
    wait_until(cx, "editor open", |cx| page.read_with(cx, |p, _| p.editor().is_some()));
    let wfs = workflows(&fx);
    assert_eq!((wfs.len(), wfs[0].name.as_str(), wfs[0].status), (1, "日常巡检", WorkflowStatus::Draft));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-editor").is_some() && w.try_find("wf-canvas-empty").is_some());
        w.click("wf-back", cx);
    });
    wait_until(cx, "list reloaded", |cx| cx.update(|cx| workflow_store(cx).read(cx).workflows.len() == 1));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-card-0").is_some(), "back to the list, now with a card");
    });
    page.read_with(cx, |p, _| assert!(p.editor().is_none()));
}

#[gpui_kit::test]
fn the_editor_builds_a_graph_edits_nodes_and_saves_compatible_json(cx: &mut TestAppContext) {
    let _s = serial();
    let host_id = std::cell::RefCell::new(String::new());
    let fx = common::setup(cx, |core| {
        *host_id.borrow_mut() = ops_core::rt::block_on(ops_ssh::hosts::add_host(&core.db, common::new_host("web-1", "10.0.0.1", "linux", None))).unwrap().id;
    });
    let host_id = host_id.into_inner();
    let page = open_page(cx, &fx);
    let wf_id = seed_workflow(&fx, "发布", vec![], vec![]);
    wait_until(cx, "card", |cx| cx.update(|cx| workflow_store(cx).read(cx).workflows.len() == 1));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-edit-0", cx);
        w.render_frame(cx);
    });
    let editor: Entity<WorkflowEditor> = page.read_with(cx, |p, _| p.editor().cloned()).expect("editor");

    // Palette: search then add nodes by clicking.
    win(cx, &fx, |w, cx| {
        for kind in ["selectHost", "command", "condition", "command"] {
            w.click(eid(format!("wf-palette-{kind}")), cx);
            w.render_frame(cx);
        }
    });
    let ids: Vec<String> = editor.read_with(cx, |e, _| e.graph().nodes.iter().map(|n| n.id.clone()).collect());
    assert_eq!(ids.len(), 4);
    editor.read_with(cx, |e, _| {
        assert_eq!(e.graph().nodes[0].name, "选择主机");
        assert_eq!(e.selected(), Some(ids[3].as_str()), "the new node is selected");
    });

    // Host picker (config written as {"hostIds":[...]}), command text, condition text.
    win(cx, &fx, |w, cx| {
        editor.update(cx, |e, cx| e.select(Some(ids[0].clone()), w, cx));
        w.render_frame(cx);
        w.click("wf-host-0", cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |e, _| assert_eq!(e.graph().nodes[0].config, format!(r#"{{"hostIds":["{host_id}"]}}"#)));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-host-count").is_some());
        editor.update(cx, |e, cx| e.select(Some(ids[1].clone()), w, cx));
    });
    // Typing emits Change events, which carry the text into the node config.
    win(cx, &fx, |w, cx| {
        let form = editor.read(cx).form().cloned().unwrap();
        form.update(cx, |f, cx| f.text_input().update(cx, |s, cx| s.focus(w, cx)));
        w.input("uptime -a", cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |e, _| assert_eq!(e.graph().nodes[1].config, "uptime -a"));

    win(cx, &fx, |w, cx| {
        editor.update(cx, |e, cx| e.select(Some(ids[2].clone()), w, cx));
        w.render_frame(cx);
        let form = editor.read(cx).form().cloned().unwrap();
        form.update(cx, |f, cx| f.line_input().update(cx, |s, cx| s.focus(w, cx)));
        w.input("1 === 1", cx);
        // Snippet chips insert at the cursor.
        w.click("wf-snippet-0", cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |e, _| assert_eq!(e.graph().nodes[2].config, "1 === 1{{lastResult.output}}"));

    // Connections are drawn on the canvas: drag from a node's right port onto another node's left port.
    win(cx, &fx, |w, cx| {
        editor.update(cx, |e, cx| {
            for (i, id) in ids.iter().enumerate() {
                e.set_node_position(id, 40.0 + 220.0 * i as f64, 100.0, cx);
            }
            e.fit(cx);
        });
        w.render_frame(cx);
        let out = |id: &str, handle: Option<&str>| eid(match handle { Some(h) => format!("wf-out-{id}-{h}"), None => format!("wf-out-{id}") });
        w.drag_to(out(&ids[0], None), eid(format!("wf-in-{}", ids[1])), cx);
        w.drag_to(out(&ids[1], None), eid(format!("wf-in-{}", ids[2])), cx);
        w.drag_to(out(&ids[2], Some("true")), eid(format!("wf-in-{}", ids[3])), cx);
        // a duplicate and a connection onto the node itself are ignored
        w.drag_to(out(&ids[0], None), eid(format!("wf-in-{}", ids[1])), cx);
        w.drag_to(out(&ids[1], None), eid(format!("wf-in-{}", ids[1])), cx);
    });
    editor.read_with(cx, |e, _| {
        assert_eq!(e.graph().connections.len(), 3);
        assert_eq!(e.graph().connections[0].source_handle.as_deref(), Some("default"), "ordinary nodes save the original's `default` handle");
        assert_eq!(e.graph().connections[2].source_handle.as_deref(), Some("true"));
        assert_eq!(e.graph().connections[2].id, format!("edge-{}-{}-true", ids[2], ids[3]));
        assert_eq!(e.graph().levels().len(), 4);
    });
    // The context menu on a connection deletes it.
    win(cx, &fx, |w, cx| {
        let edge_id = editor.read(cx).graph().connections[2].id.clone();
        let p = editor.read(cx).edge_point(&edge_id, cx).expect("edge is drawn");
        w.pointer_down(p, gpui_kit::MouseButton::Right, cx);
        w.pointer_up(p, gpui_kit::MouseButton::Right, cx);
        assert!(w.try_find("wf-context-menu").is_some());
        w.click("wf-context-delete", cx);
    });
    editor.read_with(cx, |e, _| assert_eq!(e.graph().connections.len(), 2));

    // Disable + delete a node, interval, save.
    win(cx, &fx, |w, cx| {
        editor.update(cx, |e, cx| e.select(Some(ids[1].clone()), w, cx));
        w.render_frame(cx);
        w.click("wf-edit-enabled-row", cx);
        editor.update(cx, |e, cx| e.select(Some(ids[3].clone()), w, cx));
        w.render_frame(cx);
        w.click("wf-delete-node", cx);
        let interval = editor.read(cx).interval_input().clone();
        interval.update(cx, |s, cx| s.set_value("2".to_string(), w, cx));
        w.render_frame(cx);
        w.click("wf-save", cx);
    });
    wait_until(cx, "saved", |_| workflows(&fx)[0].status == WorkflowStatus::Ready && workflows(&fx)[0].nodes.len() == 3);
    let saved = workflows(&fx).remove(0);
    assert_eq!(saved.id, wf_id);
    assert_eq!(saved.settings, WorkflowSettings { default_node_interval_seconds: 2.0 });
    assert!(!saved.nodes[1].enabled && saved.nodes[1].config == "uptime -a");
    assert_eq!(saved.connections.len(), 2);
    assert_eq!(ops_workflow::model::collect_workflow_host_ids(&saved.nodes), vec![host_id]);
    // The stored column is the {nodes, settings} payload of the Tauri build.
    let raw: String = fx.ctx.db.pool.get().unwrap().query_row("SELECT nodes FROM workflows WHERE id=?1", [&wf_id], |r| r.get(0)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(v["nodes"].is_array() && v["settings"]["defaultNodeIntervalSeconds"] == 2.0);
    assert_eq!(v["nodes"][0]["type"], "selectHost");
}

#[gpui_kit::test]
fn the_canvas_pans_zooms_drags_nodes_and_deletes_with_the_keyboard_and_the_menu(cx: &mut TestAppContext) {
    use gpui_kit::{point, px, MouseButton, ScrollDelta};
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let page = open_page(cx, &fx);
    let mut a = node("a", "start", "开始", "");
    let mut b = node("b", "command", "执行命令", "ls");
    let mut c = node("c", "end", "结束", "");
    (a.x, a.y, b.x, b.y, c.x, c.y) = (40.0, 60.0, 300.0, 60.0, 560.0, 60.0);
    seed_workflow(&fx, "画布", vec![a, b, c], vec![edge("a", "b", Some("default")), edge("b", "c", Some("default"))]);
    wait_until(cx, "card", |cx| cx.update(|cx| workflow_store(cx).read(cx).workflows.len() == 1));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-edit-0", cx);
        w.render_frame(cx);
        w.render_frame(cx);
    });
    let editor: Entity<WorkflowEditor> = page.read_with(cx, |p, _| p.editor().cloned()).expect("editor");

    win(cx, &fx, |w, cx| {
        // the view was fitted to the three nodes: all of them are on screen, nothing is cut off
        let canvas = w.find("wf-canvas").bounds();
        for id in ["a", "b", "c"] {
            let n = w.find(eid(format!("wf-node-{id}"))).bounds();
            assert!(n.origin.x >= canvas.origin.x && n.right() <= canvas.right() && n.origin.y >= canvas.origin.y && n.bottom() <= canvas.bottom(), "{id} is inside the canvas");
        }
        assert!(w.try_find("wf-minimap").is_some() && w.try_find("wf-zoom-in").is_some() && w.try_find("wf-fit").is_some());

        // dragging a node moves it by the mouse distance divided by the zoom
        let zoom = editor.read(cx).view().zoom;
        let from = editor.read(cx).node_screen_origin("b").unwrap() + point(px(30.), px(10.));
        w.drag(from, from + point(px(60.), px(30.)), cx);
        let n = editor.read(cx).graph().node("b").unwrap().clone();
        assert!((n.x - (300.0 + 60.0 / zoom as f64)).abs() < 1.0 && (n.y - (60.0 + 30.0 / zoom as f64)).abs() < 1.0, "moved to ({}, {})", n.x, n.y);
        assert_eq!(editor.read(cx).selected(), Some("b"), "dragging selects the node");

        // dragging the background pans, and deselects
        let view = editor.read(cx).view();
        let canvas = w.find("wf-canvas").bounds();
        let empty = canvas.origin + point(px(20.), canvas.size.height - px(80.));
        w.drag(empty, empty + point(px(50.), px(-25.)), cx);
        let panned = editor.read(cx).view();
        assert!((panned.x - view.x - 50.0).abs() < 1.0 && (panned.y - view.y + 25.0).abs() < 1.0 && panned.zoom == view.zoom);
        assert_eq!(editor.read(cx).selected(), None);

        // the wheel zooms around the cursor, within limits; the buttons zoom around the middle; fit restores the view
        w.scroll("wf-canvas", ScrollDelta::Pixels(point(px(0.), px(200.))), cx);
        let zoomed = editor.read(cx).view().zoom;
        assert!(zoomed > panned.zoom, "wheel up zooms in ({} -> {zoomed})", panned.zoom);
        for _ in 0..40 {
            w.click("wf-zoom-out", cx);
        }
        assert_eq!(editor.read(cx).view().zoom, 0.3);
        w.click("wf-zoom-in", cx);
        assert!((editor.read(cx).view().zoom - 0.36).abs() < 1e-4);
        w.click("wf-fit", cx);
        let fitted = editor.read(cx).view();
        assert!(fitted.zoom > 0.3 && fitted.zoom <= 2.0);

        // Backspace deletes the selected connection; a click on it selects it first
        let edge_id = editor.read(cx).graph().connections[0].id.clone();
        let mid = editor.read(cx).edge_point(&edge_id, cx).unwrap();
        w.pointer_down(mid, MouseButton::Left, cx);
        w.pointer_up(mid, MouseButton::Left, cx);
        assert_eq!(editor.read(cx).selected_edge(), Some(edge_id.as_str()));
        w.press("backspace", cx);
        assert_eq!(editor.read(cx).graph().connections.len(), 1);

        // Delete removes the selected node together with its connections
        let origin = editor.read(cx).node_screen_origin("c").unwrap() + point(px(20.), px(10.));
        w.pointer_down(origin, MouseButton::Left, cx);
        w.pointer_up(origin, MouseButton::Left, cx);
        assert_eq!(editor.read(cx).selected(), Some("c"));
        w.press("delete", cx);
        assert!(editor.read(cx).graph().node("c").is_none() && editor.read(cx).graph().connections.is_empty());

        // right click on a node opens the menu; Escape closes it, choosing the item deletes
        let origin = editor.read(cx).node_screen_origin("a").unwrap() + point(px(20.), px(10.));
        w.pointer_down(origin, MouseButton::Right, cx);
        w.pointer_up(origin, MouseButton::Right, cx);
        assert!(w.try_find("wf-context-menu").is_some());
        w.press("escape", cx);
        assert!(w.try_find("wf-context-menu").is_none());
        w.pointer_down(origin, MouseButton::Right, cx);
        w.pointer_up(origin, MouseButton::Right, cx);
        w.click("wf-context-delete", cx);
        assert!(editor.read(cx).graph().node("a").is_none());
        assert!(w.try_find("wf-canvas-empty").is_none() || editor.read(cx).graph().nodes.len() == 1);
    });
}

#[gpui_kit::test]
fn the_canvas_selects_several_nodes_with_a_marquee_moves_them_together_and_deletes_them(cx: &mut TestAppContext) {
    use gpui_kit::{point, px, Modifiers, MouseButton};
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let page = open_page(cx, &fx);
    let mut nodes = vec![node("a", "command", "甲", ""), node("b", "command", "乙", ""), node("c", "command", "丙", "")];
    for (i, n) in nodes.iter_mut().enumerate() {
        n.x = 40.0 + 260.0 * i as f64;
        n.y = 80.0;
    }
    seed_workflow(&fx, "多选", nodes, vec![]);
    wait_until(cx, "card", |cx| cx.update(|cx| workflow_store(cx).read(cx).workflows.len() == 1));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-edit-0", cx);
        w.render_frame(cx);
        w.render_frame(cx);
    });
    let editor: Entity<WorkflowEditor> = page.read_with(cx, |p, _| p.editor().cloned()).expect("editor");
    win(cx, &fx, |w, cx| {
        // shift + drag on the background draws a selection rectangle around the first two nodes
        let canvas = w.find("wf-canvas").bounds();
        let a = editor.read(cx).node_screen_origin("a").unwrap();
        let b = editor.read(cx).node_screen_origin("b").unwrap();
        let from = point(canvas.origin.x + px(4.), a.y - px(10.));
        let to = b + point(px(60.), px(60.));
        w.change_modifiers(Modifiers::shift(), cx);
        w.drag(from, to, cx);
        w.change_modifiers(Modifiers::default(), cx);
        assert_eq!(editor.read(cx).selected_nodes(), ["a".to_string(), "b".to_string()]);
        assert!(w.try_find("wf-edit-panel").is_none(), "the edit panel is for a single node");

        // dragging one of the selected nodes moves both
        let before: Vec<(f64, f64)> = ["a", "b", "c"].iter().map(|id| { let n = editor.read(cx).graph().node(id).unwrap(); (n.x, n.y) }).collect();
        let zoom = editor.read(cx).view().zoom as f64;
        let from = editor.read(cx).node_screen_origin("a").unwrap() + point(px(30.), px(10.));
        w.drag(from, from + point(px(0.), px(40.)), cx);
        let after: Vec<(f64, f64)> = ["a", "b", "c"].iter().map(|id| { let n = editor.read(cx).graph().node(id).unwrap(); (n.x, n.y) }).collect();
        assert!((after[0].1 - before[0].1 - 40.0 / zoom).abs() < 1.0 && (after[1].1 - before[1].1 - 40.0 / zoom).abs() < 1.0 && after[2] == before[2]);

        // arrow keys nudge the selection by 5 (20 with shift)
        let y = editor.read(cx).graph().node("a").unwrap().y;
        w.press("down", cx);
        assert_eq!(editor.read(cx).graph().node("a").unwrap().y, y + 5.0);
        w.press("shift-right", cx);
        assert_eq!(editor.read(cx).graph().node("b").unwrap().x, after[1].0 + 20.0);

        // shift + click toggles a node: c joins, then leaves again
        let c = editor.read(cx).node_screen_origin("c").unwrap() + point(px(30.), px(10.));
        w.change_modifiers(Modifiers::shift(), cx);
        w.pointer_down(c, MouseButton::Left, cx);
        w.pointer_up(c, MouseButton::Left, cx);
        assert_eq!(editor.read(cx).selected_nodes().len(), 3);
        w.pointer_down(c, MouseButton::Left, cx);
        w.pointer_up(c, MouseButton::Left, cx);
        w.change_modifiers(Modifiers::default(), cx);
        assert_eq!(editor.read(cx).selected_nodes(), ["a".to_string(), "b".to_string()]);

        // Backspace removes every selected node
        w.press("backspace", cx);
        let left: Vec<String> = editor.read(cx).graph().nodes.iter().map(|n| n.id.clone()).collect();
        assert_eq!(left, ["c".to_string()]);
    });
}

#[gpui_kit::test]
fn runs_from_the_list_through_ssh_with_branches_confirmation_and_templates(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let host = std::cell::RefCell::new(String::new());
    let fx = common::setup(cx, |core| *host.borrow_mut() = add_ssh_host(core, sshd.port as i32, &user, &sshd.private_key));
    let host = host.into_inner();
    let nodes = vec![
        node("sh", "selectHost", "目标", &format!(r#"{{"hostIds":["{host}"]}}"#)),
        node("a", "command", "第一步", "echo wf-first"),
        node("ok", "confirm", "确认上线", "上线 {{lastResult.output}} ?"),
        node("cond", "condition", "是否成功", "{{result.a.exitCode}} === 0"),
        node("yes", "command", "成功分支", r#"echo "yes:{{result.a}}""#),
        node("no", "command", "失败分支", "echo no"),
    ];
    let edges = vec![edge("sh", "a", None), edge("a", "ok", None), edge("ok", "cond", None), edge("cond", "yes", Some("true")), edge("cond", "no", Some("false"))];
    let wf_id = seed_workflow(&fx, "上线", nodes.clone(), edges.clone());
    let nohost = seed_workflow(&fx, "没有主机", vec![node("c", "command", "c", "echo x")], vec![]);
    let page = open_page(cx, &fx);
    wait_until(cx, "cards", |cx| cx.update(|cx| workflow_store(cx).read(cx).workflows.len() == 2));
    win(cx, &fx, |w, cx| w.render_frame(cx));
    let (idx, idx_nohost) = {
        let list = cx.update(|cx| workflow_store(cx).read(cx).workflows.clone());
        (list.iter().position(|w| w.id == wf_id).unwrap(), list.iter().position(|w| w.id == nohost).unwrap())
    };
    win(cx, &fx, |w, cx| {
        assert!(w.try_find(eid(format!("wf-state-{idx}"))).is_some());
        // A workflow without a target host cannot run (the button is disabled; the method also refuses).
        w.click(eid(format!("wf-exec-{idx_nohost}")), cx);
    });
    cx.run_until_parked();
    page.read_with(cx, |p, cx_| {
        let _ = cx_;
        assert!(!p.run().read(cx_).running);
    });

    win(cx, &fx, |w, cx| w.click(eid(format!("wf-exec-{idx}")), cx));
    // Wait for the confirmation node, then continue.
    wait_until(cx, "confirmation requested", |cx| page.read_with(cx, |p, app| p.run().read(app).pending_confirm().is_some()));
    let (name, desc) = page.read_with(cx, |p, app| {
        let (n, d) = p.run().read(app).pending_confirm().unwrap();
        (n.to_string(), d.to_string())
    });
    assert_eq!(name, "确认上线");
    assert_eq!(desc, "上线 [ssh-box] wf-first\n ?", "the previous node's output (with its trailing newline) is substituted");
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-run-dialog").is_some() && w.try_find("wf-confirm").is_some());
        w.click("wf-confirm-ok", cx);
    });
    wait_until(cx, "finished", |cx| page.read_with(cx, |p, app| !p.run().read(app).running));
    let log = log_text(cx, &page);
    assert!(log.contains("第一步|[ssh-box] wf-first"), "{log}");
    assert!(log.contains("成功分支|[ssh-box] yes:[ssh-box] wf-first"), "named results substitute: {log}");
    assert!(!log.contains("失败分支"), "the false branch never ran: {log}");
    assert!(log.contains("条件节点「是否成功」评估结果: 是 (true)") && log.contains("工作流执行完成"), "{log}");
    page.read_with(cx, |p, app| {
        let run = p.run().read(app);
        assert_eq!(run.statuses.get("yes"), Some(&opsbatch_app::workflow::run::NodeStatus::Success));
        assert_eq!(run.statuses.get("no"), None);
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-log-row-0").is_some());
        w.click("wf-run-close", cx);
    });

    // Declining the confirmation terminates the run before the next node.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click(eid(format!("wf-exec-{idx}")), cx);
    });
    wait_until(cx, "second confirmation", |cx| page.read_with(cx, |p, app| p.run().read(app).pending_confirm().is_some()));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-confirm-cancel", cx);
    });
    wait_until(cx, "terminated", |cx| page.read_with(cx, |p, app| !p.run().read(app).running));
    let log = log_text(cx, &page);
    assert!(log.contains("工作流已被用户终止") && !log.contains("成功分支"), "{log}");
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-run-close", cx);
    });

    // Save as a template and create a new workflow from it.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click(eid(format!("wf-tpl-{idx}")), cx);
    });
    wait_until(cx, "template saved", |cx| cx.update(|cx| workflow_store(cx).read(cx).templates.len() == 1));
    page.update(cx, |p, cx| p.set_tab(PageTab::Templates, cx));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-tpl-row-0").is_some());
        w.click("wf-tpl-use-0", cx);
    });
    wait_until(cx, "editor from template", |cx| page.read_with(cx, |p, _| p.editor().is_some()));
    let created = workflows(&fx);
    assert_eq!(created.len(), 3);
    let copy = created.iter().find(|w| w.name == "上线 模板").expect("named after the template");
    assert_eq!((copy.nodes.clone(), copy.connections.clone()), (nodes, edges), "graph copied untouched");
    let _ = Duration::from_secs(0);
}

#[gpui_kit::test]
fn scheduled_tasks_can_be_listed_toggled_and_deleted(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let wf_id = seed_workflow(&fx, "巡检", vec![], vec![]);
    let task_id = db::add_scheduled_task(&fx.ctx.db, "每小时", "every:3600", &wf_id).unwrap();
    db::add_scheduled_task(&fx.ctx.db, "孤儿", "every:60", "gone").unwrap();
    let page = open_page(cx, &fx);
    wait_until(cx, "tasks loaded", |cx| cx.update(|cx| workflow_store(cx).read(cx).tasks.len() == 2));
    page.update(cx, |p, cx| p.set_tab(PageTab::Scheduled, cx));
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("wf-task-row-0").is_some() && w.try_find("wf-task-row-1").is_some());
        w.click("wf-task-enabled-0", cx);
    });
    // The list is sorted by name, so find which row is ours.
    let (row, row_name) = cx.update(|cx| {
        let tasks = workflow_store(cx).read(cx).tasks.clone();
        let i = tasks.iter().position(|t| t.id == task_id).unwrap();
        (i, tasks[i].name.clone())
    });
    assert_eq!(row_name, "每小时");
    if row != 0 {
        // The click above hit the other row; flip the right one.
        win(cx, &fx, |w, cx| w.click(eid(format!("wf-task-enabled-{row}")), cx));
    }
    wait_until(cx, "disabled", |_| !db::list_scheduled_tasks(&fx.ctx.db).unwrap().iter().find(|t| t.id == task_id).unwrap().enabled);
    // Create through the dialog's validation: nothing is created while fields are empty.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("wf-task-create", cx);
        w.render_frame(cx);
        assert!(w.try_find("wf-task-dialog").is_some());
        w.click("wf-task-ok", cx);
    });
    cx.run_until_parked();
    assert_eq!(db::list_scheduled_tasks(&fx.ctx.db).unwrap().len(), 2);
    win(cx, &fx, |w, cx| w.click("wf-task-cancel", cx));
    // Delete with confirmation.
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click(eid(format!("wf-task-del-{row}")), cx);
        w.render_frame(cx);
        w.press("enter", cx);
    });
    wait_until(cx, "deleted", |_| db::list_scheduled_tasks(&fx.ctx.db).unwrap().len() == 1);
}
