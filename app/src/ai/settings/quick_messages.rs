//! Quick messages section of the tool-access tab (`QuickMessagesSettings`): reusable prompts offered as `/slug`.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::prefs::quick::{self, QuickMessage, SlugIssue, MAX_ITEMS};
use ops_i18n::t;

use super::ui;
use crate::ai::prefs_store::PrefsStore;

pub struct QuickMessagesSection {
    store: Entity<PrefsStore>,
    /// Slugs of ready user skills; a quick message may not reuse them.
    reserved: Vec<String>,
    _subs: Vec<Subscription>,
}

impl QuickMessagesSection {
    pub fn new(store: Entity<PrefsStore>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            reserved: Vec::new(),
            _subs: subs,
        }
    }

    pub fn set_reserved(&mut self, slugs: Vec<String>, cx: &mut Context<Self>) {
        self.reserved = slugs;
        cx.notify();
    }

    pub fn messages(&self, cx: &App) -> Vec<QuickMessage> {
        self.store.read(cx).prefs().quick_messages.clone()
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.quick_messages.retain(|m| m.id != id))
        });
    }

    fn confirm_remove(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (weak, id) = (weak.clone(), id.clone());
            alert
                .confirm()
                .title(t!("ncai.quickMessages.confirmDelete", name = name.clone()))
                .show_cancel(true)
                .ok_text(t!("common.delete"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| this.remove(&id, cx));
                    true
                })
        });
    }

    pub fn open_edit(
        &mut self,
        message: Option<QuickMessage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<QuickMessageDialog> {
        let (store, reserved) = (self.store.clone(), self.reserved.clone());
        let editing = message.is_some();
        let view = cx.new(|cx| QuickMessageDialog::new(store, reserved, message, window, cx));
        let shown = view.clone();
        let title = if editing {
            t!("ncai.quickMessages.editTitle")
        } else {
            t!("ncai.quickMessages.createTitle")
        };
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(560.)).child(shown.clone())
        });
        view
    }
}

impl Render for QuickMessagesSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let messages = self.messages(cx);
        let weak = cx.entity().downgrade();
        let full = messages.len() >= MAX_ITEMS;
        let add = Button::new("ai-quick-add")
            .small()
            .icon(Icon::new(IconName::Plus))
            .label(t!("ncai.quickMessages.add"))
            .disabled(full)
            .on_click(cx.listener(|this, _, window, cx| drop(this.open_edit(None, window, cx))));
        let body: AnyElement = if messages.is_empty() {
            v_flex()
                .id("ai-quick-empty")
                .test_support()
                .items_center()
                .p(px(20.))
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(ui_colors.border)
                .child(ui::muted_text(t!("ncai.quickMessages.empty"), cx))
                .into_any_element()
        } else {
            ui::card(
                messages
                    .iter()
                    .enumerate()
                    .map(|(ix, m)| {
                        let (w1, w2) = (weak.clone(), weak.clone());
                        let (edit, name_for_delete, id_for_delete) =
                            (m.clone(), m.name.clone(), m.id.clone());
                        h_flex()
                            .id(("ai-quick-row", ix))
                            .test_support()
                            .gap(px(12.))
                            .items_start()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .truncate()
                                                    .child(m.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .font_family("Menlo")
                                                    .text_size(px(11.))
                                                    .text_color(ui_colors.primary)
                                                    .child(format!("/{}", m.slug)),
                                            ),
                                    )
                                    .when_some(m.description.clone(), |d, desc| {
                                        d.child(ui::muted_text(desc, cx))
                                    })
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(ui_colors.text_muted)
                                            .truncate()
                                            .child(m.content.replace('\n', " ")),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap(px(2.))
                                    .child(
                                        Button::new(("ai-quick-edit", ix))
                                            .ghost()
                                            .small()
                                            .icon(Icon::new(IconName::Pencil))
                                            .on_click(move |_, window, cx| {
                                                let m = edit.clone();
                                                let _ = w1.update(cx, |this, cx| {
                                                    drop(this.open_edit(Some(m), window, cx))
                                                });
                                            }),
                                    )
                                    .child(
                                        Button::new(("ai-quick-delete", ix))
                                            .ghost()
                                            .small()
                                            .icon(Icon::new(IconName::Trash))
                                            .on_click(move |_, window, cx| {
                                                let (id, name) = (
                                                    id_for_delete.clone(),
                                                    name_for_delete.clone(),
                                                );
                                                let _ = w2.update(cx, |this, cx| {
                                                    this.confirm_remove(id, name, window, cx)
                                                });
                                            }),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect(),
                true,
                cx,
            )
            .into_any_element()
        };
        ui::section(
            "ai-quick-messages",
            t!("ncai.quickMessages.title"),
            Some(add.into_any_element()),
            vec![
                ui::muted_text(t!("ncai.quickMessages.description"), cx).into_any_element(),
                body,
            ],
            cx,
        )
    }
}

/// Create / edit dialog.
pub struct QuickMessageDialog {
    store: Entity<PrefsStore>,
    reserved: Vec<String>,
    editing: Option<QuickMessage>,
    pub name: Entity<InputState>,
    pub slug: Entity<InputState>,
    pub description: Entity<InputState>,
    pub content: Entity<TextareaState>,
    /// The slug last derived from the name (an existing message keeps its slug: nothing is derived for it).
    generated_slug: String,
    errors: Vec<(&'static str, String)>,
    _subs: Vec<Subscription>,
}

impl QuickMessageDialog {
    fn new(
        store: Entity<PrefsStore>,
        reserved: Vec<String>,
        message: Option<QuickMessage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editing_existing = message.is_some();
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.quickMessages.name.placeholder"))
        });
        let slug = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.quickMessages.slug.placeholder"))
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ncai.quickMessages.descriptionField.placeholder"))
        });
        let content = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("ncai.quickMessages.content.placeholder"))
                .auto_grow(5, 12)
        });
        if let Some(m) = &message {
            name.update(cx, |s, cx| s.set_value(m.name.clone(), window, cx));
            slug.update(cx, |s, cx| s.set_value(m.slug.clone(), window, cx));
            description.update(cx, |s, cx| {
                s.set_value(m.description.clone().unwrap_or_default(), window, cx)
            });
            content.update(cx, |s, cx| s.set_value(m.content.clone(), window, cx));
        }
        let subs =
            vec![
                cx.subscribe_in(&name, window, |this, name, ev: &InputEvent, window, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    // the slug follows the name until the user has typed their own
                    let current = this.slug.read(cx).value().to_string();
                    if current.is_empty() || current == this.generated_slug {
                        let generated = quick::slug_from_name(name.read(cx).value().as_ref());
                        this.generated_slug = generated.clone();
                        this.slug
                            .update(cx, |s, cx| s.set_value(generated, window, cx));
                    }
                }),
            ];
        Self {
            store,
            reserved,
            editing: message,
            name,
            slug,
            description,
            content,
            generated_slug: if editing_existing {
                "\u{0}".to_string()
            } else {
                String::new()
            },
            errors: Vec::new(),
            _subs: subs,
        }
    }

    pub fn errors(&self) -> &[(&'static str, String)] {
        &self.errors
    }

    /// Validates and stores the message; closes the dialog on success.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        let slug = quick::normalize_slug(self.slug.read(cx).value().as_ref());
        let description = self.description.read(cx).value().trim().to_string();
        let content = self.content.read(cx).value().trim().to_string();
        self.errors.clear();
        if name.is_empty() {
            self.errors
                .push(("name", t!("ncai.quickMessages.error.nameRequired")));
        }
        let messages = self.store.read(cx).prefs().quick_messages.clone();
        let editing_id = self.editing.as_ref().map(|m| m.id.as_str());
        match quick::check_slug(&slug, editing_id, &messages, &self.reserved) {
            Some(SlugIssue::Empty | SlugIssue::Invalid) => self
                .errors
                .push(("slug", t!("ncai.quickMessages.error.invalidSlug"))),
            Some(SlugIssue::Duplicate) => self
                .errors
                .push(("slug", t!("ncai.quickMessages.error.slugTaken"))),
            Some(SlugIssue::Reserved) => self.errors.push((
                "slug",
                t!(
                    "ncai.quickMessages.error.slugConflictsWithSkill",
                    slug = slug.clone()
                ),
            )),
            None => {}
        }
        if content.is_empty() {
            self.errors
                .push(("content", t!("ncai.quickMessages.error.contentRequired")));
        }
        if self.editing.is_none() && messages.len() >= MAX_ITEMS {
            self.errors.push((
                "name",
                t!("ncai.quickMessages.error.maxItems", max = MAX_ITEMS),
            ));
        }
        if !self.errors.is_empty() {
            cx.notify();
            return false;
        }
        let entry = QuickMessage {
            id: self
                .editing
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_else(quick::create_id),
            name,
            slug,
            content,
            description: Some(description).filter(|d| !d.is_empty()),
        };
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                match p.quick_messages.iter_mut().find(|m| m.id == entry.id) {
                    Some(slot) => *slot = entry,
                    None => p.quick_messages.push(entry),
                }
            })
        });
        window.close_dialog(cx);
        true
    }
}

impl Render for QuickMessageDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let err = |key: &str, errors: &[(&'static str, String)]| {
            errors
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, e)| SharedString::from(e.clone()))
        };
        let field = |label: String, control: AnyElement, error: Option<SharedString>| {
            crate::widgets::form::field(label, control, error, cx)
        };
        v_flex()
            .id("ai-quick-dialog")
            .test_support()
            .gap_3()
            .pt(px(12.))
            .child(field(
                t!("ncai.quickMessages.name"),
                Input::new(&self.name).into_any_element(),
                err("name", &self.errors),
            ))
            .child(field(
                t!("ncai.quickMessages.slug"),
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(div().text_color(theme.muted_foreground).child("/"))
                    .child(div().flex_1().child(Input::new(&self.slug)))
                    .into_any_element(),
                err("slug", &self.errors),
            ))
            .child(field(
                t!("ncai.quickMessages.descriptionField"),
                Input::new(&self.description).into_any_element(),
                None,
            ))
            .child(field(
                t!("ncai.quickMessages.content"),
                ui::textarea(&self.content).into_any_element(),
                err("content", &self.errors),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("ai-quick-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("ai-quick-save")
                            .primary()
                            .label(t!("common.save"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit(window, cx);
                            })),
                    ),
            )
    }
}
