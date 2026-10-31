//! Managed windows: one registry keyed by label (settings, global log, editor per file, rdp/vnc per host, ...),
//! mirroring `open_managed_window` in the Tauri build. Sizes come from `commands/window.rs`.

use std::collections::HashMap;

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::{
    px, size, AnyWindowHandle, App, Bounds, Entity, Global, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    GlobalLog,
    BatchTerminal,
    BatchTransfer,
    Editor,
    Library,
    BatchCommands,
    Rdp,
    Vnc,
}

impl WindowKind {
    /// (width, height, min width, min height) in logical pixels.
    pub fn size(self) -> (f32, f32, f32, f32) {
        match self {
            WindowKind::Main => (1280.0, 800.0, 1024.0, 600.0),
            WindowKind::BatchTerminal => (1200.0, 800.0, 800.0, 600.0),
            WindowKind::BatchTransfer => (900.0, 700.0, 700.0, 550.0),
            WindowKind::Editor => (1100.0, 760.0, 760.0, 560.0),
            WindowKind::Library => (1100.0, 720.0, 800.0, 520.0),
            WindowKind::BatchCommands => (1100.0, 760.0, 800.0, 560.0),
            WindowKind::Settings => (900.0, 680.0, 760.0, 560.0),
            WindowKind::GlobalLog => (720.0, 600.0, 500.0, 400.0),
            WindowKind::Rdp | WindowKind::Vnc => (1280.0, 820.0, 760.0, 560.0),
        }
    }

    /// Remote-desktop windows must keep animating at full rate when unfocused (GPUI caps inactive windows at ~30 fps).
    pub fn keeps_full_frame_rate_when_inactive(self) -> bool {
        matches!(self, WindowKind::Rdp | WindowKind::Vnc)
    }
}

pub fn window_options(kind: WindowKind, title: &str, cx: &App) -> WindowOptions {
    let (w, h, min_w, min_h) = kind.size();
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(w), px(h)),
            cx,
        ))),
        window_min_size: Some(size(px(min_w), px(min_h))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            // the main window's own 38px bar: traffic lights sit where the original draws its 12px dots
            traffic_light_position: match kind {
                WindowKind::Main => Some(gpui_kit::point(px(16.), px(13.))),
                // the original draws its own 12px dots at (16,21) / (12,13.5) in these two
                WindowKind::Settings => Some(gpui_kit::point(px(16.), px(21.))),
                WindowKind::GlobalLog => Some(gpui_kit::point(px(12.), px(13.5))),
                _ => TitleBar::title_bar_options().traffic_light_position,
            },
            ..TitleBar::title_bar_options()
        }),
        inactive_frame_interval: if kind.keeps_full_frame_rate_when_inactive() {
            None
        } else {
            WindowOptions::default().inactive_frame_interval
        },
        ..TitleBar::window_options()
    }
}

#[derive(Default)]
pub struct WindowRegistry {
    windows: HashMap<String, AnyWindowHandle>,
}

impl Global for WindowRegistry {}

pub fn init(cx: &mut App) {
    cx.set_global(WindowRegistry::default());
}

/// Is a window with this label open?
pub fn is_open(label: &str, cx: &mut App) -> bool {
    let handle = cx.global::<WindowRegistry>().windows.get(label).copied();
    match handle {
        Some(h) => h.update(cx, |_, _, _| ()).is_ok(),
        None => false,
    }
}

pub fn open_labels(cx: &mut App) -> Vec<String> {
    let labels: Vec<String> = cx
        .global::<WindowRegistry>()
        .windows
        .keys()
        .cloned()
        .collect();
    labels.into_iter().filter(|l| is_open(l, cx)).collect()
}

/// Focus the window registered under `label`, or build a new one. Returns the handle either way.
pub fn open_or_focus<V: 'static + Render>(
    label: &str,
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<AnyWindowHandle> {
    let existing = cx.global::<WindowRegistry>().windows.get(label).copied();
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            cx.activate(true);
            return Some(handle);
        }
        cx.global_mut::<WindowRegistry>().windows.remove(label);
    }
    match gpui_kit::open_window(options, cx, build) {
        Ok((handle, _view)) => {
            cx.global_mut::<WindowRegistry>()
                .windows
                .insert(label.to_string(), handle);
            cx.activate(true);
            Some(handle)
        }
        Err(error) => {
            log::error!("failed to open window {label}: {error}");
            None
        }
    }
}

/// Opens the window under `label` without taking the focus; an open one is left as it is (activated only when
/// `activate` is set). For small status windows that must not interrupt what the user is typing.
pub fn open_background<V: 'static + Render>(
    label: &str,
    options: WindowOptions,
    activate: bool,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<AnyWindowHandle> {
    let existing = cx.global::<WindowRegistry>().windows.get(label).copied();
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| {
                if activate {
                    window.activate_window()
                }
            })
            .is_ok()
        {
            return Some(handle);
        }
        cx.global_mut::<WindowRegistry>().windows.remove(label);
    }
    match gpui_kit::open_window(options, cx, build) {
        Ok((handle, _view)) => {
            cx.global_mut::<WindowRegistry>()
                .windows
                .insert(label.to_string(), handle);
            Some(handle)
        }
        Err(error) => {
            log::error!("failed to open window {label}: {error}");
            None
        }
    }
}

/// Like [`open_or_focus`] but always shows fresh content: an existing window with this label is closed first
/// (windows whose context comes from their opener, e.g. the batch upload target list).
pub fn replace_window<V: 'static + Render>(
    label: &str,
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<AnyWindowHandle> {
    let existing = cx.global_mut::<WindowRegistry>().windows.remove(label);
    if let Some(handle) = existing {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    open_or_focus(label, options, cx, build)
}

#[allow(unused)]
fn _assert_root_is_used(_: Option<Root>) {}

/// Ask the main window to show the asset sidebar (what the original did with the `open-asset-manager` event).
pub fn request_open_assets(cx: &mut App) {
    cx.global_mut::<crate::ui_state::UiState>()
        .asset_panel_visible = true;
    let ctx = crate::services::ctx(cx);
    ctx.core
        .emit(OPEN_ASSET_MANAGER_TOPIC, serde_json::json!({}));
    crate::app::open_main_window(cx);
}

pub const OPEN_ASSET_MANAGER_TOPIC: &str = "open-asset-manager";
