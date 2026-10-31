//! Danger-command rules: list, enable/disable, add (regex validated by the backend) and delete.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::*;
use ops_core::settings::DangerRule;
use ops_i18n::t;

use crate::widgets::form::field;
use crate::widgets::toast;

pub struct DangerSection {
    rules: Vec<DangerRule>,
    loaded: bool,
}

impl DangerSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            rules: Vec::new(),
            loaded: false,
        };
        this.reload(window, cx);
        this
    }

    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let rules =
                ops_core::rt::run(
                    async move { ops_core::settings::list_danger_rules(&ctx.db).await },
                )
                .await
                .and_then(|r| r)
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.rules = rules;
                this.loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn rules(&self) -> &[DangerRule] {
        &self.rules
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn toggle(
        &mut self,
        id: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // optimistic, like the switch in the Tauri table
        if let Some(r) = self.rules.iter_mut().find(|r| r.id == id) {
            r.enabled = enabled;
        }
        cx.notify();
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = ops_core::rt::run(async move {
                ops_core::settings::toggle_danger_rule(&ctx.db, id, enabled).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = r {
                    toast::error(window, cx, t!("common.operationFailed", error = e));
                }
                this.reload(window, cx);
            });
        })
        .detach();
    }

    pub fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = ops_core::rt::run(async move {
                ops_core::settings::delete_danger_rule(&ctx.db, id).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = r {
                    toast::error(window, cx, t!("common.deleteFailed", error = e));
                }
                this.reload(window, cx);
            });
        })
        .detach();
    }

    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (weak, id) = (weak.clone(), id.clone());
            alert
                .confirm()
                .title(t!("common.delete"))
                .description(t!("common.confirmDelete"))
                .show_cancel(true)
                .ok_text(t!("common.delete"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.delete(id.clone(), window, cx));
                    true
                })
        });
    }

    pub fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<RuleDialog> {
        let section = cx.entity().downgrade();
        let view = cx.new(|cx| RuleDialog::new(section, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| {
            d.title(t!("settings.addDangerRule"))
                .w(px(520.))
                .child(shown.clone())
        });
        // like Radix, the first field has the focus when the dialog opens
        let first = view.read(cx).name.clone();
        first.update(cx, |s, cx| s.focus(window, cx));
        view
    }
}

impl Render for DangerSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let head = |label: String| {
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui.text_secondary)
                .child(label)
        };
        let weak = cx.entity().downgrade();
        v_flex()
            .id("settings-danger")
            .test_support()
            .gap_3()
            .child(
                h_flex().justify_end().child(
                    Button::new("danger-add")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Plus))
                        .label(t!("settings.addRule"))
                        .on_click(
                            cx.listener(|this, _, window, cx| drop(this.open_add(window, cx))),
                        ),
                ),
            )
            .child(
                v_flex()
                    .child(
                        h_flex()
                            .py(px(8.))
                            .border_b_1()
                            .border_color(ui.border)
                            .child(head(t!("common.name")).w(px(180.)).px(px(8.)))
                            .child(head(t!("settings.regex")).flex_1().px(px(8.)))
                            .child(head(t!("common.source")).w(px(80.)).px(px(8.)))
                            .child(head(t!("common.enabled")).w(px(80.)).px(px(8.)))
                            .child(head(t!("common.action")).w(px(80.)).px(px(8.))),
                    )
                    .children(self.rules.iter().enumerate().map(|(ix, rule)| {
                        let (toggle_id, delete_id) = (rule.id.clone(), rule.id.clone());
                        let (w1, w2) = (weak.clone(), weak.clone());
                        h_flex()
                            .id(("danger-row", ix))
                            .test_support()
                            .py(px(8.))
                            .items_center()
                            .border_b_1()
                            .border_color(ui.border.opacity(0.6))
                            .child(
                                div()
                                    .w(px(180.))
                                    .px(px(8.))
                                    .truncate()
                                    .text_size(px(13.))
                                    .child(rule.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(8.))
                                    .truncate()
                                    .font_family("Menlo")
                                    .text_size(px(12.))
                                    .child(rule.pattern.clone()),
                            )
                            .child(
                                h_flex()
                                    .w(px(80.))
                                    .px(px(8.))
                                    .child(
                                        div()
                                            .px(px(7.))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(if rule.is_builtin {
                                                ui.border_strong
                                            } else {
                                                theme.primary.opacity(0.5)
                                            })
                                            .bg(if rule.is_builtin {
                                                ui.text.opacity(0.06)
                                            } else {
                                                theme.primary.opacity(0.1)
                                            })
                                            .text_size(px(11.))
                                            .text_color(if rule.is_builtin {
                                                ui.text_secondary
                                            } else {
                                                theme.primary
                                            })
                                            .child(if rule.is_builtin {
                                                t!("common.builtin")
                                            } else {
                                                t!("common.custom")
                                            }),
                                    )
                                    .flex_none(),
                            )
                            .child(
                                div().w(px(80.)).px(px(8.)).child(
                                    Switch::new(("danger-toggle", ix))
                                        .checked(rule.enabled)
                                        .on_click(move |on, window, cx| {
                                            let (id, on) = (toggle_id.clone(), *on);
                                            let _ = w1.update(cx, |this, cx| {
                                                this.toggle(id, on, window, cx)
                                            });
                                        }),
                                ),
                            )
                            .child(div().w(px(80.)).px(px(8.)).child(if rule.is_builtin {
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child("-")
                                    .into_any_element()
                            } else {
                                Button::new(("danger-delete", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Trash))
                                    .on_click(move |_, window, cx| {
                                        let id = delete_id.clone();
                                        let _ = w2.update(cx, |this, cx| {
                                            this.confirm_delete(id, window, cx)
                                        });
                                    })
                                    .into_any_element()
                            }))
                    })),
            )
    }
}

/// "Add danger rule" dialog.
pub struct RuleDialog {
    section: WeakEntity<DangerSection>,
    pub name: Entity<InputState>,
    pub pattern: Entity<InputState>,
    errors: Vec<&'static str>,
    backend_error: Option<String>,
    saving: bool,
}

impl RuleDialog {
    fn new(
        section: WeakEntity<DangerSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            section,
            name: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.ruleNamePlaceholder"))
            }),
            pattern: cx
                .new(|cx| InputState::new(window, cx).placeholder(t!("settings.regexPlaceholder"))),
            errors: Vec::new(),
            backend_error: None,
            saving: false,
        }
    }

    pub fn errors(&self) -> &[&'static str] {
        &self.errors
    }

    pub fn backend_error(&self) -> Option<&str> {
        self.backend_error.as_deref()
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        let pattern = self.pattern.read(cx).value().trim().to_string();
        self.errors.clear();
        self.backend_error = None;
        if name.is_empty() {
            self.errors.push("name");
        }
        if pattern.is_empty() {
            self.errors.push("pattern");
        }
        if !self.errors.is_empty() {
            cx.notify();
            return;
        }
        self.saving = true;
        cx.notify();
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = ops_core::rt::run(async move {
                ops_core::settings::add_danger_rule(&ctx.db, name, pattern).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(_) => {
                        toast::success(window, cx, t!("settings.ruleAdded"));
                        window.close_dialog(cx);
                        let _ = this.section.update(cx, |s, cx| s.reload(window, cx));
                    }
                    Err(e) => this.backend_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for RuleDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let required = |key: &'static str, errors: &[&'static str]| {
            errors
                .contains(&key)
                .then(|| SharedString::from(t!("common.required")))
        };
        v_flex()
            .id("danger-rule-dialog")
            .test_support()
            .gap_3()
            .pt(px(16.))
            .child(field(
                t!("settings.ruleName"),
                Input::new(&self.name),
                required("name", &self.errors),
                cx,
            ))
            .child(field(
                t!("settings.regex"),
                Input::new(&self.pattern),
                required("pattern", &self.errors)
                    .or_else(|| self.backend_error.clone().map(SharedString::from)),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("danger-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("danger-ok")
                            .primary()
                            .label(t!("common.ok"))
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
