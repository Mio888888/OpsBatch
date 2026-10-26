//! Small pieces shared by the library pages: category rail, chips, icon action buttons, form helpers.

use std::collections::BTreeMap;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{h_flex, v_flex, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use super::model::Risk;

/// Left rail (`.clib-categories`): 156px, 6px padding, "All" plus one row per category with its count.
/// Row ids are `(id, index)`, index 0 = all.
pub fn category_rail(
    id: &'static str,
    total: usize,
    counts: &BTreeMap<String, usize>,
    selected: Option<&str>,
    cx: &App,
    on_select: impl Fn(Option<String>, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let ui = crate::theme::ui(cx);
    let row = |ix: usize, label: String, count: usize, active: bool, value: Option<String>| {
        let on_select = on_select.clone();
        // `.clib-cat`: 6px 10px, 6px corners, 13px text; the active one is primary + semibold on the nav-active tint
        h_flex()
            .id((id, ix))
            .test_support()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .items_center()
            .justify_between()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(13.))
            .line_height(relative(1.3))
            .text_color(if active { ui.primary } else { ui.text_secondary })
            .when(active, |d| d.font_weight(FontWeight::SEMIBOLD).bg(ui.primary.opacity(if ui.is_dark { 0.1 } else { 0.08 })))
            .when(!active, |d| d.hover(move |d| d.text_color(ui.text).bg(ui.text_muted.opacity(0.05))))
            .on_click(move |_, window, cx| on_select(value.clone(), window, cx))
            .child(div().min_w_0().truncate().child(label))
            .child(div().flex_none().min_w(px(16.)).text_size(px(10.)).text_center().text_color(if active { ui.primary } else { ui.text_muted }).child(count.to_string()))
    };
    v_flex()
        .id((id, 10_000usize))
        .w(px(156.))
        .flex_none()
        .h_full()
        .overflow_y_scroll()
        .p(px(6.))
        .gap(px(1.))
        .border_r_1()
        .border_color(ui.border)
        .bg(ui.card.blend(ui.text_muted.opacity(0.03)))
        .child(row(0, t!("library.category.all"), total, selected.is_none(), None))
        .children(counts.iter().enumerate().map(|(i, (cat, n))| row(i + 1, cat.clone(), *n, selected == Some(cat.as_str()), Some(cat.clone()))))
}

/// `.clib-risk`: 6px dot + 11px label, red text for high / critical.
pub fn risk_chip(risk: Risk, cx: &App) -> impl IntoElement {
    let ui = crate::theme::ui(cx);
    let (dot, text): (Hsla, Hsla) = match risk {
        Risk::Low => (rgb(0x52c41a).into(), ui.text_muted),
        Risk::Medium => (rgb(0xfaad14).into(), ui.text_muted),
        Risk::High | Risk::Critical => (rgb(0xf85149).into(), rgb(0xf85149).into()),
    };
    h_flex().gap(px(5.)).items_center().text_size(px(11.)).text_color(text).whitespace_nowrap().child(div().size(px(6.)).flex_none().rounded_full().bg(dot)).child(t!(risk.label_key()))
}

/// `.clib-row-tag`: 10px micro label on a 12% tint (platform style).
pub fn tag_chip(text: impl Into<SharedString>, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    div().flex_none().px(px(5.)).rounded(px(4.)).border_1().border_color(ui.text_muted.opacity(0.25)).bg(ui.text_muted.opacity(0.08)).text_color(ui.text_secondary).text_size(px(10.)).line_height(relative(1.4)).child(text.into())
}

/// Coloured micro label (`clib-row-tag-docker` blue, `clib-row-tag-remote` teal).
pub fn tag_chip_color(text: impl Into<SharedString>, color: u32) -> Div {
    let c: Hsla = rgb(color).into();
    div().flex_none().px(px(5.)).rounded(px(4.)).border_1().border_color(c.opacity(0.35)).bg(c.opacity(0.12)).text_color(c).text_size(px(10.)).line_height(relative(1.4)).child(text.into())
}

pub fn lang_label(language: &str) -> &'static str {
    match language {
        "python" => "Python",
        "powershell" => "PS",
        _ => "Shell",
    }
}

/// Row action (`.clib-act`): 26px square, muted icon, soft hover fill.
pub fn act_button(id: impl Into<ElementId>, icon: IconName, tooltip: String, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    act(id, icon, tooltip, false, on_click)
}

/// Like [`act_button`], turning red on hover (`.clib-act-danger`).
pub fn act_button_danger(id: impl Into<ElementId>, icon: IconName, tooltip: String, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    act(id, icon, tooltip, true, on_click)
}

fn act(id: impl Into<ElementId>, icon: IconName, tooltip: String, danger: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let danger_color: Hsla = rgb(0xf85149).into();
    div()
        .id(id.into())
        .test_support()
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .child(Icon::new(icon).size_3p5())
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(on_click)
        .map(|d| if danger { d.hover(move |d| d.text_color(danger_color).bg(danger_color.opacity(0.12))) } else { d.hover(|d| d.bg(gpui_kit::rgba(0x8080801a))) })
}

/// Search box of the top bar: 240px wide, may shrink to 140px.
pub fn search_box(state: &Entity<InputState>) -> impl IntoElement {
    use gpui_kit::component::input::Input;
    div().w(px(196.)).flex_shrink_0().child(Input::new(state).prefix(Icon::new(IconName::Search).size_3p5()).cleanable(true))
}

/// `.clib-empty`: centred muted text, 48/24 padding.
pub fn empty_state(text: String, cx: &App) -> Div {
    div().py(px(48.)).px(px(24.)).flex().justify_center().text_size(px(13.)).text_color(crate::theme::ui(cx).text_muted).child(text)
}

/// Page frame shared by the command / docker / script / quick-action libraries (`.clib`): top bar with search, count
/// chip and the add button, then a bordered card holding the category rail and the list.
pub fn page_frame(id: &'static str, search: impl IntoElement, count: String, add: impl IntoElement, rail: impl IntoElement, list: impl IntoElement, cx: &App) -> impl IntoElement {
    let ui = crate::theme::ui(cx);
    v_flex()
        .id(id)
        .test_support()
        .size_full()
        .gap(px(10.))
        .child(
            h_flex()
                .flex_none()
                .gap(px(8.))
                .pb(px(2.))
                .items_center()
                .child(search)
                .child(div().flex_1())
                .child(div().px(px(8.)).py(px(2.)).rounded(px(5.)).bg(ui.text_muted.opacity(0.05)).text_size(px(12.)).text_color(ui.text_muted).whitespace_nowrap().child(count))
                .child(add),
        )
        .child(h_flex().flex_1().min_h_0().overflow_hidden().rounded(px(14.)).border_1().border_color(ui.border).bg(ui.card).child(rail).child(list))
}

pub fn copy_to_clipboard(text: String, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
}

/// An action that can be switched off (greyed to 30% and inert).
pub fn act_button_enabled(id: impl Into<ElementId>, icon: IconName, tooltip: String, enabled: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    act(id, icon, tooltip, false, move |e, w, cx| {
        if enabled {
            on_click(e, w, cx)
        }
    })
    .into_any_element()
    .opacity_if(!enabled)
}

trait OpacityIf {
    fn opacity_if(self, dim: bool) -> AnyElement;
}

impl OpacityIf for AnyElement {
    fn opacity_if(self, dim: bool) -> AnyElement {
        if dim {
            div().opacity(0.3).child(self).into_any_element()
        } else {
            self
        }
    }
}
