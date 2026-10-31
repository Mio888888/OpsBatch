//! Script library page. Port of `pages/ScriptLib/ScriptLibPage.tsx`.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_library::library::ScriptVersion;

use super::code_box::CodeBox;
use super::model::{
    category_counts, category_options, filter_scripts, Platform, QuickActionEntry, Risk,
    ScriptEntry,
};
use super::store::{library_store, LibraryStore};
use super::ui::*;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::field;
use crate::widgets::toast;

const LANGUAGES: [(&str, &str); 3] = [
    ("shell", "Shell"),
    ("python", "Python"),
    ("powershell", "PowerShell"),
];

fn language_choices() -> Vec<Choice> {
    LANGUAGES.iter().map(|(v, l)| Choice::new(*v, *l)).collect()
}

fn risk_choices() -> Vec<Choice> {
    Risk::ALL
        .iter()
        .map(|r| Choice::new(r.as_str(), t!(r.option_key())))
        .collect()
}

pub struct ScriptLibPage {
    store: Entity<LibraryStore>,
    search: Entity<InputState>,
    category: Option<String>,
    _subs: Vec<Subscription>,
}

impl ScriptLibPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = library_store(cx);
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("scriptLib.searchPlaceholder")));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        store.update(cx, |s, cx| s.load_scripts(cx));
        Self {
            store,
            search,
            category: None,
            _subs: subs,
        }
    }

    fn toast_result<T: 'static>(
        task: Task<Result<T, String>>,
        ok: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        window
            .spawn(cx, async move |cx| {
                let r = task.await;
                let _ = cx.update(|window, cx| match r {
                    Ok(_) => toast::success(window, cx, ok),
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                });
            })
            .detach();
    }

    fn add_to_quick_actions(
        &mut self,
        s: &ScriptEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry = QuickActionEntry {
            id: String::new(),
            name: s.name.clone(),
            command: s
                .content
                .lines()
                .next()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| s.content.clone()),
            category: s.category.clone(),
            parameters_json: "[]".into(),
            order: 0,
            starred: false,
            description: s.description.clone(),
            tags: vec![],
            language: s.language.clone(),
            last_run_at: String::new(),
            last_status: String::new(),
        };
        let task = self.store.update(cx, |st, cx| st.add_action(entry, cx));
        Self::toast_result(task, t!("library.addedToQuickActions"), window, cx);
    }

    pub fn open_view(
        &mut self,
        script: ScriptEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ScriptViewDialog> {
        let view = cx.new(|cx| ScriptViewDialog::new(script.clone(), window, cx));
        let shown = view.clone();
        let title = script.name.clone();
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(750.)).child(shown.clone())
        });
        view
    }

    /// Saves the current content as a version (like the original), then opens the editor.
    pub fn open_edit(
        &mut self,
        script: ScriptEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ScriptEditDialog> {
        self.store
            .update(cx, |s, cx| {
                s.save_script_version(
                    script.id.clone(),
                    script.content.clone(),
                    t!("scriptLib.autoSaveBeforeEdit"),
                    cx,
                )
            })
            .detach();
        let store = self.store.clone();
        let view = cx.new(|cx| ScriptEditDialog::new(store, script.clone(), window, cx));
        let shown = view.clone();
        let title = t!("scriptLib.editTitle", name = script.name.clone());
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(800.)).child(shown.clone())
        });
        view
    }

    pub fn open_add(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ScriptAddDialog> {
        let existing: Vec<String> = self
            .store
            .read(cx)
            .scripts
            .iter()
            .map(|c| c.category.clone())
            .filter(|c| !c.trim().is_empty())
            .collect();
        let store = self.store.clone();
        let view = cx.new(|cx| ScriptAddDialog::new(store, existing, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| {
            d.title(t!("scriptLib.addCustomScript"))
                .w(px(720.))
                .child(shown.clone())
        });
        view
    }
}

impl Render for ScriptLibPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let query = self.search.read(cx).value().to_string();
        let (rows, total, counts): (Vec<ScriptEntry>, usize, _) = {
            let all = &self.store.read(cx).scripts;
            (
                filter_scripts(all, self.category.as_deref(), &query)
                    .into_iter()
                    .cloned()
                    .collect(),
                all.len(),
                category_counts(all.iter().map(|s| s.category.as_str())),
            )
        };
        let weak = cx.entity().downgrade();
        let rail_weak = weak.clone();
        let filtered = rows.len();

        let list = if rows.is_empty() {
            empty_state(t!("scriptLib.noScripts"), cx).into_any_element()
        } else {
            v_flex()
                .id("sl-list")
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(4.))
                .gap(px(2.))
                .children(rows.into_iter().enumerate().map(|(ix, s)| {
                    let text = s.display_text();
                    let preview: String = if text.chars().count() > 120 {
                        text.chars().take(120).collect::<String>() + "…"
                    } else {
                        text
                    };
                    let (w_view, w_quick, w_edit, w_del) =
                        (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                    let (s_view, s_quick, s_edit, id) =
                        (s.clone(), s.clone(), s.clone(), s.id.clone());
                    h_flex()
                        .id(("sl-row", ix))
                        .test_support()
                        .group("slib-row")
                        .h(px(53.))
                        .gap(px(12.))
                        .px(px(10.))
                        .py(px(7.))
                        .items_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(gpui_kit::transparent_black())
                        .hover(move |d| {
                            d.bg(ui.text_muted.opacity(0.04))
                                .border_color(ui.border.opacity(0.7))
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .px_1p5()
                                                .rounded(px(3.))
                                                .bg(Hsla {
                                                    a: 0.15,
                                                    ..theme.primary
                                                })
                                                .text_color(theme.primary)
                                                .text_size(px(10.))
                                                .child(lang_label(&s.language)),
                                        )
                                        .child(
                                            div()
                                                .max_w(px(240.))
                                                .truncate()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(13.))
                                                .text_color(ui.text)
                                                .child(s.name.clone()),
                                        )
                                        .when(!s.url.is_empty(), |d| {
                                            d.child(tag_chip(t!("library.remote"), cx))
                                        })
                                        .when(!s.parameters.is_empty(), |d| {
                                            d.child(tag_chip(t!("library.parameters"), cx))
                                        }),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family("Menlo")
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .child(preview),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(risk_chip(s.risk, cx))
                                .child(
                                    div()
                                        .whitespace_nowrap()
                                        .text_right()
                                        .text_size(px(11.))
                                        .text_color(ui.text_muted)
                                        .child(s.category.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(px(2.))
                                .text_color(ui.text_muted)
                                .opacity(0.4)
                                .group_hover("slib-row", |d| d.opacity(1.))
                                .child(act_button(
                                    ("sl-view", ix),
                                    IconName::Eye,
                                    t!("library.view"),
                                    move |_, window, cx| {
                                        let s = s_view.clone();
                                        let _ = w_view.update(cx, |this, cx| {
                                            drop(this.open_view(s, window, cx))
                                        });
                                    },
                                ))
                                .child(act_button(
                                    ("sl-quick", ix),
                                    IconName::Zap,
                                    t!("library.addToQuickActions"),
                                    move |_, window, cx| {
                                        let s = s_quick.clone();
                                        let _ = w_quick.update(cx, |this, cx| {
                                            this.add_to_quick_actions(&s, window, cx)
                                        });
                                    },
                                ))
                                .when(!s.is_builtin, |d| {
                                    d.child(act_button(
                                        ("sl-edit", ix),
                                        IconName::SquarePen,
                                        t!("common.edit"),
                                        move |_, window, cx| {
                                            let s = s_edit.clone();
                                            let _ = w_edit.update(cx, |this, cx| {
                                                drop(this.open_edit(s, window, cx))
                                            });
                                        },
                                    ))
                                    .child(act_button(
                                        ("sl-delete", ix),
                                        IconName::Trash,
                                        t!("common.delete"),
                                        move |_, window, cx| {
                                            let id = id.clone();
                                            let _ = w_del.update(cx, |this, cx| {
                                                let task = this
                                                    .store
                                                    .update(cx, |st, cx| st.delete_script(id, cx));
                                                Self::toast_result(
                                                    task,
                                                    t!("library.deleted"),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    ))
                                }),
                        )
                }))
                .into_any_element()
        };

        let count = t!("scriptLib.count", filtered = filtered, total = total);
        let add = Button::new("sl-add")
            .primary()
            .icon(Icon::new(IconName::Plus))
            .label(t!("scriptLib.addScript"))
            .on_click(cx.listener(|this, _, window, cx| drop(this.open_add(window, cx))));
        let rail = category_rail(
            "sl-cat",
            total,
            &counts,
            self.category.as_deref(),
            cx,
            move |value, _, cx| {
                let _ = rail_weak.update(cx, |this, cx| {
                    this.category = value;
                    cx.notify();
                });
            },
        );
        page_frame(
            "script-lib",
            search_box(&self.search),
            count,
            add,
            rail,
            list,
            cx,
        )
    }
}

// ---- view ----

pub struct ScriptViewDialog {
    script: ScriptEntry,
    code: Entity<CodeBox>,
}

impl ScriptViewDialog {
    fn new(script: ScriptEntry, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let code = cx.new(|cx| {
            CodeBox::new(
                &script.content,
                &script.language,
                true,
                px(350.),
                window,
                cx,
            )
        });
        Self { script, code }
    }
}

impl Render for ScriptViewDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let s = &self.script;
        v_flex()
            .id("script-view")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(tag_chip(s.language.clone(), cx))
                    .child(tag_chip(
                        t!("library.riskPrefix", risk = t!(s.risk.label_key())),
                        cx,
                    ))
                    .child(tag_chip(s.category.clone(), cx))
                    .children(s.tags.iter().map(|tg| tag_chip(tg.clone(), cx))),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(s.description.clone()),
            )
            .when(!s.parameters.is_empty(), |d| {
                d.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("scriptLib.params")),
                        )
                        .children(s.parameters.iter().map(|p| {
                            tag_chip(
                                format!("{}={} ({})", p.name, p.default_value, p.description),
                                cx,
                            )
                        })),
                )
            })
            .child(self.code.clone())
            .child(
                h_flex().justify_end().child(
                    Button::new("script-view-close")
                        .label(t!("common.close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    }
}

// ---- edit ----

pub struct ScriptEditDialog {
    store: Entity<LibraryStore>,
    script: ScriptEntry,
    original_content: String,
    pub name: Entity<InputState>,
    pub description: Entity<InputState>,
    language: Entity<ChoiceSelect>,
    risk: Entity<ChoiceSelect>,
    pub code: Entity<CodeBox>,
    versions: Vec<ScriptVersion>,
    show_versions: bool,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl ScriptEditDialog {
    fn new(
        store: Entity<LibraryStore>,
        script: ScriptEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("scriptLib.scriptName"))
                .default_value(script.name.clone())
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("scriptLib.scriptDescription"))
                .default_value(script.description.clone())
        });
        let language = cx.new(|cx| new_select(language_choices(), &script.language, window, cx));
        let risk = cx.new(|cx| new_select(risk_choices(), script.risk.as_str(), window, cx));
        let code = cx.new(|cx| {
            CodeBox::new(
                &script.content,
                &script.language,
                false,
                px(400.),
                window,
                cx,
            )
        });
        let subs = vec![
            cx.subscribe_in(&language, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.script.language = v.clone();
                    this.code.update(cx, |c, cx| c.set_language(&v, window, cx));
                    cx.notify();
                }
            }),
            cx.subscribe_in(&risk, window, |this, _, ev, _, cx| {
                this.script.risk = Risk::parse(&confirmed_value(ev).unwrap_or_default());
                cx.notify();
            }),
        ];
        let original_content = script.content.clone();
        Self {
            store,
            script,
            original_content,
            name,
            description,
            language,
            risk,
            code,
            versions: Vec::new(),
            show_versions: false,
            saving: false,
            _subs: subs,
        }
    }

    pub fn versions(&self) -> &[ScriptVersion] {
        &self.versions
    }

    pub fn toggle_versions(&mut self, cx: &mut Context<Self>) {
        let task = self.store.update(cx, |s, cx| {
            s.list_script_versions(self.script.id.clone(), cx)
        });
        cx.spawn(async move |this, cx| {
            let v = task.await;
            let _ = this.update(cx, |this, cx| {
                this.versions = v;
                this.show_versions = !this.show_versions;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn restore(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = self.versions.get(ix).cloned() else {
            return;
        };
        self.code
            .update(cx, |c, cx| c.set_text(&v.content, window, cx));
        toast::success(window, cx, t!("library.restoredVersion", label = v.label));
        self.show_versions = false;
        cx.notify();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let mut script = self.script.clone();
        script.name = self.name.read(cx).value().to_string();
        script.description = self.description.read(cx).value().to_string();
        script.content = self.code.read(cx).text(cx);
        self.saving = true;
        cx.notify();
        // The pre-edit content is recorded as a version first, then the new content is saved.
        self.store
            .update(cx, |s, cx| {
                s.save_script_version(
                    script.id.clone(),
                    self.original_content.clone(),
                    t!("scriptLib.autoSaveBeforeEdit"),
                    cx,
                )
            })
            .detach();
        let task = self.store.update(cx, |s, cx| s.update_script(script, cx));
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(()) => {
                        toast::success(window, cx, t!("library.savedWithVersion"));
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

impl Render for ScriptEditDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let n = self.versions.len();
        v_flex()
            .id("script-edit")
            .test_support()
            .gap_2()
            .when(self.show_versions && n > 0, |d| {
                d.child(
                    v_flex()
                        .id("sl-versions")
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .gap_1()
                        .children(self.versions.iter().enumerate().map(|(i, v)| {
                            h_flex()
                                .justify_between()
                                .items_center()
                                .text_size(px(12.))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(tag_chip(
                                            if i == 0 {
                                                t!("common.latest")
                                            } else {
                                                format!("v{}", n - i)
                                            },
                                            cx,
                                        ))
                                        .child(
                                            div()
                                                .text_color(theme.muted_foreground)
                                                .child(format!("{} - {}", v.label, v.created_at)),
                                        ),
                                )
                                .child(
                                    Button::new(("sl-restore", i))
                                        .xsmall()
                                        .label(t!("common.restoreVersion"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.restore(i, window, cx)
                                        })),
                                )
                        })),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(200.)).child(Input::new(&self.name).small()))
                    .child(div().w(px(130.)).child(Select::new(&self.language).small()))
                    .child(div().w(px(110.)).child(Select::new(&self.risk).small())),
            )
            .child(self.code.clone())
            .child(Input::new(&self.description).small())
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sl-history")
                            .icon(Icon::new(IconName::Undo2))
                            .label(format!(
                                "{} ({})",
                                if self.show_versions {
                                    t!("common.hideVersions")
                                } else {
                                    t!("common.versionHistory")
                                },
                                n
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_versions(cx))),
                    )
                    .child(
                        Button::new("sl-edit-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("sl-edit-save")
                            .primary()
                            .label(t!("common.save"))
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

// ---- add ----

pub struct ScriptAddDialog {
    store: Entity<LibraryStore>,
    pub name: Entity<InputState>,
    pub description: Entity<InputState>,
    pub category: Entity<ChoiceSelect>,
    language: Entity<ChoiceSelect>,
    risk: Entity<ChoiceSelect>,
    pub code: Entity<CodeBox>,
    language_value: String,
    category_value: Option<String>,
    risk_value: Risk,
    errors: Vec<&'static str>,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl ScriptAddDialog {
    fn new(
        store: Entity<LibraryStore>,
        existing: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let presets = [
            t!("scriptLib.categories.deployment"),
            t!("scriptLib.categories.inspection"),
            t!("scriptLib.categories.maintenance"),
            t!("scriptLib.categories.monitoring"),
            t!("scriptLib.categories.security"),
        ];
        let refs: Vec<&str> = presets.iter().map(String::as_str).collect();
        let categories = category_options(&refs, existing);
        let category = cx.new(|cx| {
            new_select(
                categories
                    .iter()
                    .map(|c| Choice::new(c.clone(), c.clone()))
                    .collect(),
                "",
                window,
                cx,
            )
        });
        let language = cx.new(|cx| new_select(language_choices(), "shell", window, cx));
        let risk = cx.new(|cx| new_select(risk_choices(), "low", window, cx));
        let code = cx.new(|cx| CodeBox::new("", "shell", false, px(280.), window, cx));
        let subs = vec![
            cx.subscribe_in(&category, window, |this, _, ev, _, cx| {
                this.category_value = confirmed_value(ev);
                cx.notify();
            }),
            cx.subscribe_in(&language, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.language_value = v.clone();
                    this.code.update(cx, |c, cx| c.set_language(&v, window, cx));
                }
            }),
            cx.subscribe_in(&risk, window, |this, _, ev, _, cx| {
                this.risk_value = Risk::parse(&confirmed_value(ev).unwrap_or_default());
                cx.notify();
            }),
        ];
        Self {
            store,
            name: cx
                .new(|cx| InputState::new(window, cx).placeholder(t!("scriptLib.namePlaceholder"))),
            description: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("scriptLib.descriptionPlaceholder"))
            }),
            category,
            language,
            risk,
            code,
            language_value: "shell".into(),
            category_value: None,
            risk_value: Risk::Low,
            errors: Vec::new(),
            saving: false,
            _subs: subs,
        }
    }

    pub fn errors(&self) -> &[&'static str] {
        &self.errors
    }

    pub fn set_category(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.category_value = Some(value.to_string());
        let v = value.to_string();
        self.category
            .update(cx, |s, cx| s.set_selected_value(&v, window, cx));
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        let content = self.code.read(cx).text(cx);
        self.errors.clear();
        if name.is_empty() {
            self.errors.push("name");
        }
        if self.category_value.is_none() {
            self.errors.push("category");
        }
        if content.trim().is_empty() {
            self.errors.push("content");
        }
        if !self.errors.is_empty() {
            cx.notify();
            return;
        }
        let entry = ScriptEntry {
            id: String::new(),
            name,
            language: self.language_value.clone(),
            category: self.category_value.clone().unwrap_or_default(),
            tags: vec![],
            risk: self.risk_value,
            description: self.description.read(cx).value().trim().to_string(),
            content,
            parameters: vec![],
            url: String::new(),
            platform: Platform::Linux,
            starred: false,
            is_builtin: false,
        };
        self.saving = true;
        cx.notify();
        let task = self.store.update(cx, |s, cx| s.add_script(entry, cx));
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(_) => {
                        toast::success(window, cx, t!("scriptLib.added"));
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

impl Render for ScriptAddDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let e = &self.errors;
        let err = |k: &'static str, msg: String| e.contains(&k).then(|| SharedString::from(msg));
        v_flex()
            .id("script-add")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        field(
                            t!("scriptLib.scriptName"),
                            Input::new(&self.name),
                            err("name", t!("scriptLib.enterName")),
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            t!("scriptLib.language"),
                            Select::new(&self.language),
                            None,
                            cx,
                        )
                        .w(px(160.)),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        field(
                            t!("scriptLib.category"),
                            Select::new(&self.category),
                            err("category", t!("scriptLib.selectCategoryRequired")),
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(t!("scriptLib.riskLevel"), Select::new(&self.risk), None, cx)
                            .w(px(160.)),
                    ),
            )
            .child(field(
                t!("scriptLib.description"),
                Input::new(&self.description),
                None,
                cx,
            ))
            .child(field(
                t!("scriptLib.scriptCode"),
                self.code.clone(),
                err("content", t!("scriptLib.writeCode")),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sl-add-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("sl-add-ok")
                            .primary()
                            .label(t!("common.add"))
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
