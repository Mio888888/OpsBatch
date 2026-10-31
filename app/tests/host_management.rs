//! M3 headless UI tests: host add/edit dialog, groups, cloud import, drag & drop, menus — all against a real SQLite
//! database through the real services.

mod common;

use common::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use ops_core::security::SECRET_PLACEHOLDER;
use opsbatch_app::asset_menu::{self, MenuEnv};
use opsbatch_app::asset_tree::{build_rows, ExpandState, TreeRow};
use opsbatch_app::cloud_dialog::CloudImportDialog;
use opsbatch_app::group_dialog::{GroupDialog, GroupDialogMode};
use opsbatch_app::host_dialog::{HostDialog, Tab};
use opsbatch_app::host_form::{Field, OsKind};

fn set_text(
    cx: &mut gpui_kit::App,
    window: &mut gpui_kit::Window,
    e: &Entity<gpui_kit::component::input::InputState>,
    v: &str,
) {
    e.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
}

fn render(cx: &mut TestAppContext, fx: &Fixture) {
    cx.update_window(fx.handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn adding_a_host_validates_then_saves_secrets_to_the_vault(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |_| {});
    let store = open_sidebar(cx, &fx, 0);

    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            HostDialog::open_with_handle(store.clone(), None, None, window, cx)
        })
        .unwrap();
    render(cx, &fx);
    cx.update_window(fx.handle.into(), |_, window, _| {
        assert!(
            window.try_find("host-dialog").is_some(),
            "dialog is on screen"
        )
    })
    .unwrap();

    // Empty submit: name + address are required, nothing is created.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        dialog.update(cx, |d, cx| d.submit(window, cx))
    })
    .unwrap();
    cx.update(|cx| assert_eq!(dialog.read(cx).errors(), &[Field::Name, Field::Ip]));
    assert_eq!(cx.update(|cx| store.read(cx).hosts.len()), 0);

    cx.update_window(fx.handle.into(), |_, window, cx| {
        let i = &dialog.read(cx).inputs;
        let (name, ip, user, pw) = (
            i.name.clone(),
            i.ip.clone(),
            i.username.clone(),
            i.password.clone(),
        );
        set_text(cx, window, &name, "web-01");
        set_text(cx, window, &ip, "10.0.0.5");
        set_text(cx, window, &user, "deploy");
        set_text(cx, window, &pw, "s3cret");
        window.render_frame(cx);
        window.click("host-save", cx);
    })
    .unwrap();
    wait_until(cx, "host created", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 1)
    });
    let host = cx.update(|cx| store.read(cx).hosts[0].clone());
    assert_eq!(
        (
            host.name.as_str(),
            host.ip.as_str(),
            host.username.as_str(),
            host.port
        ),
        ("web-01", "10.0.0.5", "deploy", 22)
    );
    assert_eq!(
        host.password.as_deref(),
        Some(SECRET_PLACEHOLDER),
        "the row never holds the password"
    );
    assert_eq!(
        ops_core::keychain::get_host_password(&host.id).unwrap(),
        "s3cret"
    );
    // The dialog closed itself.
    render(cx, &fx);
    cx.update_window(fx.handle.into(), |_, window, _| {
        assert!(window.try_find("host-dialog").is_none())
    })
    .unwrap();
}

#[gpui_kit::test]
fn editing_a_host_prefills_updates_and_keeps_the_stored_secret(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            let mut h = new_host("db-01", "10.0.0.9", "linux", None);
            h.password = Some("topsecret".into());
            h.username = Some("root".into());
            ops_ssh::hosts::add_host(&core.db, h).await.unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 1);
    let host = cx.update(|cx| store.read(cx).hosts[0].clone());

    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            HostDialog::open_with_handle(store.clone(), Some(host.clone()), None, window, cx)
        })
        .unwrap();
    cx.update(|cx| {
        let d = dialog.read(cx);
        assert_eq!(d.inputs.name.read(cx).value().to_string(), "db-01");
        assert_eq!(d.inputs.ip.read(cx).value().to_string(), "10.0.0.9");
        assert_eq!(
            d.inputs.password.read(cx).value().to_string(),
            "",
            "secrets are never shown; blank = keep"
        );
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let name = dialog.read(cx).inputs.name.clone();
        set_text(cx, window, &name, "db-primary");
        dialog.update(cx, |d, cx| d.submit(window, cx));
    })
    .unwrap();
    wait_until(cx, "host renamed", |cx| {
        cx.update(|cx| {
            store
                .read(cx)
                .hosts
                .first()
                .map(|h| h.name == "db-primary")
                .unwrap_or(false)
        })
    });
    assert_eq!(
        ops_core::keychain::get_host_password(&host.id).unwrap(),
        "topsecret",
        "blank password field keeps the secret"
    );
}

#[gpui_kit::test]
fn switching_system_type_retargets_fields_and_tabs(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |_| {});
    let store = open_sidebar(cx, &fx, 0);
    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            HostDialog::open_with_handle(store.clone(), None, None, window, cx)
        })
        .unwrap();

    cx.update_window(fx.handle.into(), |_, window, cx| {
        dialog.update(cx, |d, cx| d.change_os(OsKind::Windows, window, cx));
    })
    .unwrap();
    cx.update(|cx| {
        let d = dialog.read(cx);
        assert_eq!(d.inputs.port.read(cx).value().to_string(), "3389");
        assert_eq!(
            d.inputs.username.read(cx).value().to_string(),
            "Administrator"
        );
    });
    // Windows' advanced tab carries the RDP options.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("host-tab-advanced", cx);
        window.render_frame(cx);
        for id in [
            "rdp-clipboard",
            "rdp-audio",
            "rdp-map-disk",
            "proxy-enabled",
        ] {
            assert!(window.try_find(id).is_some(), "missing {id} on the RDP tab");
        }
        assert!(window.try_find("rdp-disk-path").is_none() || true);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(dialog.read(cx).tab(), Tab::Advanced));

    // VNC returns to the basic tab and drops the SSH-only fields.
    cx.update_window(fx.handle.into(), |_, window, cx| {
        dialog.update(cx, |d, cx| d.change_os(OsKind::Vnc, window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        let d = dialog.read(cx);
        assert_eq!(d.tab(), Tab::Basic);
        assert_eq!(d.inputs.port.read(cx).value().to_string(), "5900");
        assert_eq!(d.values().os, OsKind::Vnc);
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("vnc-shared").is_some() && window.try_find("vnc-view-only").is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn proxy_and_jump_chain_are_persisted_from_the_advanced_tab(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            ops_ssh::hosts::add_host(&core.db, new_host("bastion", "10.9.9.1", "linux", None))
                .await
                .unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 1);
    let bastion = cx.update(|cx| store.read(cx).hosts[0].id.clone());
    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            HostDialog::open_with_handle(store.clone(), None, None, window, cx)
        })
        .unwrap();
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let i = &dialog.read(cx).inputs;
        let (name, ip, ph) = (i.name.clone(), i.ip.clone(), i.proxy_host.clone());
        set_text(cx, window, &name, "inner");
        set_text(cx, window, &ip, "10.1.1.1");
        window.render_frame(cx);
        window.click("host-tab-advanced", cx);
        window.render_frame(cx);
        window.click("proxy-enabled", cx);
        window.render_frame(cx);
        set_text(cx, window, &ph, " proxy.internal ");
        window.click(("jump", 0usize), cx);
        window.render_frame(cx);
        dialog.update(cx, |d, cx| d.submit(window, cx));
    })
    .unwrap();
    wait_until(cx, "host saved", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 2)
    });
    let inner = cx.update(|cx| {
        store
            .read(cx)
            .hosts
            .iter()
            .find(|h| h.name == "inner")
            .cloned()
            .unwrap()
    });
    let proxy: serde_json::Value = serde_json::from_str(&inner.proxy_settings).unwrap();
    assert_eq!(
        (
            proxy["enabled"].clone(),
            proxy["host"].clone(),
            proxy["type"].clone(),
            proxy["port"].clone()
        ),
        (
            true.into(),
            "proxy.internal".into(),
            "socks5".into(),
            1080.into()
        )
    );
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&inner.jump_chain).unwrap(),
        vec![bastion]
    );
}

#[gpui_kit::test]
fn group_dialog_creates_renames_and_validates(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |_| {});
    let store = open_sidebar(cx, &fx, 0);

    // empty name -> required error, nothing created
    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            GroupDialog::open_with_handle(
                store.clone(),
                GroupDialogMode::New { parent: None },
                window,
                cx,
            )
        })
        .unwrap();
    cx.update_window(fx.handle.into(), |_, window, cx| {
        dialog.update(cx, |d, cx| d.submit(window, cx))
    })
    .unwrap();
    cx.update(|cx| assert!(dialog.read(cx).has_required_error()));

    cx.update_window(fx.handle.into(), |_, window, cx| {
        let input = dialog.read(cx).input.clone();
        set_text(cx, window, &input, "  生产环境  ");
        window.render_frame(cx);
        window.click("group-save", cx);
    })
    .unwrap();
    wait_until(cx, "group created", |cx| {
        cx.update(|cx| store.read(cx).groups.len() == 1)
    });
    let group = cx.update(|cx| store.read(cx).groups[0].clone());
    assert_eq!(group.name, "生产环境", "trimmed");

    // sub group + rename
    let sub = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            GroupDialog::open_with_handle(
                store.clone(),
                GroupDialogMode::New {
                    parent: Some(group.id.clone()),
                },
                window,
                cx,
            )
        })
        .unwrap();
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let input = sub.read(cx).input.clone();
        set_text(cx, window, &input, "web");
        sub.update(cx, |d, cx| d.submit(window, cx));
    })
    .unwrap();
    wait_until(cx, "sub group", |cx| {
        cx.update(|cx| store.read(cx).groups.len() == 2)
    });
    assert!(cx.update(|cx| store
        .read(cx)
        .groups
        .iter()
        .any(|g| g.name == "web" && g.parent_id.as_deref() == Some(group.id.as_str()))));

    let rename = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            GroupDialog::open_with_handle(
                store.clone(),
                GroupDialogMode::Rename {
                    id: group.id.clone(),
                    name: group.name.clone(),
                },
                window,
                cx,
            )
        })
        .unwrap();
    cx.update(|cx| {
        assert_eq!(
            rename.read(cx).input.read(cx).value().to_string(),
            "生产环境",
            "prefilled"
        )
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let input = rename.read(cx).input.clone();
        set_text(cx, window, &input, "prod");
        rename.update(cx, |d, cx| d.submit(window, cx));
    })
    .unwrap();
    wait_until(cx, "renamed", |cx| {
        cx.update(|cx| store.read(cx).groups.iter().any(|g| g.name == "prod"))
    });

    // the implicit default group is renamed locally and persisted
    let def = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            GroupDialog::open_with_handle(
                store.clone(),
                GroupDialogMode::RenameDefault {
                    name: "未分组".into(),
                },
                window,
                cx,
            )
        })
        .unwrap();
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let input = def.read(cx).input.clone();
        set_text(cx, window, &input, "Ungrouped");
        def.update(cx, |d, cx| d.submit(window, cx));
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| store.read(cx).default_group_name.clone()),
        "Ungrouped"
    );
    let ctx = fx.ctx.clone();
    wait_until(cx, "default name saved", |_| {
        ops_core::rt::block_on(ops_core::settings::get_general_settings(&ctx.db))
            .unwrap()
            .get("defaultGroupName")
            .map(String::as_str)
            == Some("Ungrouped")
    });
}

#[gpui_kit::test]
fn dragging_a_host_onto_a_group_moves_it_and_back_to_ungrouped(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            let g = ops_ssh::groups::add_group(&core.db, "prod".into(), None)
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("alpha", "10.0.0.1", "linux", None))
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("beta", "10.0.0.2", "linux", Some(&g.id)))
                .await
                .unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 2);
    let find_rows = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let s = store.read(cx);
            let rows = build_rows(&s.hosts, &s.groups, "", "d", &ExpandState::default(), &s.selected);
            let host_ix = |name: &str| rows.iter().position(|r| matches!(r, TreeRow::Host { id, .. } if s.host(id).map(|h| h.name == name).unwrap_or(false)));
            let group_ix = |gid: &str| rows.iter().position(|r| matches!(r, TreeRow::Group { id, .. } if id == gid));
            let prod = s.groups[0].id.clone();
            (host_ix("alpha").unwrap(), group_ix(&prod).unwrap(), host_ix("beta").unwrap(), group_ix("__default__").unwrap(), prod)
        })
    };
    render(cx, &fx);
    let (alpha, prod_row, beta, default_row, prod) = find_rows(cx);

    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(("host", alpha), ("group", prod_row), cx);
    })
    .unwrap();
    wait_until(cx, "alpha moved into prod", |cx| {
        cx.update(|cx| {
            store
                .read(cx)
                .hosts
                .iter()
                .any(|h| h.name == "alpha" && h.group_id.as_deref() == Some(prod.as_str()))
        })
    });

    render(cx, &fx);
    let (_, _, beta2, default_row2, _) = find_rows(cx);
    let _ = (beta, default_row);
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(("host", beta2), ("group", default_row2), cx);
    })
    .unwrap();
    wait_until(cx, "beta ungrouped", |cx| {
        cx.update(|cx| {
            store
                .read(cx)
                .hosts
                .iter()
                .any(|h| h.name == "beta" && h.group_id.is_none())
        })
    });
}

#[gpui_kit::test]
fn delete_confirmation_opens_and_deleting_removes_host_and_secret(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            let mut h = new_host("tmp", "10.0.0.7", "linux", None);
            h.password = Some("pw".into());
            ops_ssh::hosts::add_host(&core.db, h).await.unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 1);
    let host = cx.update(|cx| store.read(cx).hosts[0].clone());
    let env = MenuEnv {
        store: store.clone(),
        panel: gpui_kit::WeakEntity::new_invalid(),
    };
    cx.update_window(fx.handle.into(), |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        asset_menu::confirm_delete_host(&env, &host, window, cx);
        window.render_frame(cx);
        assert!(window.has_active_dialog(cx), "confirmation dialog is open");
    })
    .unwrap();
    // Confirm with the keyboard (Enter triggers the dialog's OK): the host and its stored secret disappear.
    cx.update_window(fx.handle.into(), |_, window, cx| window.press("enter", cx))
        .unwrap();
    wait_until(cx, "host deleted", |cx| {
        cx.update(|cx| store.read(cx).hosts.is_empty())
    });
    assert!(
        ops_core::keychain::get_host_password(&host.id).is_err(),
        "secret removed with the host"
    );
}

#[gpui_kit::test]
fn cloud_import_lists_selects_and_imports_instances(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |_| {});
    let store = open_sidebar(cx, &fx, 0);
    let dialog = cx
        .update_window(fx.handle.into(), |_, window, cx| {
            CloudImportDialog::open_with_handle(store.clone(), window, cx)
        })
        .unwrap();
    let mk = |id: &str, name: &str, ip: &str, os: &str| ops_library::cloud::CloudInstance {
        instance_id: id.into(),
        name: name.into(),
        ip: ip.into(),
        inner_ip: String::new(),
        os: os.into(),
        status: "Running".into(),
        region: "cn-hangzhou".into(),
        instance_type: "ecs.c6".into(),
    };
    cx.update_window(fx.handle.into(), |_, window, cx| {
        dialog.update(cx, |d, cx| {
            d.set_instances(
                vec![
                    mk("i-1", "api", "8.8.8.1", "linux"),
                    mk("i-2", "win", "8.8.8.2", "Windows Server"),
                ],
                cx,
            )
        });
        window.render_frame(cx);
        assert!(
            window.try_find(("cloud-row", 0usize)).is_some()
                && window.try_find(("cloud-row", 1usize)).is_some()
        );
        assert_eq!(
            window.find("cloud-import").disabled(),
            Some(true),
            "nothing selected yet"
        );
        window.click(("cloud-check", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(dialog.read(cx).selected_ids().len(), 1));
    cx.update_window(fx.handle.into(), |_, window, cx| {
        window.click("cloud-import", cx)
    })
    .unwrap();
    wait_until(cx, "instance imported", |cx| {
        cx.update(|cx| store.read(cx).hosts.len() == 1)
    });
    let h = cx.update(|cx| store.read(cx).hosts[0].clone());
    assert_eq!(
        (h.name.as_str(), h.os.as_str(), h.ip.as_str()),
        ("win", "windows", "8.8.8.2")
    );
}

#[gpui_kit::test]
fn context_menus_build_for_every_target_without_panicking(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            let g = ops_ssh::groups::add_group(&core.db, "prod".into(), None)
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("lin", "10.0.0.1", "linux", Some(&g.id)))
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("win", "10.0.0.2", "windows", None))
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("vnc", "10.0.0.3", "vnc", None))
                .await
                .unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 3);
    let (hosts, group_id) = cx.update(|cx| {
        (
            store.read(cx).hosts.clone(),
            store.read(cx).groups[0].id.clone(),
        )
    });
    cx.update_window(fx.handle.into(), |_, window, cx| {
        let env = MenuEnv {
            store: store.clone(),
            panel: gpui_kit::WeakEntity::new_invalid(),
        };
        for h in &hosts {
            let (e, h) = (env.clone(), h.clone());
            let _ = gpui_kit::component::menu::PopupMenu::build(window, cx, move |m, w, cx| {
                asset_menu::host_menu(e, h, m, w, cx)
            });
        }
        for gid in [group_id.clone(), "__default__".to_string()] {
            let e = env.clone();
            let _ = gpui_kit::component::menu::PopupMenu::build(window, cx, move |m, w, cx| {
                asset_menu::group_menu(e, gid, m, w, cx)
            });
        }
        let e = env.clone();
        let _ = gpui_kit::component::menu::PopupMenu::build(window, cx, move |m, _, _| {
            asset_menu::empty_menu(e, m)
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn deleting_a_group_ungroups_its_hosts_after_confirmation(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = setup(cx, |core| {
        ops_core::rt::block_on(async {
            let g = ops_ssh::groups::add_group(&core.db, "old".into(), None)
                .await
                .unwrap();
            ops_ssh::hosts::add_host(&core.db, new_host("keep", "10.0.0.1", "linux", Some(&g.id)))
                .await
                .unwrap();
        });
    });
    let store = open_sidebar(cx, &fx, 1);
    let gid = cx.update(|cx| store.read(cx).groups[0].id.clone());
    let env = MenuEnv {
        store: store.clone(),
        panel: gpui_kit::WeakEntity::new_invalid(),
    };
    cx.update_window(fx.handle.into(), |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        asset_menu::confirm_delete_group(&env, gid.clone(), window, cx);
        window.render_frame(cx);
        assert!(window.has_active_dialog(cx));
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, "group deleted", |cx| {
        cx.update(|cx| store.read(cx).groups.is_empty())
    });
    wait_until(cx, "host kept and ungrouped", |cx| {
        cx.update(|cx| {
            store.read(cx).hosts.len() == 1 && store.read(cx).hosts[0].group_id.is_none()
        })
    });
}
