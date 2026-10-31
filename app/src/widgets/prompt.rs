//! Single-line text prompt dialog (new file / rename / ...), replacing `prompt()` and small antd Modals.

use std::rc::Rc;

use crate::scale::px;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, WindowExt as _};
use gpui_kit::*;
use ops_i18n::t;

use super::toast;

pub type SubmitFn = Rc<dyn Fn(String, &mut Window, &mut App) -> Task<Result<(), String>>>;

pub struct TextPrompt {
    input: Entity<InputState>,
    busy: bool,
    on_submit: SubmitFn,
    _sub: Subscription,
}

impl TextPrompt {
    /// Open the dialog; `on_submit` receives the trimmed, non-empty text and reports success or an error toast.
    pub fn open(
        title: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        initial: &str,
        on_submit: SubmitFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let title: SharedString = title.into();
        let placeholder: SharedString = placeholder.into();
        let initial = initial.to_string();
        let view = cx.new(|cx| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(initial)
            });
            let sub = cx.subscribe_in(
                &input,
                window,
                |this: &mut Self, _, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        this.submit(window, cx);
                    }
                },
            );
            Self {
                input,
                busy: false,
                on_submit,
                _sub: sub,
            }
        });
        let shown = view.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title.clone()).w(px(420.)).child(shown.clone())
        });
        // The dialog takes focus when it opens; focus the field once that has happened.
        let input = view.read(cx).input.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        view
    }

    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if self.busy || text.is_empty() {
            return;
        }
        self.busy = true;
        cx.notify();
        let task = (self.on_submit)(text, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => window.close_dialog(cx),
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for TextPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.input.read(cx).value().trim().is_empty();
        v_flex()
            .id("text-prompt")
            .test_support()
            .gap_3()
            .child(Input::new(&self.input))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("prompt-cancel")
                            .label(t!("common.cancel"))
                            .disabled(self.busy)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("prompt-ok")
                            .primary()
                            .label(t!("common.ok"))
                            .loading(self.busy)
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
