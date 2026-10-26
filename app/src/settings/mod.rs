//! Settings window (M10-01…03): general, appearance, libraries, AI, danger rules, data.

pub mod appearance;
pub mod danger;
pub mod data;
pub mod general;
pub mod window;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;

/// Titled group of rows (`.settings-general-group`): card colour, hairline border, 14px corners, 16/18 padding.
pub(crate) fn group(title: impl Into<SharedString>, hint: impl Into<SharedString>, rows: Vec<AnyElement>, cx: &App) -> Div {
    group_with(title, hint, rows, true, cx)
}

/// [`group`] with optional hairlines between rows (the AI cards have none).
pub(crate) fn group_with(title: impl Into<SharedString>, hint: impl Into<SharedString>, rows: Vec<AnyElement>, dividers: bool, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .gap(px(10.))
        .py(px(16.))
        .px(px(18.))
        .bg(ui.card)
        .border_1()
        .border_color(ui.border)
        .rounded(px(14.))
        .child(
            v_flex()
                .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).line_height(relative(1.25)).text_color(ui.text).child(title.into()))
                .child(div().mt(px(2.)).text_size(px(12.)).line_height(relative(1.5)).text_color(ui.text_muted).child(hint.into())),
        )
        .child(v_flex().gap(px(4.)).mt(px(2.)).children(rows.into_iter().enumerate().map(|(i, row)| {
            // first row has no divider and only 2px of top padding (`.settings-general-row:first-child`)
            div().min_h(px(36.)).pb(px(6.)).pt(px(if i == 0 { 2. } else { 6. })).when(i > 0 && dividers, |d| d.border_t_1().border_color(ui.border)).child(row)
        })))
}

/// Row with the title/description on the left and the control on the right (`.settings-general-row`).
pub(crate) fn row(title: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement, cx: &App) -> AnyElement {
    let ui = crate::theme::ui(cx);
    h_flex()
        .gap(px(16.))
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).line_height(relative(1.3)).text_color(ui.text_secondary).child(title.into()))
                .when_some(desc, |d, desc| d.child(div().text_size(px(11.)).line_height(relative(1.35)).text_color(ui.text_muted).child(desc))),
        )
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}
