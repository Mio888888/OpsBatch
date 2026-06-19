//! DevTools 切换命令，仅用于辅助开发调试。
//!
//! 在 debug 构建下会切换当前窗口的 WebView 开发者工具；
//! release 构建下为空操作，避免暴露调试能力。

use tauri::WebviewWindow;

/// 切换调用该命令的窗口的 DevTools。
///
/// 前端在开发环境通过快捷键（F12 / Ctrl+Shift+I / Cmd+Option+I）触发，
/// 命令会作用于当前所在的 WebView 窗口，主窗口与所有子窗口通用。
#[tauri::command]
pub fn toggle_devtools(window: WebviewWindow) -> Result<(), String> {
    #[cfg(debug_assertions)]
    {
        let is_open = window.is_devtools_open();
        if is_open {
            window.close_devtools();
        } else {
            window.open_devtools();
        }
    }
    #[cfg(not(debug_assertions))]
    {
        // release 构建下不提供 DevTools，静默忽略
        let _ = &window;
    }
    Ok(())
}
