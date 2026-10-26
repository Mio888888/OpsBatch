//! ⌘I floating command generator (port of `AiInlinePanel.tsx`): describe what you want, get a shell command,
//! and put it on the terminal prompt without running it.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::client::{ai_chat_stream, ChatMessage, StreamChatRequest};
use ops_i18n::t;

use super::panel::AiTarget;
use super::prompt;
use super::store::{ai_store, AiStoreEvent};

#[derive(Debug, Clone)]
pub enum InlineEvent {
    Close,
    /// Put this command on the terminal prompt.
    Inject(String),
}

pub struct AiInlinePanel {
    target: AiTarget,
    input: Entity<InputState>,
    response: String,
    streaming: bool,
    request_id: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<InlineEvent> for AiInlinePanel {}

impl AiInlinePanel {
    pub fn new(target: AiTarget, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ai_store(cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("ai.inlinePlaceholder")));
        let subs = vec![
            cx.subscribe(&store, |this, _, ev: &AiStoreEvent, cx| {
                let AiStoreEvent::Chunk(chunk) = ev;
                if chunk.client_request_id.is_none() || chunk.client_request_id != this.request_id {
                    return;
                }
                if chunk.done {
                    this.streaming = false;
                    this.request_id = None;
                } else {
                    this.response.push_str(&chunk.delta);
                }
                cx.notify();
            }),
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = ev {
                    this.send(window, cx);
                }
            }),
            cx.observe(&input, |_, _, cx| cx.notify()),
        ];
        input.update(cx, |i, cx| i.focus(window, cx));
        Self { target, input, response: String::new(), streaming: false, request_id: None, _subs: subs }
    }

    pub fn set_target(&mut self, target: AiTarget) {
        self.target = target;
    }

    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }
    pub fn response(&self) -> &str {
        &self.response
    }
    pub fn streaming(&self) -> bool {
        self.streaming
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt_text = self.input.read(cx).value().trim().to_string();
        if prompt_text.is_empty() || self.streaming {
            return;
        }
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
        self.response.clear();
        self.streaming = true;
        let request_id = uuid::Uuid::new_v4().to_string();
        self.request_id = Some(request_id.clone());

        let buffer = self.target.pane.upgrade().map(|p| p.read(cx).buffer_text()).unwrap_or_default();
        let request = StreamChatRequest {
            messages: vec![
                ChatMessage { role: "system".into(), content: prompt::inline_context(&buffer), images: None },
                ChatMessage { role: "user".into(), content: prompt_text, images: None },
            ],
            model: None,
            temperature: None,
            max_tokens: None,
            conversation_id: None,
            scope: Some(self.target.scope.scope.clone()),
            scope_id: Some(self.target.scope.scope_id.clone()),
            client_request_id: Some(request_id.clone()),
        };
        let ctx = crate::services::ctx(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { ai_chat_stream(&ctx.core, &ctx.db, request).await }).await.and_then(|r| r);
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    if this.request_id.as_deref() == Some(request_id.as_str()) {
                        this.response = t!("ai.inlineFailed");
                        this.streaming = false;
                        this.request_id = None;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub fn inject(&mut self, cx: &mut Context<Self>) {
        let command = self.response.trim().to_string();
        if command.is_empty() || self.streaming {
            return;
        }
        cx.emit(InlineEvent::Inject(command));
        cx.emit(InlineEvent::Close);
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.response.clone()));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(InlineEvent::Close);
    }
}

impl Render for AiInlinePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let has_input = !self.input.read(cx).value().trim().is_empty();
        v_flex()
            .id("ai-inline")
            .test_support()
            .w(px(420.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_lg()
            .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                this.close(cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    this.close(cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(t!("ai.inlineTitle")))
                    .child(Button::new("ai-inline-close").ghost().xsmall().icon(Icon::new(IconName::X)).on_click(cx.listener(|this, _, _, cx| this.close(cx)))),
            )
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.input).small().disabled(self.streaming)))
                            .child(
                                Button::new("ai-inline-send")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(IconName::Send))
                                    .disabled(self.streaming || !has_input)
                                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                            ),
                    )
                    .when(!self.response.is_empty(), |d| {
                        d.child(div().id("ai-inline-response").test_support().p_2().rounded(px(6.)).bg(theme.muted).font_family("Menlo").text_size(px(12.)).max_h(px(160.)).overflow_y_scroll().child(self.response.clone()))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Button::new("ai-inline-inject").primary().small().label(format!("↵ {}", t!("ai.inlineInject"))).disabled(self.streaming).on_click(cx.listener(|this, _, _, cx| this.inject(cx))))
                                    .child(Button::new("ai-inline-copy").small().label(t!("ai.inlineCopy")).on_click(cx.listener(|this, _, _, cx| this.copy(cx)))),
                            )
                    })
                    .when(self.streaming && self.response.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("ai.inlineGenerating")))),
            )
    }
}
