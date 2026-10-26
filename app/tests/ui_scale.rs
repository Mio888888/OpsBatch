//! The interface scale zooms fixed-pixel controls too (the Tauri build zooms the whole webview).

mod common;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};

#[gpui_kit::test]
fn fixed_pixel_widths_follow_the_interface_scale(cx: &mut TestAppContext) {
    let _s = common::serial();
    let fx = common::setup(cx, |_| {});
    let width = |cx: &mut TestAppContext, fx: &common::Fixture| -> f32 {
        cx.update_window(fx.handle.into(), |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.find("asset-panel").bounds().size.width)
        })
        .unwrap()
    };
    common::open_sidebar(cx, &fx, 0);
    assert_eq!(width(cx, &fx), 380.0, "100%: the asset panel is 380px wide");
    cx.update(|cx| {
        opsbatch_app::scale::set_percent(125);
        cx.refresh_windows();
    });
    assert_eq!(width(cx, &fx), 475.0, "125%: 380 * 1.25");
    cx.update(|_| opsbatch_app::scale::set_percent(100));
}
