//! "Edit SSH authentication" dialog shown from a failed connection (port of the auth retry Modal in TerminalPage).

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, WindowExt as _};
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_ssh::hosts::Host;

use super::workspace::{AuthRetryValues, TerminalWorkspace};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::field;
use crate::widgets::toast;

pub struct AuthRetryDialog {
    workspace: Entity<TerminalWorkspace>,
    tab_key: String,
    key_auth: bool,
    pub auth: Entity<ChoiceSelect>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub private_key: Entity<TextareaState>,
    username_missing: bool,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl AuthRetryDialog {
    /// Look up the stored host first (it may have been deleted meanwhile), then show the dialog.
    pub fn open(workspace: Entity<TerminalWorkspace>, tab_key: String, window: &mut Window, cx: &mut App) {
        let Some(host_id) = workspace.read(cx).tab(&tab_key).and_then(|t| t.host_id.clone()) else { return };
        let task = workspace.update(cx, |w, cx| w.find_host(host_id, cx));
        window
            .spawn(cx, async move |cx| {
                let result = task.await;
                let _ = cx.update(|window, cx| match result {
                    Ok(Some(host)) if host.os == "linux" => {
                        Self::open_for_host(workspace, tab_key, host, window, cx);
                    }
                    Ok(Some(_)) => toast::error(window, cx, t!("terminal.authRetryUnsupportedHost")),
                    Ok(None) => toast::error(window, cx, t!("terminal.authRetryHostMissing")),
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                });
            })
            .detach();
    }

    pub fn open_for_host(workspace: Entity<TerminalWorkspace>, tab_key: String, host: Host, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(workspace, tab_key, host, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |dialog, _, _| dialog.title(t!("terminal.authRetryTitle")).w(px(460.)).child(shown.clone()));
        view
    }

    fn new(workspace: Entity<TerminalWorkspace>, tab_key: String, host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key_auth = host.auth_type == "key";
        let auth = cx.new(|cx| {
            new_select(
                vec![Choice::new("password", t!("assets.passwordAuth")), Choice::new("key", t!("assets.keyAuth"))],
                if key_auth { "key" } else { "password" },
                window,
                cx,
            )
        });
        let username_value = host.username.clone();
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("root").default_value(username_value));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder(t!("assets.passwordKeepPlaceholder")).masked(true));
        let private_key = cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("assets.pastePrivateKey")).auto_grow(4, 8));
        let subs = vec![cx.subscribe_in(&auth, window, |this, _, ev, _, cx| {
            if let Some(v) = confirmed_value(ev) {
                this.key_auth = v == "key";
                cx.notify();
            }
        })];
        Self { workspace, tab_key, key_auth, auth, username, password, private_key, username_missing: false, saving: false, _subs: subs }
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let username = self.username.read(cx).value().to_string();
        self.username_missing = username.trim().is_empty();
        if self.username_missing {
            cx.notify();
            return;
        }
        let values = AuthRetryValues {
            key_auth: self.key_auth,
            username,
            password: self.password.read(cx).value().to_string(),
            private_key: self.private_key.read(cx).value().to_string(),
        };
        self.saving = true;
        cx.notify();
        let key = self.tab_key.clone();
        let task = self.workspace.update(cx, |w, cx| w.save_auth_and_retry(&key, values, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        toast::success(window, cx, t!("terminal.authRetrySaved"));
                        window.close_dialog(cx);
                    }
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AuthRetryDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let secret = if self.key_auth {
            field(t!("assets.sshPrivateKey"), Textarea::new(&self.private_key), None, cx)
                .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("terminal.authRetrySecretExtra")))
        } else {
            field(t!("assets.password"), Input::new(&self.password).mask_toggle(), None, cx)
                .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("terminal.authRetrySecretExtra")))
        };
        v_flex()
            .id("auth-retry-dialog")
            .test_support()
            .gap_3()
            .child(div().text_size(px(13.)).text_color(theme.muted_foreground).child(t!("terminal.authRetryDescription")))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(field(t!("assets.auth"), Select::new(&self.auth), None, cx).w(px(170.)))
                    .child(
                        field(
                            t!("assets.username"),
                            Input::new(&self.username),
                            self.username_missing.then(|| t!("common.required").into()),
                            cx,
                        )
                        .flex_1(),
                    ),
            )
            .child(secret)
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("auth-cancel").label(t!("common.cancel")).disabled(self.saving).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(
                        Button::new("auth-save")
                            .primary()
                            .label(t!("terminal.saveAuthAndReconnect"))
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
