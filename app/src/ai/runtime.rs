//! The AI safety settings the chat panel consults on every turn (permission mode, command timeout, iteration cap).
//! Settings keep this in step with what they save; at startup it is read from the database.

use gpui_kit::{App, Global};
use ops_ai::prefs::RuntimeLimits;

struct AiRuntime(RuntimeLimits);
impl Global for AiRuntime {}

pub fn limits(cx: &App) -> RuntimeLimits {
    cx.try_global::<AiRuntime>().map(|r| r.0.clone()).unwrap_or_default()
}

pub fn set(cx: &mut App, limits: RuntimeLimits) {
    cx.set_global(AiRuntime(limits));
}

pub fn init(cx: &mut App) {
    set(cx, RuntimeLimits::default());
    let ctx = crate::services::ctx(cx);
    cx.spawn(async move |cx| {
        let loaded = ops_core::rt::run(async move { ops_ai::prefs::runtime_limits(&ctx.db) }).await;
        if let Ok(limits) = loaded {
            let _ = cx.update(|cx| set(cx, limits));
        }
    })
    .detach();
}
