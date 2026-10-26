//! Process-wide services (what Tauri kept in managed state), reachable from any view through `cx`.

use std::sync::Arc;

use gpui_kit::{App, Global};
use ops_ssh::Ctx;

pub struct Services {
    pub ctx: Arc<Ctx>,
}

impl Global for Services {}

pub fn init(ctx: Arc<Ctx>, cx: &mut App) {
    cx.set_global(Services { ctx });
}

pub fn ctx(cx: &App) -> Arc<Ctx> {
    cx.global::<Services>().ctx.clone()
}
