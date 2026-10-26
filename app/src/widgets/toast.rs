//! Toasts (`message.success / error / warning` in the Tauri build).

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, Window};

pub fn success(window: &mut Window, cx: &mut App, text: impl Into<gpui_kit::SharedString>) {
    window.push_notification(Notification::success(text), cx);
}

pub fn error(window: &mut Window, cx: &mut App, text: impl Into<gpui_kit::SharedString>) {
    let text = text.into();
    // like `logHandledError` in the Tauri build: errors the user sees are also in the global log
    crate::logging::log_handled_error("ui.error", &text, "error");
    window.push_notification(Notification::error(text), cx);
}

pub fn warning(window: &mut Window, cx: &mut App, text: impl Into<gpui_kit::SharedString>) {
    window.push_notification(Notification::warning(text), cx);
}

pub fn info(window: &mut Window, cx: &mut App, text: impl Into<gpui_kit::SharedString>) {
    window.push_notification(Notification::info(text), cx);
}
