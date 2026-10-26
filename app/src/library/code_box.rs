//! A code editor whose grammar can change: `EditorState` fixes the language at construction, so switching rebuilds
//! the state with the same text.

use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::*;
use crate::scale::px;

pub fn editor_language(language: &str) -> &'static str {
    match language {
        "python" => "python",
        "powershell" => "powershell",
        _ => "bash",
    }
}

pub struct CodeBox {
    state: Entity<EditorState>,
    language: String,
    read_only: bool,
    height: Pixels,
    sub: Option<Subscription>,
}

impl CodeBox {
    pub fn new(text: &str, language: &str, read_only: bool, height: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { state: Self::make(text, language, read_only, window, cx), language: language.to_string(), read_only, height, sub: None };
        this.watch(cx);
        this
    }

    fn make(text: &str, language: &str, read_only: bool, window: &mut Window, cx: &mut Context<Self>) -> Entity<EditorState> {
        let lang = editor_language(language);
        let state = cx.new(|cx| EditorState::new(window, cx).language(lang));
        let text = text.to_string();
        state.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            if read_only {
                s.set_disabled(true, cx);
            }
        });
        state
    }

    fn watch(&mut self, cx: &mut Context<Self>) {
        self.sub = Some(cx.subscribe(&self.state, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        }));
    }

    pub fn text(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }

    pub fn state(&self) -> &Entity<EditorState> {
        &self.state
    }

    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.state.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    pub fn set_language(&mut self, language: &str, window: &mut Window, cx: &mut Context<Self>) {
        if editor_language(language) == editor_language(&self.language) {
            self.language = language.to_string();
            return;
        }
        let text = self.text(cx);
        self.language = language.to_string();
        self.state = Self::make(&text, language, self.read_only, window, cx);
        self.watch(cx);
        cx.notify();
    }
}

impl Render for CodeBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::ActiveTheme as _;
        div().h(self.height).w_full().border_1().border_color(cx.theme().border).rounded(px(8.)).overflow_hidden().child(Editor::new(&self.state).size_full())
    }
}
