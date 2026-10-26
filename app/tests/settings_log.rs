//! Global log window, settings window and update flow (headless GPUI, real core services).

mod common;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use opsbatch_app::log::store::{log_store, MAX_LOG_ENTRIES};
use opsbatch_app::log::window::{GlobalLogWindow, MAX_RENDERED_LOGS};

use common::{serial, wait_until};

fn open_log_window(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, Entity<GlobalLogWindow>) {
    cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds { origin: Default::default(), size: gpui_kit::size(gpui_kit::px(720.), gpui_kit::px(600.)) })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| GlobalLogWindow::new(window, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    })
}

#[gpui_kit::test]
fn global_log_streams_filters_localises_caps_and_clears(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let (handle, win) = open_log_window(cx);
    let store = cx.update(|cx| log_store(cx));
    wait_until(cx, "history loaded", |cx| cx.update(|cx| !store.read(cx).loading));
    // the "connected" notice is emitted by the window itself
    wait_until(cx, "connected notice", |cx| cx.update(|cx| store.read(cx).logs.iter().any(|l| l.origin == "frontend")));

    let core = fx.ctx.core.clone();
    ops_core::log::emit_log(&core, "error", "ssh", "主机 h1 连接失败: refused", "backend");
    ops_core::log::emit_log(&core, "success", "ssh", "主机 h2 已连接", "backend");
    wait_until(cx, "live entries", |cx| cx.update(|cx| store.read(cx).logs.iter().filter(|l| l.origin == "backend").count() >= 2));

    // newest first
    cx.update(|cx| {
        let lines = win.read(cx).visible(cx);
        assert!(lines[0].3.contains("h2"), "{lines:?}");
    });

    // filters
    cx.update_window(handle.into(), |_, w, cx| win.update(cx, |v, cx| v.set_filters("error", "backend", w, cx))).unwrap();
    cx.update(|cx| {
        let lines = win.read(cx).visible(cx);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].3.contains("h1") && lines[0].3.contains("refused"));
    });
    cx.update_window(handle.into(), |_, w, cx| {
        win.update(cx, |v, cx| v.set_filters("all", "frontend", w, cx));
        w.render_frame(cx);
        assert!(w.try_find("log-count").is_some());
    })
    .unwrap();
    cx.update(|cx| assert!(win.read(cx).visible(cx).iter().all(|l| l.1 == ops_i18n::t!("log.frontend"))));

    // cap: storing keeps 600, rendering draws 300
    cx.update_window(handle.into(), |_, w, cx| win.update(cx, |v, cx| v.set_filters("all", "all", w, cx))).unwrap();
    for i in 0..(MAX_LOG_ENTRIES + 50) {
        ops_core::log::emit_log(&core, "info", "system", &format!("bulk {i}"), "backend");
    }
    wait_until(cx, "bulk delivered", |cx| cx.update(|cx| store.read(cx).logs.first().map(|l| l.message == format!("bulk {}", MAX_LOG_ENTRIES + 49)).unwrap_or(false)));
    cx.update(|cx| {
        assert_eq!(store.read(cx).logs.len(), MAX_LOG_ENTRIES);
        assert_eq!(win.read(cx).visible(cx).len(), MAX_RENDERED_LOGS);
    });

    // clear
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("log-clear", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(store.read(cx).logs.is_empty()));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("log-empty").is_some());
    })
    .unwrap();
    wait_until(cx, "history cleared", |_| ops_core::log::get_log_history(&core, Some(10)).map(|h| h.is_empty()).unwrap_or(false));
}

// ---------------------------------------------------------------------------
// Settings window
// ---------------------------------------------------------------------------

use opsbatch_app::settings::window::{Section, SettingsWindow};

fn open_settings(cx: &mut TestAppContext, section: Section) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, Entity<SettingsWindow>) {
    cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds { origin: Default::default(), size: gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(700.)) })),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| SettingsWindow::new(section, window, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    })
}

fn stored(fx: &common::Fixture) -> std::collections::HashMap<String, String> {
    ops_core::rt::block_on(ops_core::settings::get_general_settings(&fx.ctx.db)).unwrap()
}

#[gpui_kit::test]
fn settings_navigation_and_general_section_persist_clamped_values(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let (handle, win) = open_settings(cx, Section::General);
    let general = cx.update(|cx| win.read(cx).general().clone());
    wait_until(cx, "general loaded", |cx| cx.update(|cx| general.read(cx).loaded()));

    // defaults of the Tauri build
    cx.update(|cx| {
        let [c, t, r] = general.read(cx).inputs().map(|i| i.read(cx).value().to_string());
        assert_eq!((c.as_str(), t.as_str(), r.as_str()), ("10", "30", "90"));
        assert!(general.read(cx).minimize_to_tray());
    });

    // the step arrows of the number fields add / subtract one inside the limits (they show while hovering)
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.hover("set-concurrency-up", cx);
        w.click("set-concurrency-up", cx);
        w.click("set-concurrency-up", cx);
        w.click("set-concurrency-down", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(general.read(cx).inputs()[0].read(cx).value().to_string(), "11"));
    cx.update_window(handle.into(), |_, w, cx| {
        general.update(cx, |g, cx| g.inputs()[0].clone().update(cx, |s, cx| s.set_value("100", w, cx)));
        w.render_frame(cx);
        w.hover("set-concurrency-up", cx);
        w.click("set-concurrency-up", cx);
        general.update(cx, |g, cx| g.inputs()[0].clone().update(cx, |s, cx| s.set_value("10", w, cx)));
    })
    .unwrap();
    cx.update(|cx| assert_eq!(general.read(cx).inputs()[0].read(cx).value().to_string(), "10"));

    // out-of-range numbers are clamped on save, the tray switch is stored
    cx.update_window(handle.into(), |_, w, cx| {
        general.update(cx, |g, cx| {
            let [c, t, r] = g.inputs().map(|i| i.clone());
            c.update(cx, |s, cx| s.set_value("500", w, cx));
            t.update(cx, |s, cx| s.set_value("abc", w, cx));
            r.update(cx, |s, cx| s.set_value("1", w, cx));
            g.set_minimize_to_tray(false, cx);
            g.save(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "stored", |_| stored(&fx).get("defaultConcurrency").is_some());
    let map = stored(&fx);
    assert_eq!(map["defaultConcurrency"], "100");
    assert_eq!(map["defaultTimeout"], "30");
    assert_eq!(map["logRetentionDays"], "7");
    assert_eq!(map["minimizeToTray"], "false");
    assert!(!ops_core::settings::minimize_to_tray(&fx.ctx.db));

    // navigation through the rail shows every section
    for (ix, expected) in Section::ALL.iter().enumerate() {
        cx.update_window(handle.into(), |_, w, cx| {
            w.render_frame(cx);
            w.click(("settings-section", ix), cx);
        })
        .unwrap();
        cx.update(|cx| assert_eq!(win.read(cx).section(), *expected));
        cx.update_window(handle.into(), |_, w, cx| w.render_frame(cx)).unwrap();
    }

    // the language select applies immediately
    cx.update_window(handle.into(), |_, w, cx| {
        win.update(cx, |v, cx| v.select(Section::General, cx));
        w.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| {
        opsbatch_app::language::set_mode(ops_i18n::LanguageMode::EnUs, cx);
        assert_eq!(ops_i18n::current_language(), ops_i18n::Language::EnUs);
    });
    wait_until(cx, "english stored", |_| stored(&fx).get("languageMode").map(|v| v == "en-US").unwrap_or(false));
    cx.update(|cx| opsbatch_app::language::set_mode(ops_i18n::LanguageMode::ZhCn, cx));
    wait_until(cx, "language stored", |_| stored(&fx).get("languageMode").map(|v| v == "zh-CN").unwrap_or(false));
}

#[gpui_kit::test]
fn appearance_changes_apply_immediately_and_persist(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let (handle, win) = open_settings(cx, Section::Appearance);
    let appearance = cx.update(|cx| win.read(cx).appearance().clone());
    let light_ix = ops_core::theme::THEMES.iter().position(|t| t.tone == ops_core::theme::ThemeTone::Light).unwrap();
    let light_id = ops_core::theme::THEMES[light_ix].id;

    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("set-theme", light_ix), cx);
        w.click(("set-accent", 3usize), cx);
        w.click("set-reduce-motion", cx);
    })
    .unwrap();
    cx.update(|cx| {
        let s = &opsbatch_app::theme::current(cx).settings;
        assert_eq!((s.terminal_theme_id.as_str(), s.terminal_theme_tone), (light_id, ops_core::theme::ThemeTone::Light));
        assert_eq!(s.accent_color_id, ops_core::theme::ACCENTS[3].id);
        assert!(s.reduce_motion);
        assert!(!opsbatch_app::theme::current(cx).resolved.is_dark);
    });

    // system mode, then a scale change reaches the global theme's font size
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("set-theme-system", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(opsbatch_app::theme::current(cx).settings.theme_mode, ops_core::theme::ThemeSettingsMode::System));
    cx.update(|cx| {
        opsbatch_app::theme::update(cx, |s| s.ui_scale = 125);
        assert!((f32::from(gpui_kit::component::Theme::global(cx).font_size) - 20.0).abs() < 0.01);
    });

    // typing in the numeric fields applies in-range values only
    cx.update_window(handle.into(), |_, w, cx| {
        let size = appearance.read(cx).size_input().clone();
        size.update(cx, |s, cx| {
            s.set_value("", w, cx);
            s.focus(w, cx)
        });
        w.input("18", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(opsbatch_app::theme::current(cx).settings.terminal_font_size, 18));
    cx.update_window(handle.into(), |_, w, cx| {
        let size = appearance.read(cx).size_input().clone();
        size.update(cx, |s, cx| {
            s.set_value("", w, cx);
            s.focus(w, cx)
        });
        w.input("99", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(opsbatch_app::theme::current(cx).settings.terminal_font_size, 18, "out of range ignored"));

    // everything reached the database
    wait_until(cx, "theme persisted", |_| {
        let m = stored(&fx);
        m.get("terminalFontSize").map(|v| v == "18").unwrap_or(false) && m.get("uiScale").map(|v| v == "125").unwrap_or(false)
    });
    let m = stored(&fx);
    assert_eq!(m["terminalThemeId"], light_id);
    assert_eq!(m["themeMode"], "system");
    assert_eq!(m["reduceMotion"], "true");
    assert_eq!(m["accentColorId"], ops_core::theme::ACCENTS[3].id);

    // font list comes from the machine
    wait_until(cx, "fonts", |cx| cx.update(|cx| appearance.read(cx).fonts_loaded()));
    cx.update(|cx| assert!(appearance.read(cx).font_count() >= 9));
}

#[gpui_kit::test]
fn danger_rules_add_validate_toggle_delete_and_data_export(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let (handle, win) = open_settings(cx, Section::Danger);
    let danger = cx.update(|cx| win.read(cx).danger().clone());
    wait_until(cx, "rules loaded", |cx| cx.update(|cx| danger.read(cx).loaded() && !danger.read(cx).rules().is_empty()));
    let builtin = cx.update(|cx| danger.read(cx).rules().len());
    assert!(cx.update(|cx| danger.read(cx).rules().iter().all(|r| r.is_builtin)));

    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("danger-toggle", 0usize), cx);
    })
    .unwrap();
    cx.update(|cx| assert!(!danger.read(cx).rules()[0].enabled, "switch applies at once"));
    let first = cx.update(|cx| danger.read(cx).rules()[0].id.clone());
    wait_until(cx, "built-in rule disabled in the database", |_| ops_core::rt::block_on(ops_core::settings::list_danger_rules(&fx.ctx.db)).unwrap().iter().any(|r| r.id == first && !r.enabled));
    // dialog: empty -> required, bad regex -> backend message, good -> added
    let dlg = cx.update_window(handle.into(), |_, w, cx| danger.update(cx, |d, cx| d.open_add(w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| dlg.update(cx, |d, cx| d.submit(w, cx))).unwrap();
    cx.update(|cx| assert_eq!(dlg.read(cx).errors(), ["name", "pattern"]));
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.name.update(cx, |s, cx| s.set_value("Wipe disk", w, cx));
            d.pattern.update(cx, |s, cx| s.set_value("dd if=(", w, cx));
            d.submit(w, cx);
        })
    })
    .unwrap();
    wait_until(cx, "regex rejected", |cx| cx.update(|cx| dlg.read(cx).backend_error().is_some()));
    cx.update(|cx| assert!(danger.read(cx).rules().len() == builtin));
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.pattern.update(cx, |s, cx| s.set_value(r"dd\s+if=", w, cx));
            d.submit(w, cx);
        })
    })
    .unwrap();
    wait_until(cx, "rule added", |cx| cx.update(|cx| danger.read(cx).rules().len() == builtin + 1));
    let custom = cx.update(|cx| danger.read(cx).rules().iter().find(|r| !r.is_builtin).cloned().unwrap());
    assert_eq!((custom.name.as_str(), custom.pattern.as_str(), custom.enabled), ("Wipe disk", r"dd\s+if=", true));

    // enable / disable of a custom rule
    cx.update_window(handle.into(), |_, w, cx| danger.update(cx, |d, cx| d.toggle(custom.id.clone(), false, w, cx))).unwrap();
    wait_until(cx, "custom rule disabled", |_| ops_core::rt::block_on(ops_core::settings::list_danger_rules(&fx.ctx.db)).unwrap().iter().any(|r| r.id == custom.id && !r.enabled));

    // delete (builtin rows have no delete button)
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("danger-delete", 0usize)).is_none());
        danger.update(cx, |d, cx| d.delete(custom.id.clone(), w, cx));
    })
    .unwrap();
    wait_until(cx, "rule deleted", |cx| cx.update(|cx| danger.read(cx).rules().len() == builtin));

    // data: backup export copies the database file
    let dest = std::env::temp_dir().join(format!("opsbatch-backup-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let data = cx.update(|cx| win.read(cx).data().clone());
    cx.update_window(handle.into(), |_, w, cx| data.update(cx, |d, cx| d.export_to(dest.display().to_string(), w, cx))).unwrap();
    wait_until(cx, "backup written", |_| dest.metadata().map(|m| m.len() > 0).unwrap_or(false));
    let _ = std::fs::remove_file(&dest);
    assert!(opsbatch_app::settings::data::DataSection::default_backup_name().starts_with("opsbatch-"));
}

#[gpui_kit::test]
fn settings_embeds_ai_and_library_sections(cx: &mut TestAppContext) {
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let (handle, win) = open_settings(cx, Section::Ai);
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("ai-tabs").is_some(), "AI tabs are embedded");
        win.update(cx, |v, cx| v.select(Section::CommandLib, cx));
        w.render_frame(cx);
        assert!(w.try_find("command-lib").is_some() && w.try_find("clib-add").is_some());
        win.update(cx, |v, cx| v.select(Section::ScriptLib, cx));
        w.render_frame(cx);
        assert!(w.try_find("script-lib").is_some());
        win.update(cx, |v, cx| v.select(Section::QuickActions, cx));
        w.render_frame(cx);
        assert!(w.try_find("quick-actions").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn animations_stop_when_reduce_motion_is_on_or_the_os_asks_for_it(cx: &mut TestAppContext) {
    use opsbatch_app::workbench::motion;
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    cx.update(|cx| assert!(!motion::reduced(cx)));
    // the main window renders (and animates) page switches without trouble
    for mode in ["mode-github", "mode-workflow", "mode-terminal"] {
        cx.update_window(fx.handle.into(), |_, w, cx| {
            w.render_frame(cx);
            w.click(mode, cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.executor().advance_clock(std::time::Duration::from_millis(300));
    }
    cx.update(|cx| {
        opsbatch_app::theme::update(cx, |s| s.reduce_motion = true);
        assert!(motion::reduced(cx), "the setting turns motion off");
        opsbatch_app::theme::update(cx, |s| s.reduce_motion = false);
        assert!(!motion::reduced(cx));
    });
}
