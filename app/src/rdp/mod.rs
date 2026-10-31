//! RDP window and AI agent (M8-05, M8-06).

pub mod agent;
pub mod ai_panel;
pub mod input;
pub mod window;

use std::sync::{Arc, OnceLock};

use gpui_kit::App;
use ops_rdp::RdpManager;

static MANAGER: OnceLock<Arc<RdpManager>> = OnceLock::new();

/// The process-wide RDP session manager (sessions outlive any single view, like the Tauri managed state).
pub fn global_manager() -> Arc<RdpManager> {
    MANAGER.get_or_init(|| Arc::new(RdpManager::new())).clone()
}

pub fn rdp_manager(_cx: &App) -> Arc<RdpManager> {
    global_manager()
}
