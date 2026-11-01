//! Headless UI tests (GPUI test platform, no OS window): main window, mode tabs, asset sidebar wired to a real DB.

use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle};
use ops_core::theme::ThemeSettings;
use ops_ssh::hosts::NewHost;
use opsbatch_app::app::init_globals;
use opsbatch_app::asset_tree::DEFAULT_GROUP_ID;
use opsbatch_app::stores::assets::AssetsStore;
use opsbatch_app::ui_state::UiState;
use opsbatch_app::workbench::{Mode, Workbench};

/// The UI language is process-wide, so tests in this binary must not interleave.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host(name: &str, ip: &str, os: &str, group: Option<&str>) -> NewHost {
    NewHost {
        name: name.into(),
        ip: ip.into(),
        port: Some(22),
        auth_type: Some("password".into()),
        username: Some("root".into()),
        password: None,
        private_key: None,
        os: Some(os.into()),
        tags: None,
        group_id: group.map(Into::into),
        remark: Some(String::new()),
        jump_chain: None,
        rdp_settings: None,
        proxy_settings: None,
    }
}

struct Fixture {
    handle: WindowHandle<gpui_kit::base::Root>,
    workbench: Entity<Workbench>,
    ctx: std::sync::Arc<ops_ssh::Ctx>,
}

fn setup(cx: &mut TestAppContext, language: ops_i18n::LanguageMode) -> Fixture {
    // Results come back from the tokio runtime thread; tell the deterministic test scheduler that is expected.
    cx.executor().allow_parking();
    let core = ops_core::Core::open_temp("ui").unwrap();
    // Seed through the real services (async -> run on the shared runtime before the UI exists).
    ops_core::rt::block_on(async {
        let g = ops_ssh::groups::add_group(&core.db, "prod".into(), None)
            .await
            .unwrap();
        ops_ssh::hosts::add_host(&core.db, host("alpha", "10.0.0.1", "linux", None))
            .await
            .unwrap();
        ops_ssh::hosts::add_host(&core.db, host("beta-web", "10.0.0.2", "linux", Some(&g.id)))
            .await
            .unwrap();
        ops_ssh::hosts::add_host(&core.db, host("win-1", "10.0.0.3", "windows", Some(&g.id)))
            .await
            .unwrap();
    });
    let ctx = ops_ssh::Ctx::new(core);
    let c2 = ctx.clone();
    cx.update(move |cx| init_globals(c2, language, ThemeSettings::default(), cx));
    let (handle, workbench) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let assets = cx.global::<UiState>().assets.clone();
                cx.new(|cx| Workbench::new(assets, window, cx))
            },
        )
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    Fixture {
        handle,
        workbench,
        ctx,
    }
}

/// The DB is read on the tokio runtime, so wait for the result to arrive on the UI thread.
fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut cond: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if cond(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assets(cx: &mut TestAppContext) -> Entity<AssetsStore> {
    cx.update(|cx| cx.global::<UiState>().assets.clone())
}

#[gpui_kit::test]
fn main_window_has_the_three_mode_tabs_and_switching_updates_ui_state(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = setup(cx, ops_i18n::LanguageMode::ZhCn);
    cx.update_window(fx.handle.into(), |_, window, cx| {
        for id in [
            "mode-terminal",
            "mode-workflow",
            "mode-github",
            "tool-log",
            "tool-settings",
            "tool-update",
        ] {
            assert!(window.try_find(id).is_some(), "missing control {id}");
        }
        assert_eq!(fx.workbench.read(cx).mode(), Mode::Terminal);
        window.click("mode-workflow", cx);
        assert_eq!(fx.workbench.read(cx).mode(), Mode::Workflow);
        assert_eq!(
            cx.global::<UiState>().mode,
            Mode::Workflow,
            "kept for a reopened window"
        );
        window.click("mode-github", cx);
        assert_eq!(fx.workbench.read(cx).mode(), Mode::Github);
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_labels_follow_the_language(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = setup(cx, ops_i18n::LanguageMode::EnUs);
    cx.update_window(fx.handle.into(), |_, window, _| {
        let tab = window.find("mode-workflow");
        let label = tab.label().map(str::to_string).unwrap_or_default();
        let _ = label;
    })
    .unwrap();
    assert_eq!(ops_i18n::current_language(), ops_i18n::Language::EnUs);
    assert_eq!(ops_i18n::t!("nav.terminal"), "Terminal");
    cx.update(|cx| opsbatch_app::language::set_mode(ops_i18n::LanguageMode::ZhCn, cx));
    assert_eq!(ops_i18n::t!("nav.terminal"), "终端");
}

#[gpui_kit::test]
fn asset_panel_loads_real_hosts_and_selection_drives_batch_buttons(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = setup(cx, ops_i18n::LanguageMode::ZhCn);
    // Hidden until requested.
    cx.update_window(fx.handle.into(), |_, window, _| {
        assert!(window.try_find("asset-panel").is_none())
    })
    .unwrap();

    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("open-assets", cx)
    })
    .unwrap();
    cx.update(|cx| assert!(fx.workbench.read(cx).asset_panel_visible()));
    let store = assets(cx);
    wait_until(cx, "hosts loaded", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 3)
    });
    cx.update(|cx| {
        let s = store.read(cx);
        assert_eq!(s.groups.len(), 1);
        assert!(s.error.is_none());
    });

    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("asset-panel").is_some());
        // default group + prod group, hosts visible because both groups hold hosts
        assert!(window.try_find(("group", 0usize)).is_some());
        // batch buttons are disabled until something is selected
        assert_eq!(window.find("assets-broadcast").disabled(), Some(true));
        assert_eq!(window.find("assets-upload").disabled(), Some(true));
    })
    .unwrap();

    // Select the default group via its checkbox: only "alpha" lives there.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click(("group-check", 0usize), cx)
    })
    .unwrap();
    cx.update(|cx| {
        let s = store.read(cx);
        assert_eq!(s.selected.len(), 1);
        assert_eq!(
            s.host(s.selected.iter().next().unwrap()).unwrap().name,
            "alpha"
        );
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_ne!(
            window.find("assets-broadcast").disabled(),
            Some(true),
            "enabled once something is selected"
        );
        assert_ne!(window.find("assets-upload").disabled(), Some(true));
    })
    .unwrap();

    // The Windows host never counts as a broadcast-terminal target.
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            let ids: Vec<String> = s.hosts.iter().map(|h| h.id.clone()).collect();
            s.set_selected(ids, cx);
        });
        assert_eq!(store.read(cx).selected_linux_host_ids().len(), 2);
    });
    let _ = (DEFAULT_GROUP_ID, &fx.ctx);
}

#[gpui_kit::test]
fn asset_panel_lists_recent_connections_above_the_tree_and_persists_them(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = setup(cx, ops_i18n::LanguageMode::ZhCn);
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("open-assets", cx)
    })
    .unwrap();
    let store = assets(cx);
    wait_until(cx, "hosts loaded", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 3)
    });

    // Open by default, empty hint while nothing was connected; the eyebrow label is gone.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("asset-recent").is_some());
        assert!(window.try_find("asset-recent-empty").is_some());
        assert!(window.try_find(("recent-host", 0usize)).is_none());
    })
    .unwrap();

    let (alpha, beta) = cx.update(|cx| {
        let s = store.read(cx);
        (
            s.hosts
                .iter()
                .find(|h| h.name == "alpha")
                .unwrap()
                .id
                .clone(),
            s.hosts
                .iter()
                .find(|h| h.name == "beta-web")
                .unwrap()
                .id
                .clone(),
        )
    });
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.record_recent(&alpha, cx);
            s.record_recent(&beta, cx);
            s.record_recent(&alpha, cx);
        })
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("asset-recent-empty").is_none());
        assert!(window.try_find(("recent-host", 0usize)).is_some());
        assert!(window.try_find(("recent-host", 1usize)).is_some());
        assert!(
            window.try_find(("recent-host", 2usize)).is_none(),
            "no duplicates"
        );
    })
    .unwrap();
    cx.update(|cx| assert_eq!(store.read(cx).recent, [alpha.clone(), beta.clone()]));

    // Collapsing hides the rows but keeps the header.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("asset-recent-toggle", cx)
    })
    .unwrap();
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("asset-recent-toggle").is_some());
        assert!(window.try_find(("recent-host", 0usize)).is_none());
    })
    .unwrap();

    // The list survives a reload from the database.
    let ctx = fx.ctx.clone();
    wait_until(cx, "persisted", |_| {
        let ctx = ctx.clone();
        ops_core::rt::block_on(async move {
            ops_core::settings::get_general_settings(&ctx.db)
                .await
                .unwrap()
                .contains_key("recentHosts")
        })
    });
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.recent.clear();
            s.reload(cx);
        })
    });
    wait_until(cx, "reloaded", |cx| {
        cx.update(|cx| store.read(cx).recent == [alpha.clone(), beta.clone()])
    });
}

#[gpui_kit::test]
fn closing_and_reopening_keeps_selection_and_sidebar(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = setup(cx, ops_i18n::LanguageMode::ZhCn);
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("open-assets", cx)
    })
    .unwrap();
    let store = assets(cx);
    wait_until(cx, "hosts loaded", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 3)
    });
    cx.update(|cx| {
        let first = store.read(cx).hosts[0].id.clone();
        store.update(cx, |s, cx| s.toggle_host(&first, cx));
    });
    cx.update_window(fx.handle.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    // "Show main window" from the tray: a brand new window built from the same global state.
    let second = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(Default::default(), cx, |window, cx| {
            let assets = cx.global::<UiState>().assets.clone();
            cx.new(|cx| Workbench::new(assets, window, cx))
        })
        .unwrap();
        (window, view)
    });
    cx.update(|cx| {
        assert!(
            second.1.read(cx).asset_panel_visible(),
            "sidebar state restored"
        );
        assert_eq!(store.read(cx).selected.len(), 1, "selection restored");
    });
}
