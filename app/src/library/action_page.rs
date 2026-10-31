//! Quick actions page. Port of `pages/QuickActions/QuickActionsPage.tsx`.

use crate::scale::px;
use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_library::params::{
    parse_quick_action_params, replace_quick_action_params, QuickActionParam,
};

use super::exec;
use super::model::{
    category_counts, category_options, filter_actions, last_run_bucket, LastRun, QuickActionEntry,
};
use super::store::{library_store, LibraryStore};
use super::ui::*;
use crate::stores::assets::AssetsStore;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::field;
use crate::widgets::toast;

pub fn last_run_text(ts: &str) -> String {
    match last_run_bucket(ts, Local::now().naive_local()) {
        LastRun::JustNow => t!("quickActions.justNow"),
        LastRun::Minutes(n) => t!("quickActions.minutesAgo", count = n),
        LastRun::Hours(n) => t!("quickActions.hoursAgo", count = n),
        LastRun::Days(n) => t!("quickActions.daysAgo", count = n),
        LastRun::Date(d) => d,
        LastRun::Unknown => String::new(),
    }
}

pub struct QuickActionsPage {
    store: Entity<LibraryStore>,
    assets: Entity<AssetsStore>,
    search: Entity<InputState>,
    category: Option<String>,
    _subs: Vec<Subscription>,
}

impl QuickActionsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = library_store(cx);
        let assets = cx.global::<crate::ui_state::UiState>().assets.clone();
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("quickActions.searchPlaceholder"))
        });
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&assets, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        store.update(cx, |s, cx| s.load_actions(cx));
        Self {
            store,
            assets,
            search,
            category: None,
            _subs: subs,
        }
    }

    fn selected_hosts(&self, cx: &App) -> Vec<String> {
        self.assets.read(cx).selected.iter().cloned().collect()
    }

    /// Run the command on the selected hosts; the list is refreshed when the task is done (last run / status).
    fn dispatch(
        &mut self,
        action_id: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hosts = self.selected_hosts(cx);
        let count = hosts.len();
        let ctx = crate::services::ctx(cx);
        let task_id = uuid::Uuid::new_v4().to_string();
        let done = ctx.events.subscribe(format!("exec:{task_id}:done"));
        let store = self.store.clone();
        let shown = command.clone();
        cx.spawn_in(window, async move |_, cx| {
            let result =
                exec::dispatch(ctx, task_id, hosts, command, 10, 30, Some(action_id)).await;
            let _ = cx.update(|window, cx| match &result {
                Ok(_) => toast::success(
                    window,
                    cx,
                    t!("quickActions.dispatched", command = shown, count = count),
                ),
                Err(e) => toast::error(
                    window,
                    cx,
                    t!("quickActions.executeFailed", error = e.clone()),
                ),
            });
            if result.is_ok() && done.recv_async().await.is_ok() {
                let _ = cx.update(|_, cx| store.update(cx, |s, cx| s.load_actions(cx)));
            }
        })
        .detach();
    }

    pub fn execute(
        &mut self,
        action: &QuickActionEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ParamDialog>> {
        if self.selected_hosts(cx).is_empty() {
            toast::warning(window, cx, t!("quickActions.selectHostsFirst"));
            return None;
        }
        let params = parse_quick_action_params(&action.command);
        if params.is_empty() {
            self.dispatch(action.id.clone(), action.command.clone(), window, cx);
            return None;
        }
        let weak = cx.entity().downgrade();
        let action = action.clone();
        let view = cx.new(|cx| ParamDialog::new(action.clone(), params, weak, window, cx));
        let shown = view.clone();
        let title = t!("quickActions.fillParams", name = action.name.clone());
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(450.)).child(shown.clone())
        });
        Some(view)
    }

    pub fn open_edit(
        &mut self,
        action: Option<QuickActionEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ActionFormDialog> {
        let existing: Vec<String> = self
            .store
            .read(cx)
            .actions
            .iter()
            .map(|a| a.category.clone())
            .filter(|c| !c.trim().is_empty())
            .collect();
        let store = self.store.clone();
        let editing = action.is_some();
        let view = cx.new(|cx| ActionFormDialog::new(store, action, existing, window, cx));
        let shown = view.clone();
        let title = if editing {
            t!("quickActions.editAction")
        } else {
            t!("quickActions.newAction")
        };
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(540.)).child(shown.clone())
        });
        view
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.store.read(cx).actions.clone();
        if actions.is_empty() {
            return;
        }
        let json =
            serde_json::to_string_pretty(&actions.iter().map(export_json).collect::<Vec<_>>())
                .unwrap_or_default();
        let dir = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some("quick-actions.json"));
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = std::fs::write(&path, json).is_ok();
                let _ = cx.update(|window, cx| {
                    if ok {
                        toast::success(window, cx, t!("quickActions.exportSuccess"))
                    } else {
                        toast::error(window, cx, t!("common.exportFailed"))
                    }
                });
            }
        })
        .detach();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let store = self.store.clone();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let parsed = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| import_json(&raw));
            match parsed {
                Some(items) => {
                    let count = items.len();
                    for item in items {
                        let task =
                            cx.update(|_, cx| store.update(cx, |s, cx| s.add_action(item, cx)));
                        if let Ok(task) = task {
                            let _ = task.await;
                        }
                    }
                    let _ = cx.update(|window, cx| {
                        toast::success(window, cx, t!("quickActions.imported", count = count))
                    });
                }
                None => {
                    let _ = cx.update(|window, cx| {
                        toast::error(window, cx, t!("quickActions.importFailed"))
                    });
                }
            }
        })
        .detach();
    }
}

fn export_json(a: &QuickActionEntry) -> serde_json::Value {
    serde_json::json!({
        "id": a.id, "name": a.name, "command": a.command, "category": a.category,
        "parameters": serde_json::from_str::<serde_json::Value>(&a.parameters_json).unwrap_or(serde_json::json!([])),
        "order": a.order, "starred": a.starred, "description": a.description, "tags": a.tags,
        "language": a.language, "lastRunAt": a.last_run_at, "lastStatus": a.last_status,
    })
}

/// Entries of an exported file (any JSON array of objects with at least name + command).
pub fn import_json(raw: &str) -> Option<Vec<QuickActionEntry>> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let arr = v.as_array()?;
    arr.iter()
        .map(|item| {
            let s = |k: &str| {
                item.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let name = s("name");
            let command = s("command");
            if name.is_empty() || command.is_empty() {
                return None;
            }
            Some(QuickActionEntry {
                id: String::new(),
                name,
                command,
                category: s("category"),
                parameters_json: item
                    .get("parameters")
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "[]".into()),
                order: 0,
                starred: false,
                description: s("description"),
                tags: item
                    .get("tags")
                    .and_then(|t| serde_json::from_value(t.clone()).ok())
                    .unwrap_or_default(),
                language: {
                    let l = s("language");
                    if l.is_empty() {
                        "shell".into()
                    } else {
                        l
                    }
                },
                last_run_at: String::new(),
                last_status: String::new(),
            })
        })
        .collect()
}

impl Render for QuickActionsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let query = self.search.read(cx).value().to_string();
        let hosts_selected = self.selected_hosts(cx).len();
        let (rows, total, counts): (Vec<QuickActionEntry>, usize, _) = {
            let all = &self.store.read(cx).actions;
            (
                filter_actions(all, self.category.as_deref(), &query)
                    .into_iter()
                    .cloned()
                    .collect(),
                all.len(),
                category_counts(all.iter().map(|a| a.category.as_str())),
            )
        };
        let filtered = rows.len();
        let weak = cx.entity().downgrade();
        let rail_weak = weak.clone();

        let list = if rows.is_empty() {
            empty_state(t!("quickActions.noMatch"), cx).into_any_element()
        } else {
            v_flex()
                .id("qa-list")
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(4.))
                .gap(px(2.))
                .children(rows.into_iter().enumerate().map(|(ix, a)| {
                    let has_params = !parse_quick_action_params(&a.command).is_empty();
                    let status = match a.last_status.as_str() {
                        "success" => Some((t!("quickActions.status.success"), theme.success)),
                        "failed" => Some((t!("quickActions.status.failed"), theme.danger)),
                        "partial" => Some((t!("quickActions.status.partial"), theme.warning)),
                        _ => None,
                    };
                    let (w_run, w_edit, w_del) = (weak.clone(), weak.clone(), weak.clone());
                    let (a_run, a_edit, id) = (a.clone(), a.clone(), a.id.clone());
                    h_flex()
                        .id(("qa-row", ix))
                        .test_support()
                        .group("qa-row")
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
                                                .child(lang_label(&a.language)),
                                        )
                                        .child(
                                            div()
                                                .max_w(px(240.))
                                                .truncate()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(13.))
                                                .text_color(ui.text)
                                                .child(a.name.clone()),
                                        )
                                        .when(has_params, |d| {
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
                                        .child(a.command.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .when_some(status, |d, (text, color)| {
                                    d.child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .text_color(color)
                                            .child(div().size(px(6.)).rounded_full().bg(color))
                                            .child(text),
                                    )
                                })
                                .when(!a.last_run_at.is_empty(), |d| {
                                    d.child(last_run_text(&a.last_run_at))
                                })
                                .child(div().w(px(70.)).truncate().child(a.category.clone())),
                        )
                        .child(
                            h_flex()
                                .gap(px(2.))
                                .text_color(ui.text_muted)
                                .opacity(0.4)
                                .group_hover("qa-row", |d| d.opacity(1.))
                                .child(
                                    Button::new(("qa-run", ix))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::CirclePlay))
                                        .tooltip(t!("quickActions.execute"))
                                        .disabled(hosts_selected == 0)
                                        .on_click(move |_, window, cx| {
                                            let a = a_run.clone();
                                            let _ = w_run.update(cx, |this, cx| {
                                                drop(this.execute(&a, window, cx))
                                            });
                                        }),
                                )
                                .child(act_button(
                                    ("qa-edit", ix),
                                    IconName::SquarePen,
                                    t!("common.edit"),
                                    move |_, window, cx| {
                                        let a = a_edit.clone();
                                        let _ = w_edit.update(cx, |this, cx| {
                                            drop(this.open_edit(Some(a), window, cx))
                                        });
                                    },
                                ))
                                .child(act_button(
                                    ("qa-delete", ix),
                                    IconName::Trash,
                                    t!("common.delete"),
                                    move |_, window, cx| {
                                        let id = id.clone();
                                        let _ = w_del.update(cx, |this, cx| {
                                            let task = this
                                                .store
                                                .update(cx, |s, cx| s.delete_action(id, cx));
                                            window
                                                .spawn(cx, async move |cx| {
                                                    let r = task.await;
                                                    let _ = cx.update(|window, cx| match r {
                                                        Ok(()) => toast::success(
                                                            window,
                                                            cx,
                                                            t!("library.deleted"),
                                                        ),
                                                        Err(e) => toast::error(
                                                            window,
                                                            cx,
                                                            t!("common.deleteFailed", error = e),
                                                        ),
                                                    });
                                                })
                                                .detach();
                                        });
                                    },
                                )),
                        )
                }))
                .into_any_element()
        };

        let count = t!("quickActions.count", filtered = filtered, total = total);
        let actions = h_flex()
            .gap(px(8.))
            .child(
                Button::new("qa-import")
                    .icon(Icon::new(IconName::Import))
                    .label(t!("quickActions.import"))
                    .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
            )
            .child(
                Button::new("qa-export")
                    .icon(Icon::new(IconName::Download))
                    .label(t!("quickActions.export"))
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
            )
            .child(
                Button::new("qa-create")
                    .primary()
                    .icon(Icon::new(IconName::Plus))
                    .label(t!("quickActions.createAction"))
                    .on_click(
                        cx.listener(|this, _, window, cx| drop(this.open_edit(None, window, cx))),
                    ),
            );
        let rail = category_rail(
            "qa-cat",
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
        v_flex()
            .size_full()
            .gap(px(10.))
            .when(hosts_selected > 0, |d| {
                d.child(
                    div()
                        .px(px(12.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .text_size(px(12.))
                        .bg(ui.primary.opacity(0.1))
                        .text_color(ui.primary)
                        .child(t!("quickActions.selectedHostHint", count = hosts_selected)),
                )
            })
            .child(div().flex_1().min_h_0().child(page_frame(
                "quick-actions",
                search_box(&self.search),
                count,
                actions,
                rail,
                list,
                cx,
            )))
    }
}

// ---- parameter dialog ----

pub struct ParamDialog {
    action: QuickActionEntry,
    params: Vec<QuickActionParam>,
    pub inputs: Vec<Entity<InputState>>,
    missing: Vec<usize>,
    page: WeakEntity<QuickActionsPage>,
}

impl ParamDialog {
    fn new(
        action: QuickActionEntry,
        params: Vec<QuickActionParam>,
        page: WeakEntity<QuickActionsPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = params
            .iter()
            .map(|p| {
                let ph = if p.default_value.is_empty() {
                    t!("quickActions.enterParam", name = p.name.clone())
                } else {
                    t!("quickActions.paramDefault", value = p.default_value.clone())
                };
                let dv = p.default_value.clone();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(ph)
                        .default_value(dv)
                })
            })
            .collect();
        Self {
            action,
            params,
            inputs,
            missing: Vec::new(),
            page,
        }
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values: Vec<String> = self
            .inputs
            .iter()
            .map(|i| i.read(cx).value().to_string())
            .collect();
        self.missing = values
            .iter()
            .enumerate()
            .filter(|(_, v)| v.trim().is_empty())
            .map(|(i, _)| i)
            .collect();
        if !self.missing.is_empty() {
            cx.notify();
            return;
        }
        let map = self
            .params
            .iter()
            .zip(values)
            .map(|(p, v)| (p.name.clone(), v))
            .collect();
        let command = replace_quick_action_params(&self.action.command, &map);
        let id = self.action.id.clone();
        let _ = self
            .page
            .update(cx, |page, cx| page.dispatch(id, command, window, cx));
        window.close_dialog(cx);
    }
}

impl Render for ParamDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("qa-param-dialog")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_1()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child(t!("quickActions.commandLabel"))
                    .child(
                        div()
                            .font_family("Menlo")
                            .child(self.action.command.clone()),
                    ),
            )
            .children(self.params.iter().enumerate().map(|(i, p)| {
                field(
                    p.name.clone(),
                    Input::new(&self.inputs[i]),
                    self.missing.contains(&i).then(|| {
                        SharedString::from(t!("quickActions.enterParam", name = p.name.clone()))
                    }),
                    cx,
                )
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("qa-param-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("qa-param-run")
                            .primary()
                            .label(t!("quickActions.execute"))
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

// ---- create / edit ----

pub struct ActionFormDialog {
    store: Entity<LibraryStore>,
    editing: Option<QuickActionEntry>,
    pub name: Entity<InputState>,
    pub command: Entity<TextareaState>,
    pub description: Entity<InputState>,
    pub tags: Entity<InputState>,
    category: Entity<ChoiceSelect>,
    language: Entity<ChoiceSelect>,
    category_value: Option<String>,
    language_value: String,
    errors: Vec<&'static str>,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl ActionFormDialog {
    fn new(
        store: Entity<LibraryStore>,
        editing: Option<QuickActionEntry>,
        existing: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let presets = [
            t!("quickActions.category.system"),
            t!("quickActions.category.disk"),
            t!("quickActions.category.memory"),
            t!("quickActions.category.network"),
            t!("quickActions.category.process"),
            t!("quickActions.category.security"),
            t!("quickActions.category.other"),
        ];
        let refs: Vec<&str> = presets.iter().map(String::as_str).collect();
        let mut categories = category_options(&refs, existing);
        if let Some(e) = &editing {
            if !e.category.is_empty() && !categories.contains(&e.category) {
                categories.push(e.category.clone());
            }
        }
        let current_cat = editing
            .as_ref()
            .map(|e| e.category.clone())
            .filter(|c| !c.is_empty());
        let category = cx.new(|cx| {
            new_select(
                categories
                    .iter()
                    .map(|c| Choice::new(c.clone(), c.clone()))
                    .collect(),
                current_cat.as_deref().unwrap_or(""),
                window,
                cx,
            )
        });
        let lang_now = editing
            .as_ref()
            .map(|e| e.language.clone())
            .unwrap_or_else(|| "shell".into());
        let language = cx.new(|cx| {
            new_select(
                vec![
                    Choice::new("shell", "Shell"),
                    Choice::new("python", "Python"),
                    Choice::new("powershell", "PowerShell"),
                ],
                &lang_now,
                window,
                cx,
            )
        });
        let subs = vec![
            cx.subscribe_in(&category, window, |this, _, ev, _, cx| {
                this.category_value = confirmed_value(ev);
                cx.notify();
            }),
            cx.subscribe_in(&language, window, |this, _, ev, _, _| {
                if let Some(v) = confirmed_value(ev) {
                    this.language_value = v;
                }
            }),
        ];
        let e = editing.clone();
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("quickActions.actionNamePlaceholder"))
                .default_value(e.as_ref().map(|e| e.name.clone()).unwrap_or_default())
        });
        let command = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("quickActions.commandPlaceholder"))
                .auto_grow(3, 6)
        });
        if let Some(e) = &editing {
            let c = e.command.clone();
            command.update(cx, |s, cx| s.set_value(c, window, cx));
        }
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("quickActions.descriptionPlaceholder"))
                .default_value(
                    e.as_ref()
                        .map(|e| e.description.clone())
                        .unwrap_or_default(),
                )
        });
        let tags = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("quickActions.tagsPlaceholder"))
                .default_value(e.as_ref().map(|e| e.tags.join(", ")).unwrap_or_default())
        });
        Self {
            store,
            category_value: current_cat,
            language_value: lang_now,
            editing,
            name,
            command,
            description,
            tags,
            category,
            language,
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
        let command = self.command.read(cx).value().trim().to_string();
        self.errors.clear();
        if name.is_empty() {
            self.errors.push("name");
        }
        if self.category_value.is_none() {
            self.errors.push("category");
        }
        if command.is_empty() {
            self.errors.push("command");
        }
        if !self.errors.is_empty() {
            cx.notify();
            return;
        }
        let tags: Vec<String> = self
            .tags
            .read(cx)
            .value()
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let base = self.editing.clone().unwrap_or(QuickActionEntry {
            id: String::new(),
            name: String::new(),
            command: String::new(),
            category: String::new(),
            parameters_json: "[]".into(),
            order: 0,
            starred: false,
            description: String::new(),
            tags: vec![],
            language: "shell".into(),
            last_run_at: String::new(),
            last_status: String::new(),
        });
        let entry = QuickActionEntry {
            name,
            command,
            category: self.category_value.clone().unwrap_or_default(),
            description: self.description.read(cx).value().trim().to_string(),
            tags,
            language: self.language_value.clone(),
            ..base
        };
        let editing = self.editing.is_some();
        self.saving = true;
        cx.notify();
        let task = self.store.update(cx, |s, cx| {
            if editing {
                let t = s.update_action(entry, cx);
                cx.spawn(async move |_, _| t.await.map(|_| String::new()))
            } else {
                s.add_action(entry, cx)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(_) => {
                        toast::success(
                            window,
                            cx,
                            if editing {
                                t!("quickActions.updated")
                            } else {
                                t!("quickActions.added")
                            },
                        );
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

impl Render for ActionFormDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let e = &self.errors;
        let err = |k: &'static str, msg: String| e.contains(&k).then(|| SharedString::from(msg));
        v_flex()
            .id("qa-form")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        field(
                            t!("quickActions.actionName"),
                            Input::new(&self.name),
                            err("name", t!("quickActions.enterActionName")),
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            t!("quickActions.category"),
                            Select::new(&self.category),
                            err("category", t!("scriptLib.selectCategoryRequired")),
                            cx,
                        )
                        .flex_1(),
                    ),
            )
            .child(field(
                t!("quickActions.commandContent"),
                Textarea::new(&self.command),
                err("command", t!("quickActions.enterCommandContent")),
                cx,
            ))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(t!("quickActions.paramPlaceholder", open = "{", close = "}")),
            )
            .child(field(
                t!("quickActions.description"),
                Input::new(&self.description),
                None,
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        field(
                            t!("quickActions.language"),
                            Select::new(&self.language),
                            None,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(t!("quickActions.tags"), Input::new(&self.tags), None, cx).flex_1(),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("qa-form-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("qa-form-ok")
                            .primary()
                            .label(if self.editing.is_some() {
                                t!("common.save")
                            } else {
                                t!("quickActions.create")
                            })
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::import_json;

    #[test]
    fn import_accepts_exported_shape_and_rejects_garbage() {
        let raw = r#"[{"id":"1","name":"disk","command":"df -h","category":"磁盘","parameters":[],"tags":["a"],"language":"shell","lastRunAt":"x"},{"name":"mem","command":"free -m"}]"#;
        let items = import_json(raw).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            (
                items[0].name.as_str(),
                items[0].tags.clone(),
                items[0].last_run_at.as_str()
            ),
            ("disk", vec!["a".to_string()], "")
        );
        assert_eq!(
            (
                items[1].category.as_str(),
                items[1].language.as_str(),
                items[1].parameters_json.as_str()
            ),
            ("", "shell", "[]")
        );
        assert!(import_json("not json").is_none());
        assert!(import_json(r#"{"a":1}"#).is_none());
        assert!(
            import_json(r#"[{"name":"x"}]"#).is_none(),
            "command is required"
        );
    }
}
