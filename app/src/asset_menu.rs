//! Context menus of the asset sidebar (hosts, groups, empty area) and the delete confirmations.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::*;
use ops_i18n::t;
use ops_ssh::hosts::Host;

use crate::asset_tree::{group_choices, HostKind, DEFAULT_GROUP_ID};
use crate::group_dialog::{GroupDialog, GroupDialogMode};
use crate::host_dialog::HostDialog;
use crate::stores::assets::AssetsStore;
use crate::widgets::toast;
use crate::workbench::asset_panel::{AssetPanel, AssetPanelEvent};

/// Everything a menu entry needs when it fires (cheap to clone into the menu builder).
#[derive(Clone)]
pub struct MenuEnv {
    pub store: Entity<AssetsStore>,
    pub panel: WeakEntity<AssetPanel>,
}

fn group_label(id: &str, store: &AssetsStore) -> String {
    if id == DEFAULT_GROUP_ID {
        if store.default_group_name.trim().is_empty() { t!("assets.defaultGroup") } else { store.default_group_name.clone() }
    } else {
        store.groups.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default()
    }
}

pub fn confirm_delete_host(env: &MenuEnv, host: &Host, window: &mut Window, cx: &mut App) {
    let store = env.store.clone();
    let id = host.id.clone();
    let name = host.name.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let store = store.clone();
        let id = id.clone();
        alert
            .confirm()
            .title(t!("assets.deleteHostAria"))
            .description(t!("assets.deleteHostNamedConfirm", name = name))
            .show_cancel(true)
            .ok_text(t!("common.delete"))
            .cancel_text(t!("common.cancel"))
            .on_ok(move |_, window, cx| {
                let task = store.update(cx, |s, cx| s.delete_host(id.clone(), cx));
                window
                    .spawn(cx, async move |cx| {
                        let result = task.await;
                        let _ = cx.update(|window, cx| match result {
                            Ok(()) => toast::success(window, cx, t!("assets.hostDeleted")),
                            Err(e) => toast::error(window, cx, t!("common.deleteFailed", error = e)),
                        });
                    })
                    .detach();
                true
            })
    });
}

pub fn confirm_delete_group(env: &MenuEnv, group_id: String, window: &mut Window, cx: &mut App) {
    let store = env.store.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let store = store.clone();
        let id = group_id.clone();
        alert
            .confirm()
            .title(t!("assets.deleteGroup"))
            .description(t!("assets.deleteGroupConfirm"))
            .show_cancel(true)
            .ok_text(t!("common.delete"))
            .cancel_text(t!("common.cancel"))
            .on_ok(move |_, window, cx| {
                let task = store.update(cx, |s, cx| s.delete_group(id.clone(), cx));
                window
                    .spawn(cx, async move |cx| {
                        let result = task.await;
                        let _ = cx.update(|window, cx| match result {
                            Ok(()) => toast::success(window, cx, t!("assets.groupDeleted")),
                            Err(e) => toast::error(window, cx, t!("common.deleteFailed", error = e)),
                        });
                    })
                    .detach();
                true
            })
    });
}

/// Move a host (used by the menu and by drag & drop). Shows a toast when it fails.
pub fn move_host(env: &MenuEnv, host_id: &str, group_id: Option<String>, window: &mut Window, cx: &mut App) {
    let task = env.store.update(cx, |s, cx| s.move_host(host_id, group_id, cx));
    if let Some(task) = task {
        window
            .spawn(cx, async move |cx| {
                if let Err(e) = task.await {
                    let _ = cx.update(|window, cx| toast::error(window, cx, t!("common.moveFailed", error = e)));
                }
            })
            .detach();
    }
}

pub fn host_menu(env: MenuEnv, host: Host, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let kind = HostKind::of(&host);
    let (open_label, open_icon) = match kind {
        HostKind::Vnc => (t!("assets.openVnc"), IconName::Server),
        HostKind::Windows => (t!("assets.openRdp"), IconName::Server),
        HostKind::Linux => (t!("assets.openTerminal"), IconName::Code),
    };
    let (groups, default_label, current_group) = {
        let s = env.store.read(cx);
        (group_choices(&s.groups, None), group_label(DEFAULT_GROUP_ID, s), host.group_id.clone().unwrap_or_else(|| DEFAULT_GROUP_ID.into()))
    };
    let store_labels: Vec<(String, String)> = {
        let s = env.store.read(cx);
        groups.iter().map(|(id, _)| (id.clone(), group_label(id, s))).collect()
    };

    let mut menu = menu.item(PopupMenuItem::new(open_label).icon(Icon::new(open_icon)).on_click({
        let env = env.clone();
        let id = host.id.clone();
        move |_, _, cx| {
            let id = id.clone();
            let _ = env.panel.update(cx, |_, cx| cx.emit(AssetPanelEvent::OpenHost(id)));
        }
    }));
    menu = menu.item(PopupMenuItem::new(t!("common.edit")).icon(Icon::new(IconName::SquarePen)).on_click({
        let env = env.clone();
        let host = host.clone();
        move |_, window, cx| HostDialog::open(env.store.clone(), Some(host.clone()), None, window, cx)
    }));
    menu = menu.separator();
    menu = menu.item(PopupMenuItem::new(default_label.clone()).icon(Icon::new(IconName::Folder)).checked(current_group == DEFAULT_GROUP_ID).on_click({
        let env = env.clone();
        let id = host.id.clone();
        move |_, window, cx| move_host(&env, &id, None, window, cx)
    }));
    for (gid, label) in store_labels.into_iter().filter(|(g, _)| g != DEFAULT_GROUP_ID) {
        let env = env.clone();
        let id = host.id.clone();
        let checked = gid == current_group;
        let gid2 = gid.clone();
        menu = menu.item(PopupMenuItem::new(label).icon(Icon::new(IconName::Folder)).checked(checked).on_click(move |_, window, cx| {
            move_host(&env, &id, Some(gid2.clone()), window, cx)
        }));
    }
    let _ = window;
    menu.separator().item(PopupMenuItem::new(t!("common.delete")).icon(Icon::new(IconName::Trash)).on_click({
        let env = env.clone();
        move |_, window, cx| confirm_delete_host(&env, &host, window, cx)
    }))
}

pub fn group_menu(env: MenuEnv, group_id: String, menu: PopupMenu, _window: &mut Window, _cx: &mut Context<PopupMenu>) -> PopupMenu {
    let is_default = group_id == DEFAULT_GROUP_ID;
    let parent = (!is_default).then(|| group_id.clone());
    let mut menu = menu;
    if !is_default {
        menu = menu.item(PopupMenuItem::new(t!("assets.addHostToGroup")).icon(Icon::new(IconName::Plus)).on_click({
            let env = env.clone();
            let gid = group_id.clone();
            move |_, window, cx| HostDialog::open(env.store.clone(), None, Some(gid.clone()), window, cx)
        }));
    }
    // "New group" and "New subgroup" both create a child of this group (top level for the default group), as before.
    for (label, icon) in [(t!("assets.newGroup"), IconName::FolderPlus), (t!("assets.newSubgroup"), IconName::FolderPlus)] {
        let env = env.clone();
        let parent = parent.clone();
        menu = menu.item(PopupMenuItem::new(label).icon(Icon::new(icon)).on_click(move |_, window, cx| {
            GroupDialog::open(env.store.clone(), GroupDialogMode::New { parent: parent.clone() }, window, cx)
        }));
    }
    menu = menu.separator();
    menu = menu.item(PopupMenuItem::new(t!("assets.editGroup")).icon(Icon::new(IconName::SquarePen)).on_click({
        let env = env.clone();
        let gid = group_id.clone();
        move |_, window, cx| {
            let mode = {
                let s = env.store.read(cx);
                if gid == DEFAULT_GROUP_ID {
                    GroupDialogMode::RenameDefault { name: group_label(&gid, s) }
                } else {
                    GroupDialogMode::Rename { id: gid.clone(), name: group_label(&gid, s) }
                }
            };
            GroupDialog::open(env.store.clone(), mode, window, cx)
        }
    }));
    if !is_default {
        menu = menu.item(PopupMenuItem::new(t!("assets.deleteGroup")).icon(Icon::new(IconName::Trash)).on_click({
            let env = env.clone();
            let gid = group_id.clone();
            move |_, window, cx| confirm_delete_group(&env, gid.clone(), window, cx)
        }));
    }
    menu
}

pub fn empty_menu(env: MenuEnv, menu: PopupMenu) -> PopupMenu {
    menu.item(PopupMenuItem::new(t!("assets.newGroup")).icon(Icon::new(IconName::FolderPlus)).on_click(move |_, window, cx| {
        GroupDialog::open(env.store.clone(), GroupDialogMode::New { parent: None }, window, cx)
    }))
}
