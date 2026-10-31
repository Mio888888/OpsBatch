//! Safety tab: permission mode, timeouts, iteration cap, command blocklist and the permission grants table
//! (`SafetySettings` + `PermissionGrantsSettings`).

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::prefs::safety::{
    self, PermissionGrantRule, GRANTABLE_CAPABILITIES, MAX_COMMAND_TIMEOUT_SECONDS,
    MAX_RESPONSE_IDLE_TIMEOUT_SECONDS,
};
use ops_ai::prefs::PermissionMode;
use ops_i18n::t;

use super::ui;
use crate::ai::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

struct GrantRow {
    id: String,
    capability: Entity<ChoiceSelect>,
    command: Entity<InputState>,
    note: Entity<InputState>,
    _subs: Vec<Subscription>,
}

struct BlockRow {
    input: Entity<InputState>,
    error: Option<String>,
    _sub: Subscription,
}

pub struct SafetyTab {
    store: Entity<PrefsStore>,
    mode: Entity<ChoiceSelect>,
    response_idle: Entity<InputState>,
    command_timeout: Entity<InputState>,
    max_iterations: Entity<InputState>,
    blocklist: Vec<BlockRow>,
    grants: Vec<GrantRow>,
    import_error: Option<String>,
    _subs: Vec<Subscription>,
}

fn mode_choices() -> Vec<Choice> {
    PermissionMode::ALL
        .iter()
        .map(|m| {
            Choice::new(
                m.as_str(),
                t!(&format!("ncai.safety.permissionMode.{}", m.as_str())),
            )
        })
        .collect()
}

fn capability_choices(current: &str) -> Vec<Choice> {
    let label = |id: &str| {
        if id == "*" {
            t!("ncai.externalMcp.allowHost")
        } else {
            id.to_string()
        }
    };
    let mut ids: Vec<&str> = GRANTABLE_CAPABILITIES.to_vec();
    if !ids.contains(&current) {
        ids.insert(0, current);
    }
    ids.into_iter()
        .map(|id| Choice::new(id, label(id)))
        .collect()
}

impl SafetyTab {
    pub fn new(store: Entity<PrefsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mode = cx.new(|cx| new_select(mode_choices(), "confirm", window, cx));
        let response_idle = cx.new(|cx| InputState::new(window, cx));
        let command_timeout = cx.new(|cx| InputState::new(window, cx));
        let max_iterations = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![
            cx.subscribe_in(&store, window, |this, _, ev: &PrefsEvent, window, cx| {
                if matches!(ev, PrefsEvent::Loaded) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&mode, window, |this, _, ev, _, cx| {
                if let Some(m) = confirmed_value(ev).and_then(|v| PermissionMode::parse(&v)) {
                    this.set_mode(m, cx);
                }
            }),
            ui::on_commit(&response_idle, window, cx, |this, text, window, cx| {
                this.commit_number(&text, Field::ResponseIdle, window, cx)
            }),
            ui::on_commit(&command_timeout, window, cx, |this, text, window, cx| {
                this.commit_number(&text, Field::CommandTimeout, window, cx)
            }),
            ui::on_commit(&max_iterations, window, cx, |this, text, window, cx| {
                this.commit_number(&text, Field::MaxIterations, window, cx)
            }),
        ];
        let mut this = Self {
            store,
            mode,
            response_idle,
            command_timeout,
            max_iterations,
            blocklist: Vec::new(),
            grants: Vec::new(),
            import_error: None,
            _subs: subs,
        };
        this.sync(window, cx);
        this
    }

    // ---- accessors for tests ----

    pub fn response_idle(&self) -> &Entity<InputState> {
        &self.response_idle
    }
    pub fn command_timeout(&self) -> &Entity<InputState> {
        &self.command_timeout
    }
    pub fn max_iterations(&self) -> &Entity<InputState> {
        &self.max_iterations
    }
    pub fn blocklist_inputs(&self) -> Vec<Entity<InputState>> {
        self.blocklist.iter().map(|r| r.input.clone()).collect()
    }
    pub fn blocklist_error(&self, ix: usize) -> Option<&str> {
        self.blocklist.get(ix).and_then(|r| r.error.as_deref())
    }
    pub fn grant_inputs(&self) -> Vec<(String, Entity<InputState>, Entity<InputState>)> {
        self.grants
            .iter()
            .map(|g| (g.id.clone(), g.command.clone(), g.note.clone()))
            .collect()
    }
    pub fn import_error(&self) -> Option<&str> {
        self.import_error.as_deref()
    }

    fn prefs_mode(&self, cx: &App) -> PermissionMode {
        self.store.read(cx).prefs().global_permission_mode
    }

    /// Rebuilds every field from the stored preferences.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = self.store.read(cx).prefs().clone();
        let mode = prefs.global_permission_mode;
        self.mode.update(cx, |s, cx| {
            s.set_selected_value(&mode.as_str().to_string(), window, cx)
        });
        ui::set_input(
            &self.response_idle,
            &prefs.response_idle_timeout.to_string(),
            window,
            cx,
        );
        ui::set_input(
            &self.command_timeout,
            &prefs.command_timeout.to_string(),
            window,
            cx,
        );
        ui::set_input(
            &self.max_iterations,
            &prefs.max_iterations.to_string(),
            window,
            cx,
        );
        self.rebuild_blocklist(&prefs.command_blocklist, window, cx);
        self.rebuild_grants(&prefs.permission_grants, window, cx);
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.global_permission_mode = mode)
        });
    }

    fn commit_number(
        &mut self,
        text: &str,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parsed = ui::parse_int(text).filter(|n| match field {
            Field::MaxIterations => *n > 0,
            _ => true,
        });
        if let Some(n) = parsed {
            self.store.update(cx, |s, cx| {
                s.update(cx, |p| match field {
                    Field::ResponseIdle => {
                        p.response_idle_timeout = safety::normalize_response_idle_timeout(n)
                    }
                    Field::CommandTimeout => {
                        p.command_timeout = safety::normalize_command_timeout(n)
                    }
                    Field::MaxIterations => p.max_iterations = safety::normalize_max_iterations(n),
                })
            })
        }
        // show what was actually stored (clamped, or the old value after invalid input)
        let prefs = self.store.read(cx).prefs().clone();
        let (input, value) = match field {
            Field::ResponseIdle => (&self.response_idle, prefs.response_idle_timeout),
            Field::CommandTimeout => (&self.command_timeout, prefs.command_timeout),
            Field::MaxIterations => (&self.max_iterations, prefs.max_iterations),
        };
        ui::set_input(input, &value.to_string(), window, cx);
    }

    // ---- blocklist ----

    fn rebuild_blocklist(
        &mut self,
        patterns: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blocklist = patterns
            .iter()
            .enumerate()
            .map(|(ix, pattern)| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("ncai.safety.blocklist.placeholder"))
                });
                input.update(cx, |s, cx| s.set_value(pattern.clone(), window, cx));
                // validated while typing, stored when the field is left (a half-typed regex must not block anything)
                let sub = cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, ev: &gpui_kit::component::input::InputEvent, window, cx| {
                        use gpui_kit::component::input::InputEvent;
                        let text = input.read(cx).value().to_string();
                        match ev {
                            InputEvent::Change => {
                                if let Some(row) = this.blocklist.get_mut(ix) {
                                    row.error = safety::validate_pattern(&text).err();
                                }
                                cx.notify();
                            }
                            InputEvent::Blur | InputEvent::PressEnter { .. } => {
                                this.commit_pattern(ix, text, window, cx)
                            }
                            _ => {}
                        }
                    },
                );
                BlockRow {
                    input,
                    error: safety::validate_pattern(pattern).err(),
                    _sub: sub,
                }
            })
            .collect();
    }

    fn commit_pattern(
        &mut self,
        ix: usize,
        text: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if safety::validate_pattern(&text).is_err() {
            return;
        }
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if let Some(slot) = p.command_blocklist.get_mut(ix) {
                    *slot = text;
                }
            })
        });
    }

    pub fn add_pattern(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.command_blocklist.push(String::new()))
        });
        let list = self.store.read(cx).prefs().command_blocklist.clone();
        self.rebuild_blocklist(&list, window, cx);
        cx.notify();
    }

    pub fn remove_pattern(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if ix < p.command_blocklist.len() {
                    p.command_blocklist.remove(ix);
                }
            })
        });
        let list = self.store.read(cx).prefs().command_blocklist.clone();
        self.rebuild_blocklist(&list, window, cx);
        cx.notify();
    }

    pub fn reset_blocklist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                p.command_blocklist = safety::default_command_blocklist()
            })
        });
        let list = self.store.read(cx).prefs().command_blocklist.clone();
        self.rebuild_blocklist(&list, window, cx);
        cx.notify();
    }

    // ---- permission grants ----

    fn rebuild_grants(
        &mut self,
        rules: &[PermissionGrantRule],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.grants = rules
            .iter()
            .map(|rule| {
                let id = rule.id.clone();
                let capability = cx.new(|cx| {
                    new_select(
                        capability_choices(&rule.capability_id),
                        &rule.capability_id,
                        window,
                        cx,
                    )
                });
                let command = cx.new(|cx| InputState::new(window, cx).placeholder("lscpu *"));
                command.update(cx, |s, cx| {
                    s.set_value(rule.command_pattern.clone().unwrap_or_default(), window, cx)
                });
                let note = cx.new(|cx| InputState::new(window, cx));
                note.update(cx, |s, cx| {
                    s.set_value(rule.note.clone().unwrap_or_default(), window, cx)
                });
                let (id_a, id_b, id_c) = (id.clone(), id.clone(), id.clone());
                let subs = vec![
                    cx.subscribe_in(&capability, window, move |this, _, ev, _, cx| {
                        if let Some(cap) = confirmed_value(ev) {
                            this.set_grant_capability(&id_a, &cap, cx);
                        }
                    }),
                    ui::on_commit(&command, window, cx, move |this, text, _, cx| {
                        let pattern = Some(text.trim().to_string()).filter(|p| !p.is_empty());
                        // only shell capabilities take a command pattern
                        this.update_grant(&id_b, cx, |r| {
                            if safety::supports_command_pattern(&r.capability_id) {
                                r.command_pattern = pattern;
                            }
                        });
                    }),
                    ui::on_commit(&note, window, cx, move |this, text, _, cx| {
                        let note = Some(text.trim().to_string()).filter(|p| !p.is_empty());
                        this.update_grant(&id_c, cx, |r| r.note = note);
                    }),
                ];
                GrantRow {
                    id,
                    capability,
                    command,
                    note,
                    _subs: subs,
                }
            })
            .collect();
    }

    /// Changes what a rule grants; a capability without command patterns drops the pattern.
    pub fn set_grant_capability(&mut self, id: &str, capability: &str, cx: &mut Context<Self>) {
        let cap = capability.to_string();
        self.update_grant(id, cx, |r| {
            if !safety::supports_command_pattern(&cap) {
                r.command_pattern = None;
            }
            r.capability_id = cap;
        });
        cx.notify();
    }

    fn update_grant(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut PermissionGrantRule),
    ) {
        let id = id.to_string();
        let changed_capability = std::cell::Cell::new(false);
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if let Some(rule) = p.permission_grants.iter_mut().find(|r| r.id == id) {
                    let before = rule.capability_id.clone();
                    f(rule);
                    changed_capability.set(before != rule.capability_id);
                }
            })
        });
        if changed_capability.get() {
            cx.notify();
        }
    }

    pub fn add_grant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.permission_grants.push(safety::new_grant()))
        });
        let rules = self.store.read(cx).prefs().permission_grants.clone();
        self.rebuild_grants(&rules, window, cx);
        cx.notify();
    }

    pub fn remove_grant(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.permission_grants.retain(|r| r.id != id))
        });
        let rules = self.store.read(cx).prefs().permission_grants.clone();
        self.rebuild_grants(&rules, window, cx);
        cx.notify();
    }

    /// Replaces the table with the rules of a JSON document (what the "import" button does after reading the file).
    pub fn import_json(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.import_error = None;
        let imported = serde_json::from_str::<serde_json::Value>(text)
            .map_err(|e| e.to_string())
            .and_then(|v| {
                let current = self.store.read(cx).prefs().permission_grants.clone();
                safety::import_grants(&current, &v, true)
            });
        match imported {
            Ok(rules) => {
                self.store
                    .update(cx, |s, cx| s.update(cx, |p| p.permission_grants = rules));
                let rules = self.store.read(cx).prefs().permission_grants.clone();
                self.rebuild_grants(&rules, window, cx);
            }
            Err(e) => self.import_error = Some(e),
        }
        cx.notify();
    }

    pub fn export_json(&self, cx: &App) -> String {
        serde_json::to_string_pretty(&self.store.read(cx).prefs().permission_grants)
            .unwrap_or_else(|_| "[]".into())
    }

    fn export_to_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let json = self.export_json(cx);
        let dir = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some("opsbatch-permission-grants.json"));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                if let Err(e) = std::fs::write(&path, json) {
                    let _ = this.update(cx, |this, cx| {
                        this.import_error = Some(e.to_string());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn import_from_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string());
            let _ = this.update_in(cx, |this, window, cx| match text {
                Ok(text) => this.import_json(&text, window, cx),
                Err(e) => {
                    this.import_error = Some(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[derive(Clone, Copy)]
enum Field {
    ResponseIdle,
    CommandTimeout,
    MaxIterations,
}

impl Render for SafetyTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let unit = |key: &str| {
            div()
                .text_size(px(12.))
                .text_color(ui_colors.text_muted)
                .child(t!(key))
        };
        let _ = self.prefs_mode(cx);

        let number = |input: &Entity<InputState>, unit_key: Option<&str>| {
            h_flex()
                .gap(px(8.))
                .items_center()
                .child(ui::input(input, 80.))
                .when_some(unit_key.map(str::to_string), |d, k| d.child(unit(&k)))
        };
        let settings = ui::card(
            vec![
                ui::setting_row(
                    Some(t!("ncai.safety.permissionMode")),
                    Some(t!("ncai.safety.permissionMode.description")),
                    div().w(px(256.)).child(Select::new(&self.mode)),
                    cx,
                ),
                ui::setting_row(
                    Some(t!("ncai.safety.responseIdleTimeout")),
                    Some(t!("ncai.safety.responseIdleTimeout.description")),
                    number(
                        &self.response_idle,
                        Some("ncai.safety.responseIdleTimeout.unit"),
                    ),
                    cx,
                ),
                ui::setting_row(
                    Some(t!("ncai.safety.commandTimeout")),
                    Some(t!("ncai.safety.commandTimeout.description")),
                    number(
                        &self.command_timeout,
                        Some("ncai.safety.commandTimeout.unit"),
                    ),
                    cx,
                ),
                ui::setting_row(
                    Some(t!("ncai.safety.maxIterations")),
                    Some(t!("ncai.safety.maxIterations.description")),
                    number(&self.max_iterations, None),
                    cx,
                ),
            ],
            true,
            cx,
        );

        // blocklist
        let weak = cx.entity().downgrade();
        let rows: Vec<AnyElement> = self
            .blocklist
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let w = weak.clone();
                v_flex()
                    .id(("ai-blocklist-row", ix))
                    .test_support()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .child(div().flex_1().min_w_0().child(ui::mono_input(&row.input)))
                            .child(
                                Button::new(("ai-blocklist-remove", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::X))
                                    .on_click(move |_, window, cx| {
                                        let _ = w.update(cx, |this, cx| {
                                            this.remove_pattern(ix, window, cx)
                                        });
                                    }),
                            ),
                    )
                    .when_some(row.error.clone(), |d, e| {
                        d.child(
                            div()
                                .pl(px(4.))
                                .text_size(px(11.))
                                .text_color(theme.danger)
                                .child(e),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let blocklist = ui::padded_card(
            vec![
                h_flex()
                    .justify_between()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("ncai.safety.blocklist")),
                            )
                            .child(ui::muted_text(t!("ncai.safety.blocklist.description"), cx)),
                    )
                    .child(
                        Button::new("ai-blocklist-reset")
                            .ghost()
                            .small()
                            .label(t!("ncai.safety.blocklist.reset"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.reset_blocklist(window, cx)),
                            ),
                    )
                    .into_any_element(),
                v_flex().gap(px(6.)).children(rows).into_any_element(),
                h_flex()
                    .child(
                        Button::new("ai-blocklist-add")
                            .small()
                            .icon(Icon::new(IconName::Plus))
                            .label(t!("ncai.safety.blocklist.add"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_pattern(window, cx)),
                            ),
                    )
                    .into_any_element(),
            ],
            cx,
        );

        // grants
        let head = |label: String, w: Option<f32>| {
            div()
                .when_some(w, |d, w| d.w(px(w)))
                .when(w.is_none(), |d| d.flex_1())
                .px(px(8.))
                .py(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui_colors.text_muted)
                .child(label)
        };
        let grant_rows: Vec<AnyElement> = self
            .grants
            .iter()
            .enumerate()
            .map(|(ix, g)| {
                let rules = self.store.read(cx).prefs().permission_grants.clone();
                let rule = rules
                    .iter()
                    .find(|r| r.id == g.id)
                    .cloned()
                    .unwrap_or_default();
                let id = g.id.clone();
                let w = weak.clone();
                h_flex()
                    .id(("ai-grant-row", ix))
                    .test_support()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(ui_colors.border)
                    .child(
                        v_flex()
                            .w(px(220.))
                            .flex_none()
                            .child(Select::new(&g.capability))
                            .when(rule.session_pattern.starts_with("host:"), |d| {
                                d.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(ui_colors.text_muted)
                                        .truncate()
                                        .child(format!(
                                            "{}: {}",
                                            t!("ncai.chat.targetLabel"),
                                            rule.note.clone().unwrap_or_else(|| rule
                                                .session_pattern[5..]
                                                .to_string())
                                        )),
                                )
                            }),
                    )
                    .child(div().flex_1().min_w_0().child(
                        if safety::supports_command_pattern(&rule.capability_id) {
                            ui::mono_input(&g.command).into_any_element()
                        } else {
                            div()
                                .px(px(4.))
                                .text_size(px(12.))
                                .text_color(ui_colors.text_muted)
                                .child("—")
                                .into_any_element()
                        },
                    ))
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .child(ui::input(&g.note, 160.)),
                    )
                    .child(
                        Button::new(("ai-grant-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Trash))
                            .tooltip(t!("ncai.safety.grants.remove"))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                let _ = w.update(cx, |this, cx| this.remove_grant(&id, window, cx));
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        let grants_body: AnyElement = if self.grants.is_empty() {
            div()
                .py(px(24.))
                .flex()
                .justify_center()
                .rounded(px(10.))
                .border_1()
                .border_color(ui_colors.border)
                .text_size(px(12.))
                .text_color(ui_colors.text_muted)
                .child(t!("ncai.safety.grants.empty"))
                .into_any_element()
        } else {
            v_flex()
                .rounded(px(10.))
                .border_1()
                .border_color(ui_colors.border)
                .overflow_hidden()
                .child(
                    h_flex()
                        .bg(ui_colors.input_bg)
                        .child(head(t!("ncai.safety.grants.capability"), Some(232.)))
                        .child(head(t!("ncai.safety.grants.commandPattern"), None))
                        .child(head(t!("ncai.safety.grants.note"), Some(172.)))
                        .child(div().w(px(34.))),
                )
                .children(grant_rows)
                .into_any_element()
        };
        let grants =
            ui::padded_card(
                vec![
                    v_flex()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("ncai.safety.grants.heading")),
                        )
                        .child(ui::muted_text(t!("ncai.safety.grants.description"), cx))
                        .into_any_element(),
                    h_flex()
                        .gap(px(6.))
                        .flex_wrap()
                        .child(
                            Button::new("ai-grant-add")
                                .small()
                                .icon(Icon::new(IconName::Plus))
                                .label(t!("ncai.safety.grants.add"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_grant(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("ai-grant-export")
                                .small()
                                .icon(Icon::new(IconName::Download))
                                .label(t!("ncai.safety.grants.export"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.export_to_file(window, cx)
                                })),
                        )
                        .child(
                            Button::new("ai-grant-import")
                                .small()
                                .icon(Icon::new(IconName::Upload))
                                .label(t!("ncai.safety.grants.import"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.import_from_file(window, cx)
                                })),
                        )
                        .into_any_element(),
                    div()
                        .when_some(self.import_error.clone(), |d, e| {
                            d.text_size(px(11.)).text_color(theme.danger).child(e)
                        })
                        .into_any_element(),
                    grants_body,
                ],
                cx,
            );

        let _ = (
            MAX_COMMAND_TIMEOUT_SECONDS,
            MAX_RESPONSE_IDLE_TIMEOUT_SECONDS,
        );
        v_flex()
            .id("ai-safety-tab")
            .test_support()
            .gap(px(24.))
            .child(ui::section(
                "ai-safety",
                t!("ncai.safety.title"),
                None,
                vec![
                    settings.into_any_element(),
                    blocklist.into_any_element(),
                    ui::muted_text(t!("ncai.safety.note"), cx).into_any_element(),
                ],
                cx,
            ))
            .child(ui::section(
                "ai-safety-grants",
                t!("ncai.safety.grants.title"),
                None,
                vec![grants.into_any_element()],
                cx,
            ))
    }
}
