//! Building blocks shared by the AI settings tabs (`SettingsSection`, `SettingCard`, `SettingRow` of Netcatty).

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub fn green() -> Hsla {
    rgb(0x10b981).into()
}

pub fn amber() -> Hsla {
    rgb(0xf59e0b).into()
}

/// `SettingsSection`: bold title with optional actions on the right, content below.
pub fn section(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    actions: Option<AnyElement>,
    children: Vec<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    use gpui_kit::base::TestSupportExt as _;
    let ui = crate::theme::ui(cx);
    v_flex()
        .id(id.into())
        .test_support()
        .gap(px(10.))
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .line_height(relative(1.3))
                        .text_color(ui.text)
                        .child(title.into()),
                )
                .when_some(actions, |d, a| d.child(a)),
        )
        .children(children)
}

/// `SettingCard`: bordered rounded box; with `divided` its rows are separated by hairlines.
pub fn card(rows: Vec<AnyElement>, divided: bool, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .w_full()
        .rounded(px(12.))
        .border_1()
        .border_color(ui.border)
        .bg(ui.card)
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(move |(i, row)| {
            div()
                .w_full()
                .px(px(16.))
                .py(px(12.))
                .when(divided && i > 0, |d| d.border_t_1().border_color(ui.border))
                .child(row)
        }))
}

/// A card whose content is laid out by the caller.
pub fn padded_card(children: Vec<AnyElement>, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .w_full()
        .gap(px(10.))
        .p(px(16.))
        .rounded(px(12.))
        .border_1()
        .border_color(ui.border)
        .bg(ui.card)
        .children(children)
}

/// `SettingRow`: label + description on the left, the control on the right.
pub fn setting_row(
    label: Option<String>,
    desc: Option<String>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let ui = crate::theme::ui(cx);
    h_flex()
        .w_full()
        .gap(px(16.))
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .when_some(label, |d, l| {
                    d.child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(relative(1.3))
                            .text_color(ui.text)
                            .child(l),
                    )
                })
                .when_some(desc, |d, desc| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(ui.text_muted)
                            .child(desc),
                    )
                }),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A label above a full-width control (the stacked layout of the provider form).
pub fn stacked(
    label: impl Into<SharedString>,
    hint: Option<String>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .gap(px(4.))
        .min_w_0()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui.text_secondary)
                .child(label.into()),
        )
        .child(control)
        .when_some(hint, |d, h| {
            d.child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.4))
                    .text_color(ui.text_muted)
                    .child(h),
            )
        })
}

pub fn muted_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(crate::theme::ui(cx).text_muted)
        .child(text.into())
}

pub fn colored_text(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(color)
        .child(text.into())
}

/// A switch bound to `on_change`.
pub fn switch(
    id: impl Into<ElementId>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .on_click(move |c, w, cx| on_change(*c, w, cx))
}

/// A text input of the given width.
pub fn input(state: &Entity<InputState>, width: f32) -> Div {
    div().w(px(width)).child(Input::new(state))
}

pub fn mono_input(state: &Entity<InputState>) -> Div {
    div()
        .w_full()
        .font_family("Menlo")
        .text_size(px(12.))
        .child(Input::new(state))
}

pub fn textarea(state: &Entity<TextareaState>) -> Div {
    div()
        .w_full()
        .font_family("Menlo")
        .text_size(px(12.))
        .child(Textarea::new(state))
}

/// Calls `f` with the field's text when it loses focus or Enter is pressed.
pub fn on_commit<V: 'static>(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<V>,
    f: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        move |this, input, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                f(this, text, window, cx);
            }
        },
    )
}

pub fn set_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        input.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
    }
}

pub fn set_textarea(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        input.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
    }
}

/// A small outlined button with an optional icon.
pub fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
) -> Button {
    Button::new(id)
        .small()
        .label(label)
        .when_some(icon, |b, i| b.icon(Icon::new(i)))
}

/// Segmented tab bar (`TabsList` / `TabsTrigger`).
pub fn tab_bar<T: Copy + PartialEq + 'static>(
    id: &'static str,
    tabs: &[(T, String)],
    active: T,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static + Clone,
    cx: &App,
) -> impl IntoElement {
    use gpui_kit::base::TestSupportExt as _;
    let ui = crate::theme::ui(cx);
    let theme = cx.theme();
    h_flex()
        .id(id)
        .test_support()
        .flex_wrap()
        .gap(px(2.))
        .p(px(3.))
        .rounded(px(10.))
        .bg(ui.input_bg)
        .w_auto()
        .children(tabs.iter().enumerate().map(|(ix, (tab, label))| {
            let (tab, selected) = (*tab, *tab == active);
            let on_select = on_select.clone();
            div()
                .id((id, ix))
                .test_support()
                .px(px(14.))
                .py(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if selected { ui.text } else { ui.text_muted })
                .when(selected, |d| d.bg(theme.background).shadow_sm())
                .when(!selected, |d| d.hover(move |d| d.text_color(ui.text)))
                .child(label.clone())
                .on_click(move |_, w, cx| on_select(tab, w, cx))
        }))
}

/// Hairline-separated list item container used by provider / agent cards.
pub fn list_card(active: bool, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .w_full()
        .gap(px(12.))
        .p(px(16.))
        .rounded(px(12.))
        .border_1()
        .when(active, |d| {
            d.border_color(ui.primary.opacity(0.5))
                .bg(ui.primary.opacity(0.05))
        })
        .when(!active, |d| d.border_color(ui.border).bg(ui.card))
}

pub fn pill(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded_full()
        .bg(color.opacity(0.15))
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(text.into())
}

/// Opens a folder in the platform file manager.
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(path).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("explorer").arg(path).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(path).status();
    status.map(|_| ()).map_err(|e| e.to_string())
}

/// Parses a whole number from a field, or `None` for empty / invalid text.
pub fn parse_int(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok()
}

/// A read-only code block with a copy button (`CopyableCodeBlock`).
pub struct CopyBlock {
    id: SharedString,
    label: Option<String>,
    text: String,
    copied: bool,
}

impl CopyBlock {
    pub fn new(id: impl Into<SharedString>, label: Option<String>, text: String) -> Self {
        Self {
            id: id.into(),
            label,
            text,
            copied: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.text != text {
            self.text = text;
            cx.notify();
        }
    }

    pub fn set_label(&mut self, label: Option<String>, cx: &mut Context<Self>) {
        if self.label != label {
            self.label = label;
            cx.notify();
        }
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if self.text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CopyBlock {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::base::TestSupportExt as _;
        let ui = crate::theme::ui(cx);
        let empty = self.text.is_empty();
        v_flex()
            .id(self.id.clone())
            .test_support()
            .gap(px(6.))
            .when_some(self.label.clone(), |d, l| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ui.text_muted)
                        .child(l),
                )
            })
            .child(
                div()
                    .relative()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(ui.border)
                    .bg(ui.text_muted.opacity(0.06))
                    .child(
                        div()
                            .max_h(px(160.))
                            .overflow_hidden()
                            .pl(px(12.))
                            .pr(px(40.))
                            .py(px(10.))
                            .font_family("Menlo")
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(if empty { ui.text_muted } else { ui.text })
                            .child(self.text.clone()),
                    )
                    .child(
                        div().absolute().top(px(6.)).right(px(6.)).child(
                            Button::new(SharedString::from(format!("{}-copy", self.id)))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(if self.copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                }))
                                .tooltip(if self.copied {
                                    ops_i18n::t!("ncai.externalMcp.copied")
                                } else {
                                    ops_i18n::t!("ncai.externalMcp.copy")
                                })
                                .disabled(empty)
                                .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                        ),
                    ),
            )
    }
}
