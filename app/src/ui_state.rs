//! Window-independent UI state that must survive the main window being closed to the tray and reopened.

use gpui_kit::{App, AppContext as _, Global};

use crate::workbench::Mode;

pub struct UiState {
    pub mode: Mode,
    pub asset_panel_visible: bool,
    pub assets: gpui_kit::Entity<crate::stores::assets::AssetsStore>,
}

impl Global for UiState {}

pub fn init(cx: &mut App) {
    let assets = cx.new(|_| crate::stores::assets::AssetsStore::new());
    cx.set_global(UiState {
        mode: Mode::Terminal,
        asset_panel_visible: false,
        assets,
    });
}
