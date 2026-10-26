//! Library window: commands, docker, scripts, quick actions (real DB; execution against a private sshd).

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use opsbatch_app::library::store::library_store;
use opsbatch_app::library::window::{LibraryTab, LibraryWindow};
use ops_ssh::hosts::NewHost;

use common::{serial, wait_until};

fn seed_library(core: &ops_core::Core) {
    let c = core.db.pool.get().unwrap();
    for (id, name, cmd, tags, cat, builtin) in [
        ("c1", "Disk usage", "df -h", "[]", "disk", 1),
        ("c2", "Memory", "free -m", "[\"ram\"]", "memory", 1),
        ("c3", "Docker ps", "docker ps", "[\"docker\"]", "container", 0),
    ] {
        c.execute("INSERT INTO commands (id,name,command,tags,category,is_builtin,description,risk,platform,parameters,url) VALUES (?1,?2,?3,?4,?5,?6,'','low','linux','[]','')", rusqlite::params![id, name, cmd, tags, cat, builtin]).unwrap();
    }
    c.execute("INSERT INTO scripts (id,name,language,category,content,is_builtin,tags,risk,description,parameters,url,platform) VALUES ('s1','Backup','shell','ops','echo backup',0,'[]','low','nightly','[]','','linux')", []).unwrap();
    c.execute("INSERT INTO scripts (id,name,language,category,content,is_builtin,tags,risk,description,parameters,url,platform) VALUES ('s2','Builtin','python','ops','print(1)',1,'[]','low','','[]','','linux')", []).unwrap();
}

fn open_window(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, gpui_kit::Entity<LibraryWindow>) {
    cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds { origin: Default::default(), size: gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(720.)) })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| LibraryWindow::new(LibraryTab::Commands, window, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    })
}

#[gpui_kit::test]
fn commands_docker_and_scripts_pages_browse_filter_add_edit_and_delete(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, seed_library);
    let (handle, win) = open_window(cx);
    let store = cx.update(|cx| library_store(cx));
    wait_until(cx, "library loaded", |cx| cx.update(|cx| store.read(cx).commands.len() == 3 && store.read(cx).scripts.len() == 2));

    // ---- commands page: docker commands are not listed here ----
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("clib-row", 0usize)).is_some() && w.try_find(("clib-row", 1usize)).is_some());
        assert!(w.try_find(("clib-row", 2usize)).is_none(), "docker command hidden in the command library");
        // built-in rows have no delete button
        assert!(w.try_find(("clib-delete", 0usize)).is_none());
        // category rail: all + 2 categories; pick one
        assert!(w.try_find(("clib-cat", 2usize)).is_some());
        w.click(("clib-cat", 1usize), cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("clib-row", 0usize)).is_some());
        assert!(w.try_find(("clib-row", 1usize)).is_none(), "filtered to one category");
        w.click(("clib-cat", 0usize), cx);
    })
    .unwrap();

    // Add a custom command through the dialog (validation first).
    let page = cx.update(|cx| win.read(cx).commands().clone());
    let dlg = cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| p.open_add(w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| d.submit(w, cx));
    })
    .unwrap();
    cx.update(|cx| assert_eq!(dlg.read(cx).errors(), ["name", "category", "command"]));
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("Uptime", w, cx));
            d.command.update(cx, |s, cx| s.set_value("uptime", w, cx));
            d.set_category("system", w, cx);
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "command added", |cx| cx.update(|cx| store.read(cx).commands.iter().any(|c| c.name == "Uptime" && !c.is_builtin)));

    // Search narrows the list; delete removes the custom command.
    cx.update_window(handle.into(), |_, w, cx| {
        page.update(cx, |p, cx| p.set_search("uptime", w, cx));
        w.render_frame(cx);
        assert!(w.try_find(("clib-row", 0usize)).is_some() && w.try_find(("clib-row", 1usize)).is_none());
        w.click(("clib-delete", 0usize), cx);
    })
    .unwrap();
    wait_until(cx, "command deleted", |cx| cx.update(|cx| !store.read(cx).commands.iter().any(|c| c.name == "Uptime")));

    // "Add to quick actions".
    cx.update_window(handle.into(), |_, w, cx| {
        page.update(cx, |p, cx| p.set_search("", w, cx));
        w.render_frame(cx);
        w.click(("clib-quick", 0usize), cx);
    })
    .unwrap();
    wait_until(cx, "quick action created", |cx| cx.update(|cx| store.read(cx).actions.len() == 1));

    // ---- docker tab shows only the docker command ----
    cx.update(|cx| win.update(cx, |w, cx| w.set_tab(LibraryTab::Docker, cx)));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("clib-row", 0usize)).is_some() && w.try_find(("clib-row", 1usize)).is_none());
    })
    .unwrap();

    // ---- scripts ----
    cx.update(|cx| win.update(cx, |w, cx| w.set_tab(LibraryTab::Scripts, cx)));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("sl-row", 1usize)).is_some());
        // Only the custom script can be edited / deleted.
        let rows = [w.try_find(("sl-edit", 0usize)).is_some(), w.try_find(("sl-edit", 1usize)).is_some()];
        assert_eq!(rows.iter().filter(|x| **x).count(), 1);
    })
    .unwrap();
    let scripts = cx.update(|cx| win.read(cx).scripts().clone());
    let custom = cx.update(|cx| store.read(cx).scripts.iter().find(|s| s.id == "s1").cloned().unwrap());
    let view = cx.update_window(handle.into(), |_, w, cx| scripts.update(cx, |p, cx| p.open_view(custom.clone(), w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("script-view").is_some());
        let _ = &view;
        w.click("script-view-close", cx);
    })
    .unwrap();

    let edit = cx.update_window(handle.into(), |_, w, cx| scripts.update(cx, |p, cx| p.open_edit(custom.clone(), w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        edit.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("Backup v2", w, cx));
            d.code.update(cx, |c, cx| c.set_text("echo backup v2", w, cx));
            d.save(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "script saved", |cx| cx.update(|cx| store.read(cx).scripts.iter().any(|s| s.name == "Backup v2" && s.content == "echo backup v2")));
    // Two versions of the old content were recorded (on opening the editor and on saving).
    let versions = ops_core::rt::block_on(ops_library::library::list_script_versions(&cx.update(|cx| opsbatch_app::services::ctx(cx)).db, "s1".into())).unwrap();
    assert!(!versions.is_empty() && versions.iter().all(|v| v.content == "echo backup"), "{versions:?}");

    // Add a script.
    let add = cx.update_window(handle.into(), |_, w, cx| scripts.update(cx, |p, cx| p.open_add(w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| add.update(cx, |d, cx| d.submit(w, cx))).unwrap();
    cx.update(|cx| assert_eq!(add.read(cx).errors(), ["name", "category", "content"]));
    cx.update_window(handle.into(), |_, w, cx| {
        add.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("Cleanup", w, cx));
            d.code.update(cx, |c, cx| c.set_text("rm -rf /tmp/x", w, cx));
            d.set_category("ops", w, cx);
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "script added", |cx| cx.update(|cx| store.read(cx).scripts.iter().any(|s| s.name == "Cleanup")));
}

#[gpui_kit::test]
fn quick_actions_run_on_selected_hosts_with_parameters_and_record_the_result(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let _fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost { name: "qa".into(), ip: "127.0.0.1".into(), port: Some(port), auth_type: Some("key".into()), username: Some(user.clone()), password: None, private_key: Some(key.clone()), os: Some("linux".into()), tags: None, group_id: None, remark: None, jump_chain: None, rdp_settings: None, proxy_settings: None },
        ))
        .unwrap()
        .id;
        let c = core.db.pool.get().unwrap();
        c.execute("INSERT INTO quick_actions (id,name,command,category,parameters,sort_order,starred,description,tags,language,last_run_at,last_status) VALUES ('a1','plain','echo plain-$((1+1))','sys','[]',0,0,'','[]','shell','','')", []).unwrap();
        c.execute("INSERT INTO quick_actions (id,name,command,category,parameters,sort_order,starred,description,tags,language,last_run_at,last_status) VALUES ('a2','with param','echo hello-{{who:world}}','sys','[]',1,0,'','[]','shell','','')", []).unwrap();
    });
    let (handle, win) = open_window(cx);
    let store = cx.update(|cx| library_store(cx));
    cx.update(|cx| win.update(cx, |w, cx| w.set_tab(LibraryTab::Actions, cx)));
    wait_until(cx, "actions loaded", |cx| cx.update(|cx| store.read(cx).actions.len() == 2));
    let page = cx.update(|cx| win.read(cx).actions().clone());
    let assets = cx.update(|cx| cx.global::<opsbatch_app::ui_state::UiState>().assets.clone());

    // No hosts selected: execute refuses (button disabled / warning), nothing runs.
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find(("qa-run", 0usize)).disabled(), Some(true));
        let a = store.read(cx).actions[0].clone();
        assert!(page.update(cx, |p, cx| p.execute(&a, w, cx)).is_none());
    })
    .unwrap();
    cx.update(|cx| assert!(store.read(cx).actions.iter().all(|a| a.last_status.is_empty())));

    // Select the host and run the plain action: status gets recorded.
    cx.update(|cx| assets.update(cx, |s, cx| s.set_selected([host_id.clone()], cx)));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_ne!(w.find(("qa-run", 0usize)).disabled(), Some(true));
        w.click(("qa-run", 0usize), cx);
    })
    .unwrap();
    wait_until(cx, "status recorded", |cx| cx.update(|cx| store.read(cx).actions.iter().any(|a| a.id == "a1" && a.last_status == "success" && !a.last_run_at.is_empty())));

    // Parameter action opens the dialog; empty values are rejected, then it runs.
    let a2 = cx.update(|cx| store.read(cx).actions.iter().find(|a| a.id == "a2").cloned().unwrap());
    let dlg = cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| p.execute(&a2, w, cx))).unwrap().expect("param dialog");
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.inputs[0].update(cx, |s, cx| s.set_value("", w, cx));
            d.submit(w, cx);
        });
    })
    .unwrap();
    assert_eq!(cx.update(|cx| store.read(cx).actions.iter().find(|a| a.id == "a2").unwrap().last_status.clone()), "");
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.inputs[0].update(cx, |s, cx| s.set_value("gpui", w, cx));
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "param action recorded", |cx| cx.update(|cx| store.read(cx).actions.iter().any(|a| a.id == "a2" && a.last_status == "success")));

    // Create + edit + delete.
    let form = cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| p.open_edit(None, w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| form.update(cx, |d, cx| d.submit(w, cx))).unwrap();
    cx.update(|cx| assert_eq!(form.read(cx).errors(), ["name", "category", "command"]));
    cx.update_window(handle.into(), |_, w, cx| {
        form.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("uptime", w, cx));
            d.command.update(cx, |s, cx| s.set_value("uptime", w, cx));
            d.tags.update(cx, |s, cx| s.set_value("a, b ,", w, cx));
            d.set_category("sys", w, cx);
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "action created", |cx| cx.update(|cx| store.read(cx).actions.iter().any(|a| a.name == "uptime" && a.tags == ["a", "b"])));
    let created = cx.update(|cx| store.read(cx).actions.iter().find(|a| a.name == "uptime").cloned().unwrap());
    let edit = cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| p.open_edit(Some(created.clone()), w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| {
        edit.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("uptime -p", w, cx));
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "action renamed", |cx| cx.update(|cx| store.read(cx).actions.iter().any(|a| a.name == "uptime -p")));
}

#[gpui_kit::test]
fn inline_panels_insert_commands_scripts_and_docker_entries_on_the_prompt(cx: &mut TestAppContext) {
    use opsbatch_app::terminal::workspace::{workspace, ConnState};
    let _s = serial();
    let fx = common::setup(cx, |core| {
        seed_library(core);
        let c = core.db.pool.get().unwrap();
        c.execute("UPDATE commands SET parameters='[{\"name\":\"NAME\",\"description\":\"container\",\"required\":true,\"defaultValue\":\"\"}]', command='docker logs ${NAME}' WHERE id='c3'", []).unwrap();
        c.execute("UPDATE scripts SET parameters='[]' WHERE id='s1'", []).unwrap();
        c.execute("INSERT INTO commands (id,name,command,tags,category,is_builtin,description,risk,platform,parameters,url) VALUES ('c4','Docker info','docker info','[\"docker\"]','overview',1,'','low','linux','[]','')", []).unwrap();
    });
    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| wsp.update(cx, |w, cx| drop(w.open_local(cx))));
    wait_until(cx, "local tab", |cx| cx.update(|cx| wsp.read(cx).active_tab().is_some_and(|t| t.state == ConnState::Connected)));
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.run_until_parked();
    let page = cx.update(|cx| fx.workbench.read(cx).terminal_page().unwrap().clone());
    let pane = cx.update(|cx| wsp.read(cx).active_tab().unwrap().pane.clone().unwrap());

    // Local shells get commands/scripts but no docker tab.
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("bottom-tab-docker").is_none());
        w.click("bottom-tab-commands", cx);
    })
    .unwrap();
    let cmds = cx.update(|cx| page.read(cx).inline_panel("commands").cloned()).expect("commands panel");
    wait_until(cx, "commands listed", |cx| cx.update(|cx| library_store(cx).read(cx).commands.len() == 4));
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("inline-cmd", 1000usize)).is_some() && w.try_find(("inline-cmd", 1003usize)).is_some(), "all commands incl. docker ones are listed");
    })
    .unwrap();

    // Run "Disk usage": the dialog previews the command; confirming puts it on the prompt without executing.
    let id = cx.update(|cx| library_store(cx).read(cx).commands.iter().find(|c| c.name == "Disk usage").unwrap().id.clone());
    let dlg = cx.update_window(fx.handle.into(), |_, w, cx| cmds.update(cx, |p, cx| p.run(&id, w, cx))).unwrap().expect("dialog");
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("inline-run-dialog").is_some());
        assert_eq!(dlg.read(cx).resolved(cx), "df -h");
        w.click("run-confirm", cx);
    })
    .unwrap();
    wait_until(cx, "inserted on prompt", |cx| cx.update(|cx| pane.read(cx).screen_text().contains("df -h")));

    // Scripts tab.
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("bottom-tab-scripts", cx);
    })
    .unwrap();
    let scripts = cx.update(|cx| page.read(cx).inline_panel("scripts").cloned()).expect("scripts panel");
    wait_until(cx, "scripts listed", |cx| cx.update(|cx| library_store(cx).read(cx).scripts.len() == 2));
    let dlg = cx.update_window(fx.handle.into(), |_, w, cx| scripts.update(cx, |p, cx| p.run("s1", w, cx))).unwrap().expect("dialog");
    cx.update(|cx| assert_eq!(dlg.read(cx).resolved(cx), "echo backup"));
    cx.update_window(fx.handle.into(), |_, w, cx| dlg.update(cx, |d, cx| d.confirm(w, cx))).unwrap();
    wait_until(cx, "script inserted", |cx| cx.update(|cx| pane.read(cx).screen_text().contains("echo backup")));

    // Star toggling goes through the store.
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("inline-script", 5000usize), cx);
    })
    .unwrap();
    wait_until(cx, "star persisted", |cx| cx.update(|cx| library_store(cx).read(cx).scripts.iter().any(|s| s.starred)));
}

#[gpui_kit::test]
fn docker_panel_needs_parameters_before_inserting_and_validates_required_ones(cx: &mut TestAppContext) {
    use opsbatch_app::terminal::workspace::{workspace, ConnState};
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost { name: "dk".into(), ip: "127.0.0.1".into(), port: Some(port), auth_type: Some("key".into()), username: Some(user.clone()), password: None, private_key: Some(key.clone()), os: Some("linux".into()), tags: None, group_id: None, remark: None, jump_chain: None, rdp_settings: None, proxy_settings: None },
        ))
        .unwrap()
        .id;
        seed_library(core);
        let c = core.db.pool.get().unwrap();
        c.execute("UPDATE commands SET parameters='[{\"name\":\"NAME\",\"description\":\"container\",\"required\":true,\"defaultValue\":\"\"}]', command='docker logs ${NAME}' WHERE id='c3'", []).unwrap();
        c.execute("INSERT INTO commands (id,name,command,tags,category,is_builtin,description,risk,platform,parameters,url) VALUES ('c4','Docker info','docker info','[\"docker\"]','overview',1,'','low','linux','[]','')", []).unwrap();
    });
    let wsp = cx.update(|cx| workspace(cx));
    cx.update(|cx| wsp.update(cx, |w, cx| { let _ = w.open_host(&host_id, "dk", "127.0.0.1", cx); }));
    wait_until(cx, "ssh tab", |cx| cx.update(|cx| wsp.read(cx).active_tab().is_some_and(|t| t.state == ConnState::Connected)));
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.run_until_parked();
    let page = cx.update(|cx| fx.workbench.read(cx).terminal_page().unwrap().clone());
    let pane = cx.update(|cx| wsp.read(cx).active_tab().unwrap().pane.clone().unwrap());
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("bottom-tab-docker", cx);
    })
    .unwrap();
    let docker = cx.update(|cx| page.read(cx).inline_panel("docker").cloned()).expect("docker panel");
    wait_until(cx, "docker commands", |cx| cx.update(|cx| library_store(cx).read(cx).commands.len() == 4));
    cx.update_window(fx.handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("inline-docker", 1000usize)).is_some() && w.try_find(("inline-docker", 1001usize)).is_some());
        assert!(w.try_find(("inline-docker", 1002usize)).is_none(), "only docker-kind commands");
    })
    .unwrap();

    // Without parameters: inserted immediately, no dialog.
    let none = cx.update_window(fx.handle.into(), |_, w, cx| docker.update(cx, |p, cx| p.run("c4", w, cx))).unwrap();
    assert!(none.is_none());
    wait_until(cx, "docker info on prompt", |cx| cx.update(|cx| pane.read(cx).screen_text().contains("docker info")));

    // With a required parameter: dialog, validation error, then resolved command.
    let dlg = cx.update_window(fx.handle.into(), |_, w, cx| docker.update(cx, |p, cx| p.run("c3", w, cx))).unwrap().expect("dialog");
    cx.update_window(fx.handle.into(), |_, w, cx| dlg.update(cx, |d, cx| d.confirm(w, cx))).unwrap();
    cx.update(|cx| assert!(dlg.read(cx).error().is_some_and(|e| e.contains("NAME"))));
    cx.update_window(fx.handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.inputs[0].update(cx, |s, cx| s.set_value("web-1", w, cx));
            assert_eq!(d.resolved(cx), "docker logs web-1");
            d.confirm(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "docker logs on prompt", |cx| cx.update(|cx| pane.read(cx).screen_text().contains("docker logs web-1")));
}
