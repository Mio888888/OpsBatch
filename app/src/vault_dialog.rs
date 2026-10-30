//! "解锁本地加密": the startup dialog that opens the local vault (port of `KeychainNoticeModal.tsx`).
//!
//! It first tries the startup password stored in the system keychain. When there is none, or it no longer opens the
//! vault, it asks for the startup password (Enter submits). After the unlock plaintext secrets are migrated into the
//! vault. The dialog cannot be dismissed; the app is not usable without its credentials.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

/// Messages of the vault that mean "ask the user for the startup password" (`KeychainNoticeModal.tsx`).
pub const MISSING_KEY: &str = "本地加密密钥不存在";
pub const STALE_KEY: &str = "系统钥匙串中的本地加密密钥无法解开";

pub struct VaultDialog {
    input: Entity<InputState>,
    loading: bool,
    /// The system keychain could not unlock it: the password field is shown.
    pending_input: bool,
    error: String,
    _sub: Subscription,
}

impl VaultDialog {
    pub fn open(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| d.title(t!("keychainNotice.title")).w(px(520.)).close_button(false).overlay_closable(false).keyboard(false).child(shown.clone()));
        view
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("keychainNotice.inputPlaceholder")).masked(true));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
        });
        let mut this = Self { input, loading: false, pending_input: false, error: String::new(), _sub: sub };
        this.try_unlock(None, window, cx);
        this
    }

    pub fn password_input(&self) -> &Entity<InputState> {
        &self.input
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
    pub fn needs_password(&self) -> bool {
        self.pending_input
    }
    pub fn error(&self) -> &str {
        &self.error
    }

    /// The Unlock button / Enter: unlock with the typed startup password.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.input.read(cx).value().to_string();
        self.try_unlock(Some(key), window, cx);
    }

    fn try_unlock(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error.clear();
        cx.notify();
        let backend = crate::vault::backend(cx);
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move {
                tokio::task::spawn_blocking(move || {
                    backend.unlock(key)?;
                    backend.after_unlock(&ctx.db)
                })
                .await
                .map_err(|e| e.to_string())?
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(()) => {
                        window.close_dialog(cx);
                        crate::vault::dialog_closed(cx);
                        crate::vault::unlocked(cx);
                    }
                    Err(message) if message.contains(MISSING_KEY) || message.contains(STALE_KEY) => {
                        this.pending_input = true;
                        this.error = if message.contains(STALE_KEY) { t!("keychainNotice.staleMasterKey") } else { t!("keychainNotice.needMasterKey") };
                        this.input.update(cx, |s, cx| s.focus(window, cx));
                    }
                    Err(message) => this.error = message,
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for VaultDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let muted = |text: String| div().text_size(px(12.)).line_height(relative(1.5)).text_color(ui.text_muted).child(text);
        v_flex()
            .id("vault-dialog")
            .test_support()
            .gap(px(12.))
            .pt(px(8.))
            .child(div().text_size(px(13.)).line_height(relative(1.6)).text_color(ui.text_secondary).child(t!("keychainNotice.body")))
            .child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(ui.border)
                    .bg(ui.input_bg)
                    .child(Icon::new(IconName::ShieldCheck).size_4().text_color(theme.primary))
                    .child(div().text_size(px(13.)).text_color(ui.text).child(t!("keychainNotice.status"))),
            )
            .child(if self.pending_input {
                h_flex()
                    .id("vault-password-form")
                    .test_support()
                    .gap(px(8.))
                    .items_center()
                    .child(div().flex_1().child(Input::new(&self.input).mask_toggle()))
                    .child(Button::new("vault-unlock").primary().icon(Icon::new(IconName::KeyRound)).label(if self.loading { t!("keychainNotice.unlocking") } else { t!("keychainNotice.unlock") }).loading(self.loading).on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))))
                    .into_any_element()
            } else {
                v_flex().id("vault-hints").test_support().gap(px(6.)).child(muted(t!("keychainNotice.autoUnlock"))).child(muted(t!("keychainNotice.lockedHint"))).into_any_element()
            })
            .when(!self.error.is_empty(), |d| d.child(div().id("vault-error").test_support().text_size(px(12.)).line_height(relative(1.5)).text_color(theme.danger).child(self.error.clone())))
            .when(self.loading || !self.pending_input, |d| d.child(h_flex().gap(px(8.)).items_center().child(Spinner::new().small()).child(muted(t!("keychainNotice.waiting")))))
    }
}
