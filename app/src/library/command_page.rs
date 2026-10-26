//! Command library page (and the Docker variant). Port of `pages/CommandLib/CommandLibPage.tsx`.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use super::model::{category_counts, category_options, filter_commands, CommandEntry, Kind, Platform, Risk};
use super::store::{library_store, LibraryStore};
use super::ui::*;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::field;
use crate::widgets::toast;

pub struct CommandLibPage {
    store: Entity<LibraryStore>,
    docker_only: bool,
    search: Entity<InputState>,
    category: Option<String>,
    _subs: Vec<Subscription>,
}

impl CommandLibPage {
    pub fn new(docker_only: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = library_store(cx);
        let placeholder = if docker_only { t!("dockerLib.searchPlaceholder") } else { t!("commandLib.searchPlaceholder") };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify()), cx.observe(&search, |_, _, cx| cx.notify())];
        store.update(cx, |s, cx| s.load_commands(cx));
        Self { store, docker_only, search, category: None, _subs: subs }
    }

    pub fn set_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.search.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    fn scoped<'a>(&self, all: &'a [CommandEntry]) -> Vec<&'a CommandEntry> {
        all.iter().filter(|c| self.docker_only == (c.kind == Kind::Docker)).collect()
    }

    pub fn add_to_quick_actions(&mut self, cmd: &CommandEntry, window: &mut Window, cx: &mut Context<Self>) {
        use super::model::QuickActionEntry;
        let entry = QuickActionEntry {
            id: String::new(),
            name: cmd.name.clone(),
            command: cmd.command.clone(),
            category: cmd.category.clone(),
            parameters_json: "[]".into(),
            order: 0,
            starred: false,
            description: cmd.description.clone(),
            tags: vec![],
            language: "shell".into(),
            last_run_at: String::new(),
            last_status: String::new(),
        };
        let task = self.store.update(cx, |s, cx| s.add_action(entry, cx));
        window
            .spawn(cx, async move |cx| {
                let r = task.await;
                let _ = cx.update(|window, cx| match r {
                    Ok(_) => toast::success(window, cx, t!("library.addedToQuickActions")),
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                });
            })
            .detach();
    }

    fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.store.update(cx, |s, cx| s.delete_command(id, cx));
        window
            .spawn(cx, async move |cx| {
                let r = task.await;
                let _ = cx.update(|window, cx| match r {
                    Ok(()) => toast::success(window, cx, t!("library.deleted")),
                    Err(e) => toast::error(window, cx, t!("common.deleteFailed", error = e)),
                });
            })
            .detach();
    }

    pub fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<CommandAddDialog> {
        let existing: Vec<String> = self.store.read(cx).commands.iter().map(|c| c.category.clone()).filter(|c| !c.trim().is_empty()).collect();
        let (store, docker_only) = (self.store.clone(), self.docker_only);
        let view = cx.new(|cx| CommandAddDialog::new(store, docker_only, existing, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| d.title(t!("commandLib.addCustomCommand")).w(px(580.)).child(shown.clone()));
        view
    }
}

impl Render for CommandLibPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = ();
        let ui = crate::theme::ui(cx);
        let query = self.search.read(cx).value().to_string();
        let (rows, total, counts): (Vec<CommandEntry>, usize, _) = {
            let all = &self.store.read(cx).commands;
            let scoped = self.scoped(all);
            let counts = category_counts(scoped.iter().map(|c| c.category.as_str()));
            let total = scoped.len();
            (filter_commands(all, self.docker_only, self.category.as_deref(), &query).into_iter().cloned().collect(), total, counts)
        };
        let docker_only = self.docker_only;
        let weak = cx.entity().downgrade();
        let rail_weak = weak.clone();

        let list = if rows.is_empty() {
            empty_state(if docker_only { t!("dockerLib.noCommands") } else { t!("commandLib.noCommands") }, cx).into_any_element()
        } else {
            v_flex()
                .id("clib-list")
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(4.))
                .gap(px(2.))
                .children(rows.into_iter().enumerate().map(|(ix, cmd)| {
                    let shown = cmd.display_command();
                    let (copy_text, c1, c2, id) = (shown.clone(), cmd.clone(), cmd.clone(), cmd.id.clone());
                    let (w1, w2) = (weak.clone(), weak.clone());
                    // `.clib-row`: 7px 10px, 12px gap, 8px corners; hairline + soft fill on hover, actions fade in
                    h_flex()
                        .id(("clib-row", ix))
                        .test_support()
                        .group("clib-row")
                        .h(px(53.))
                        .gap(px(12.))
                        .px(px(10.))
                        .py(px(7.))
                        .items_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(gpui_kit::transparent_black())
                        .hover(move |d| d.bg(ui.text_muted.opacity(0.04)).border_color(ui.border.opacity(0.7)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .line_height(relative(1.3))
                                .gap(px(2.))
                                .child(
                                    h_flex()
                                        .gap(px(6.))
                                        .items_center()
                                        .child(Icon::new(IconName::Code).size_4().text_color(ui.text_muted).opacity(0.7))
                                        .child(div().max_w(px(240.)).truncate().font_weight(FontWeight::SEMIBOLD).text_size(px(13.)).text_color(ui.text).child(cmd.name.clone()))
                                        .when(cmd.kind == Kind::Docker && !docker_only, |d| d.child(tag_chip_color("Docker", 0x1677ff)))
                                        .when(!cmd.url.is_empty(), |d| d.child(tag_chip_color(t!("library.remote"), 0x13c2c2)))
                                        .when(!cmd.parameters.is_empty(), |d| d.child(tag_chip(t!("library.parameters"), cx))),
                                )
                                .child(div().min_w_0().truncate().font_family("Menlo").text_size(px(11.)).text_color(ui.text_secondary).child(shown)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(risk_chip(cmd.risk, cx))
                                .when(cmd.platform != Platform::Linux, |d| d.child(tag_chip(t!(&format!("library.platformShort.{}", cmd.platform.as_str())), cx)))
                                .child(div().whitespace_nowrap().text_right().text_size(px(11.)).text_color(ui.text_muted).child(cmd.category.clone())),
                        )
                        .child(
                            h_flex()
                                .gap(px(2.))
                                .text_color(ui.text_muted)
                                .opacity(0.4)
                                .group_hover("clib-row", |d| d.opacity(1.))
                                .child(act_button(("clib-copy", ix), IconName::Copy, t!("commandLib.copyCommand"), move |_, window, cx| {
                                    copy_to_clipboard(copy_text.clone(), cx);
                                    toast::success(window, cx, t!("commandLib.copied"));
                                }))
                                .child(act_button(("clib-quick", ix), IconName::Zap, t!("library.addToQuickActions"), move |_, window, cx| {
                                    let c = c1.clone();
                                    let _ = w1.update(cx, |this, cx| this.add_to_quick_actions(&c, window, cx));
                                }))
                                .when(!c2.is_builtin, |d| {
                                    d.child(act_button_danger(("clib-delete", ix), IconName::Trash, t!("common.delete"), move |_, window, cx| {
                                        let id = id.clone();
                                        let _ = w2.update(cx, |this, cx| this.delete(id, window, cx));
                                    }))
                                }),
                        )
                }))
                .into_any_element()
        };

        let count = t!("commandLib.count", filtered = self.visible_count(cx), total = total);
        let add = Button::new("clib-add")
            .primary()
            .icon(Icon::new(IconName::Plus))
            .label(if docker_only { t!("dockerLib.addCommand") } else { t!("commandLib.addCommand") })
            .on_click(cx.listener(|this, _, window, cx| drop(this.open_add(window, cx))));
        let rail = category_rail("clib-cat", total, &counts, self.category.as_deref(), cx, move |value, _, cx| {
            let _ = rail_weak.update(cx, |this, cx| {
                this.category = value;
                cx.notify();
            });
        });
        page_frame(if docker_only { "docker-lib" } else { "command-lib" }, search_box(&self.search), count, add, rail, list, cx)
    }
}

impl CommandLibPage {
    fn visible_count(&self, cx: &App) -> usize {
        let q = self.search.read(cx).value().to_string();
        filter_commands(&self.store.read(cx).commands, self.docker_only, self.category.as_deref(), &q).len()
    }
}

/// "Add custom command" dialog.
pub struct CommandAddDialog {
    store: Entity<LibraryStore>,
    docker_only: bool,
    pub name: Entity<InputState>,
    pub command: Entity<TextareaState>,
    pub url: Entity<InputState>,
    pub description: Entity<InputState>,
    pub category: Entity<ChoiceSelect>,
    kind: Entity<ChoiceSelect>,
    risk: Entity<ChoiceSelect>,
    platform: Entity<ChoiceSelect>,
    kind_value: Kind,
    category_value: Option<String>,
    risk_value: Risk,
    platform_value: Platform,
    errors: Vec<&'static str>,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl CommandAddDialog {
    fn new(store: Entity<LibraryStore>, docker_only: bool, existing: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let presets = [
            t!("commandLib.category.cpu"),
            t!("commandLib.category.disk"),
            t!("commandLib.category.log"),
            t!("commandLib.category.memory"),
            t!("commandLib.category.network"),
            t!("commandLib.category.package"),
            t!("commandLib.category.process"),
            t!("commandLib.category.security"),
            t!("commandLib.category.systemInfo"),
            t!("commandLib.category.user"),
        ];
        let preset_refs: Vec<&str> = presets.iter().map(String::as_str).collect();
        let categories = category_options(&preset_refs, existing);
        let category = cx.new(|cx| new_select(categories.iter().map(|c| Choice::new(c.clone(), c.clone())).collect(), "", window, cx));
        let kind = cx.new(|cx| new_select(vec![Choice::new("command", t!("library.kindCommand")), Choice::new("docker", t!("library.kindDocker"))], "command", window, cx));
        let risk = cx.new(|cx| new_select(Risk::ALL.iter().map(|r| Choice::new(r.as_str(), t!(r.option_key()))).collect(), "low", window, cx));
        let platform = cx.new(|cx| {
            new_select(vec![Choice::new("linux", "Linux"), Choice::new("windows", "Windows"), Choice::new("both", t!("commandLib.platformGeneral"))], "linux", window, cx)
        });
        let subs = vec![
            cx.subscribe_in(&category, window, |this, _, ev, _, cx| {
                this.category_value = confirmed_value(ev);
                cx.notify();
            }),
            cx.subscribe_in(&kind, window, |this, _, ev, _, cx| {
                this.kind_value = if confirmed_value(ev).as_deref() == Some("docker") { Kind::Docker } else { Kind::Command };
                cx.notify();
            }),
            cx.subscribe_in(&risk, window, |this, _, ev, _, cx| {
                this.risk_value = Risk::parse(&confirmed_value(ev).unwrap_or_default());
                cx.notify();
            }),
            cx.subscribe_in(&platform, window, |this, _, ev, _, cx| {
                this.platform_value = Platform::parse(&confirmed_value(ev).unwrap_or_default());
                cx.notify();
            }),
        ];
        Self {
            store,
            docker_only,
            name: cx.new(|cx| InputState::new(window, cx).placeholder(t!("commandLib.commandNamePlaceholder"))),
            command: cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("commandLib.commandContentPlaceholder")).auto_grow(3, 6)),
            url: cx.new(|cx| InputState::new(window, cx).placeholder(t!("commandLib.remoteUrlPlaceholder"))),
            description: cx.new(|cx| InputState::new(window, cx).placeholder(t!("quickActions.descriptionPlaceholder"))),
            category,
            kind,
            risk,
            platform,
            kind_value: if docker_only { Kind::Docker } else { Kind::Command },
            category_value: None,
            risk_value: Risk::Low,
            platform_value: Platform::Linux,
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
        self.category.update(cx, |s, cx| s.set_selected_value(&v, window, cx));
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
        let entry = CommandEntry {
            id: String::new(),
            name,
            command,
            url: self.url.read(cx).value().trim().to_string(),
            kind: if self.docker_only { Kind::Docker } else { self.kind_value },
            category: self.category_value.clone().unwrap_or_default(),
            tags: vec![],
            risk: self.risk_value,
            description: self.description.read(cx).value().trim().to_string(),
            platform: self.platform_value,
            parameters: vec![],
            starred: false,
            is_builtin: false,
        };
        self.saving = true;
        cx.notify();
        let task = self.store.update(cx, |s, cx| s.add_command(entry, cx));
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(_) => {
                        toast::success(window, cx, t!("commandLib.commandAdded"));
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

impl Render for CommandAddDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let required = |key: &'static str, msg: String, errors: &[&'static str]| errors.contains(&key).then(|| SharedString::from(msg));
        let e = &self.errors;
        v_flex()
            .id("command-add-dialog")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(field(t!("commandLib.commandName"), Input::new(&self.name), required("name", t!("commandLib.enterCommandName"), e), cx).flex_1())
                    .when(!self.docker_only, |d| d.child(field(t!("library.type"), Select::new(&self.kind), None, cx).w(px(150.)))),
            )
            .child(field(t!("quickActions.category"), Select::new(&self.category), required("category", t!("scriptLib.selectCategoryRequired"), e), cx))
            .child(field(t!("commandLib.commandContent"), Textarea::new(&self.command), required("command", t!("commandLib.enterCommandContent"), e), cx))
            .child(field(t!("commandLib.remoteUrl"), Input::new(&self.url), None, cx))
            .child(field(t!("scriptLib.description"), Input::new(&self.description), None, cx))
            .child(
                h_flex()
                    .gap_3()
                    .child(field(t!("scriptLib.riskLevel"), Select::new(&self.risk), None, cx).flex_1())
                    .child(field(t!("commandLib.platform"), Select::new(&self.platform), None, cx).flex_1()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("cmd-add-cancel").label(t!("common.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("cmd-add-ok").primary().label(t!("common.add")).loading(self.saving).on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))),
            )
    }
}
