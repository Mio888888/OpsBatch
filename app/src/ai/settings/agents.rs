//! Agents tab: the external agent CLIs (Codex, Claude Code, Copilot, Cursor, CodeBuddy, OpenCode, Grok, MiMo) and the
//! default agent (`SettingsAITab` agents section + the `*Card` components).

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::prefs::agents::{
    self, AgentPathInfo, CodexLogin, CodexState, CodexStatus, MANAGED_AGENTS,
};
use ops_ai::prefs::{CodebuddyOptions, ExternalAgentConfig};
use ops_i18n::t;

use super::ui;
use crate::ai::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

struct AgentRow {
    key: &'static str,
    path: Entity<InputState>,
    info: Option<AgentPathInfo>,
    resolving: bool,
}

pub struct AgentsTab {
    store: Entity<PrefsStore>,
    rows: Vec<AgentRow>,
    default_agent: Entity<ChoiceSelect>,
    discovered: bool,
    // Claude
    claude_open: bool,
    claude_dir: Entity<InputState>,
    claude_settings: Entity<InputState>,
    claude_env: Entity<TextareaState>,
    // CodeBuddy
    cb_open: bool,
    cb_advanced_open: bool,
    cb_internet: Entity<ChoiceSelect>,
    cb_env: Entity<TextareaState>,
    cb_effort: Entity<ChoiceSelect>,
    cb_max_turns: Entity<InputState>,
    cb_max_budget: Entity<InputState>,
    // Cursor
    cursor_key: Entity<InputState>,
    cursor_saved: bool,
    // Codex
    codex_status: Option<CodexStatus>,
    codex_checking: bool,
    codex_login: Option<CodexLogin>,
    codex_login_url: Option<String>,
    codex_error: Option<String>,
    _subs: Vec<Subscription>,
}

fn internet_choices() -> Vec<Choice> {
    vec![
        Choice::new("", t!("ncai.codebuddy.internetEnv.default")),
        Choice::new("internal", t!("ncai.codebuddy.internetEnv.internal")),
        Choice::new("ioa", t!("ncai.codebuddy.internetEnv.ioa")),
    ]
}

fn effort_choices() -> Vec<Choice> {
    let mut v = vec![Choice::new("", t!("ncai.codebuddy.effort.default"))];
    v.extend(
        ["minimal", "low", "medium", "high", "xhigh", "max"]
            .iter()
            .map(|e| Choice::new(*e, t!(&format!("ncai.codebuddy.effort.{e}")))),
    );
    v
}

fn claude_id() -> String {
    agents::managed_id("claude")
}

impl AgentsTab {
    pub fn new(store: Entity<PrefsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rows = MANAGED_AGENTS
            .iter()
            .map(|a| AgentRow {
                key: a.key,
                path: cx.new(|cx| InputState::new(window, cx)),
                info: None,
                resolving: false,
            })
            .collect::<Vec<_>>();
        let default_agent = cx.new(|cx| {
            new_select(
                vec![Choice::new("catty", t!("ncai.defaultAgent.catty"))],
                "catty",
                window,
                cx,
            )
        });
        let claude_dir = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.claude.configDir.placeholder"))
        });
        let claude_settings = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.claude.settings.placeholder"))
        });
        let claude_env = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("ncai.claude.envVars.placeholder"))
                .auto_grow(3, 8)
        });
        let cb_internet = cx.new(|cx| new_select(internet_choices(), "", window, cx));
        let cb_env = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("ncai.codebuddy.envVars.placeholder"))
                .auto_grow(3, 8)
        });
        let cb_effort = cx.new(|cx| new_select(effort_choices(), "", window, cx));
        let cb_max_turns = cx.new(|cx| InputState::new(window, cx));
        let cb_max_budget = cx.new(|cx| InputState::new(window, cx));
        let cursor_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ncai.cursor.apiKeyPlaceholder"))
                .masked(true)
        });

        let mut subs = vec![
            cx.subscribe_in(&store, window, |this, _, ev: &PrefsEvent, window, cx| {
                if matches!(ev, PrefsEvent::Loaded) {
                    this.sync(window, cx);
                    this.discover_all(window, cx);
                }
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&default_agent, window, |this, _, ev, _, cx| {
                if let Some(id) = confirmed_value(ev) {
                    this.store
                        .update(cx, |s, cx| s.update(cx, |p| p.default_agent_id = id));
                }
            }),
            cx.subscribe_in(&cb_effort, window, |this, _, ev, _, cx| {
                if let Some(effort) = confirmed_value(ev) {
                    this.update_codebuddy_options(cx, |o| {
                        o.effort = Some(effort).filter(|e| !e.is_empty())
                    });
                }
            }),
            cx.subscribe_in(&cb_internet, window, |this, _, ev, window, cx| {
                if confirmed_value(ev).is_some() {
                    this.commit_codebuddy_env(window, cx);
                }
            }),
            ui::on_commit(&claude_dir, window, cx, |this, _, window, cx| {
                this.commit_claude_env(window, cx)
            }),
            ui::on_commit(&claude_settings, window, cx, |this, _, window, cx| {
                this.commit_claude_env(window, cx)
            }),
            ui::on_commit(&cb_max_turns, window, cx, |this, text, _, cx| {
                let turns = ui::parse_int(&text).filter(|n| *n > 0).map(|n| n as u32);
                this.update_codebuddy_options(cx, |o| o.max_turns = turns);
            }),
            ui::on_commit(&cb_max_budget, window, cx, |this, text, _, cx| {
                let budget = text.trim().parse::<f64>().ok().filter(|n| *n > 0.);
                this.update_codebuddy_options(cx, |o| o.max_budget_usd = budget);
            }),
        ];
        // the env editors keep what is typed and store when focus leaves (a half-typed `KEY` without `=` is dropped on parse)
        subs.push(cx.subscribe_in(
            &claude_env,
            window,
            |this, _, ev: &gpui_kit::component::input::InputEvent, window, cx| {
                if matches!(ev, gpui_kit::component::input::InputEvent::Blur) {
                    this.commit_claude_env(window, cx);
                }
            },
        ));
        subs.push(cx.subscribe_in(
            &cb_env,
            window,
            |this, _, ev: &gpui_kit::component::input::InputEvent, window, cx| {
                if matches!(ev, gpui_kit::component::input::InputEvent::Blur) {
                    this.commit_codebuddy_env(window, cx);
                }
            },
        ));
        let mut this = Self {
            store,
            rows,
            default_agent,
            discovered: false,
            claude_open: false,
            claude_dir,
            claude_settings,
            claude_env,
            cb_open: false,
            cb_advanced_open: false,
            cb_internet,
            cb_env,
            cb_effort,
            cb_max_turns,
            cb_max_budget,
            cursor_key,
            cursor_saved: false,
            codex_status: None,
            codex_checking: false,
            codex_login: None,
            codex_login_url: None,
            codex_error: None,
            _subs: subs,
        };
        if this.store.read(cx).loaded() {
            this.sync(window, cx);
            this.discover_all(window, cx);
        }
        this
    }

    // ---- accessors for tests ----

    pub fn path_input(&self, key: &str) -> Option<Entity<InputState>> {
        self.rows
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.path.clone())
    }
    pub fn path_info(&self, key: &str) -> Option<AgentPathInfo> {
        self.rows
            .iter()
            .find(|r| r.key == key)
            .and_then(|r| r.info.clone())
    }
    pub fn resolving(&self) -> bool {
        self.rows.iter().any(|r| r.resolving)
    }
    pub fn cursor_key(&self) -> &Entity<InputState> {
        &self.cursor_key
    }
    pub fn claude_dir(&self) -> &Entity<InputState> {
        &self.claude_dir
    }
    pub fn claude_env(&self) -> &Entity<TextareaState> {
        &self.claude_env
    }
    pub fn codex_status(&self) -> Option<&CodexStatus> {
        self.codex_status.as_ref()
    }

    fn agent(&self, key: &str, cx: &App) -> Option<ExternalAgentConfig> {
        let id = agents::managed_id(key);
        self.store
            .read(cx)
            .prefs()
            .external_agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }

    /// Pushes the stored values into the fields.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.rows.len() {
            let key = self.rows[ix].key;
            let stored = self
                .agent(key, cx)
                .filter(|a| {
                    a.command_source.as_deref() == Some("manual")
                        && agents::is_path_like(&a.command)
                })
                .map(|a| a.command)
                .unwrap_or_default();
            ui::set_input(&self.rows[ix].path, &stored, window, cx);
        }
        if let Some(a) = self.agent("claude", cx) {
            let (dir, settings, env) = agents::split_claude_env(&a.env);
            ui::set_input(&self.claude_dir, &dir, window, cx);
            ui::set_input(&self.claude_settings, &settings, window, cx);
            ui::set_textarea(&self.claude_env, &env, window, cx);
            self.claude_open =
                self.claude_open || !(dir.is_empty() && settings.is_empty() && env.is_empty());
        }
        if let Some(a) = self.agent("codebuddy", cx) {
            let (internet, env) = agents::split_codebuddy_env(&a.env);
            self.cb_internet
                .update(cx, |s, cx| s.set_selected_value(&internet, window, cx));
            ui::set_textarea(&self.cb_env, &env, window, cx);
            let o = a.codebuddy_options.clone().unwrap_or_default();
            self.cb_effort.update(cx, |s, cx| {
                s.set_selected_value(&o.effort.clone().unwrap_or_default(), window, cx)
            });
            ui::set_input(
                &self.cb_max_turns,
                &o.max_turns.map(|n| n.to_string()).unwrap_or_default(),
                window,
                cx,
            );
            ui::set_input(
                &self.cb_max_budget,
                &o.max_budget_usd.map(|n| n.to_string()).unwrap_or_default(),
                window,
                cx,
            );
            self.cb_open = self.cb_open || !(internet.is_empty() && env.is_empty());
            self.cb_advanced_open = self.cb_advanced_open || !o.is_empty();
        }
        self.refresh_default_choices(window, cx);
        cx.notify();
    }

    fn refresh_default_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = self.store.read(cx).prefs().clone();
        let mut options = agents::agent_options(&prefs);
        options[0].1 = t!("ncai.defaultAgent.catty");
        let choices: Vec<Choice> = options
            .into_iter()
            .map(|(id, name)| Choice::new(id, name))
            .collect();
        let selected = prefs.default_agent_id.clone();
        self.default_agent.update(cx, |s, cx| {
            s.set_items(choices, window, cx);
            s.set_selected_value(&selected, window, cx);
        });
    }

    // ---- discovery ----

    fn discover_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.discovered {
            return;
        }
        self.discovered = true;
        for key in MANAGED_AGENTS.iter().map(|a| a.key) {
            let custom = self
                .path_input(key)
                .map(|i| i.read(cx).value().to_string())
                .unwrap_or_default();
            self.resolve(key, custom, window, cx);
        }
    }

    /// Looks the agent binary up (a typed path first, then `PATH`) and folds the result into the stored agents.
    pub fn resolve(
        &mut self,
        key: &'static str,
        custom_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter_mut().find(|r| r.key == key) else {
            return;
        };
        row.resolving = true;
        cx.notify();
        let manual = !custom_path.trim().is_empty();
        cx.spawn_in(window, async move |this, cx| {
            let info =
                ops_core::rt::run(async move { agents::resolve_cli(key, &custom_path).await })
                    .await
                    .unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(row) = this.rows.iter_mut().find(|r| r.key == key) {
                    row.resolving = false;
                    row.info = Some(info.clone());
                }
                this.apply_probe(
                    key,
                    info,
                    if manual { "manual" } else { "auto" },
                    window,
                    cx,
                );
                if key == "codex" {
                    this.refresh_codex(window, cx);
                }
            });
        })
        .detach();
    }

    fn apply_probe(
        &mut self,
        key: &'static str,
        info: AgentPathInfo,
        source: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.store.read(cx).prefs().clone();
        let mut after = before.clone();
        agents::apply_probe(&mut after, key, &info, source);
        if after.external_agents != before.external_agents
            || after.default_agent_id != before.default_agent_id
        {
            self.store.update(cx, |s, cx| {
                s.update(cx, |p| {
                    p.external_agents = after.external_agents;
                    p.default_agent_id = after.default_agent_id;
                })
            });
        }
        self.refresh_default_choices(window, cx);
        cx.notify();
    }

    pub fn check_path(&mut self, key: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let custom = self
            .path_input(key)
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default();
        self.resolve(key, custom, window, cx);
    }

    pub fn reset_path(&mut self, key: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.path_input(key) {
            ui::set_input(&input, "", window, cx);
        }
        self.resolve(key, String::new(), window, cx);
    }

    // ---- Claude ----

    fn commit_claude_env(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.claude_dir.read(cx).value().to_string();
        let settings = self.claude_settings.read(cx).value().to_string();
        let env_text = self.claude_env.read(cx).value().to_string();
        let id = claude_id();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if p.agent_mut(&id).is_none() {
                    // configuration typed before the CLI was found is kept on a placeholder entry
                    if dir.trim().is_empty()
                        && settings.trim().is_empty()
                        && env_text.trim().is_empty()
                    {
                        return;
                    }
                    p.external_agents.push(ExternalAgentConfig {
                        id: id.clone(),
                        name: "Claude Code".into(),
                        command: "claude".into(),
                        icon: Some("claude".into()),
                        sdk_backend: Some("claude".into()),
                        enabled: false,
                        available: Some(false),
                        ..Default::default()
                    });
                }
                if let Some(a) = p.agent_mut(&id) {
                    a.env = agents::build_claude_env(&a.env, &dir, &settings, &env_text);
                }
            })
        });
    }

    // ---- CodeBuddy ----

    fn ensure_codebuddy(p: &mut ops_ai::prefs::AiPrefs) {
        let id = agents::managed_id("codebuddy");
        if p.agent_mut(&id).is_none() {
            p.external_agents.push(ExternalAgentConfig {
                id,
                name: "CodeBuddy Code".into(),
                command: "codebuddy".into(),
                icon: Some("codebuddy".into()),
                sdk_backend: Some("codebuddy".into()),
                enabled: false,
                available: Some(false),
                ..Default::default()
            });
        }
    }

    fn commit_codebuddy_env(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let internet = self
            .cb_internet
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let env_text = self.cb_env.read(cx).value().to_string();
        let id = agents::managed_id("codebuddy");
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if internet.is_empty() && env_text.trim().is_empty() && p.agent_mut(&id).is_none() {
                    return;
                }
                Self::ensure_codebuddy(p);
                if let Some(a) = p.agent_mut(&id) {
                    a.env = agents::build_codebuddy_env(&a.env, &internet, &env_text);
                }
            })
        });
    }

    pub fn update_codebuddy_options(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CodebuddyOptions),
    ) {
        let id = agents::managed_id("codebuddy");
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                Self::ensure_codebuddy(p);
                if let Some(a) = p.agent_mut(&id) {
                    let mut o = a.codebuddy_options.take().unwrap_or_default();
                    f(&mut o);
                    a.codebuddy_options = Some(o).filter(|o| !o.is_empty());
                }
            })
        });
    }

    // ---- Cursor ----

    pub fn set_cursor_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        let mode = mode.to_string();
        let id = agents::managed_id("cursor");
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if let Some(a) = p.agent_mut(&id) {
                    a.cursor_auth_mode = Some(mode);
                }
            })
        });
    }

    pub fn save_cursor_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key =
            Some(self.cursor_key.read(cx).value().trim().to_string()).filter(|k| !k.is_empty());
        let id = agents::managed_id("cursor");
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if p.agent_mut(&id).is_none() {
                    let d = agents::managed_agent("cursor").unwrap();
                    p.external_agents.push(ExternalAgentConfig {
                        id: id.clone(),
                        name: d.name.into(),
                        command: d.commands[0].into(),
                        icon: Some(d.icon.into()),
                        sdk_backend: Some("cursor".into()),
                        cursor_auth_mode: Some("api-key".into()),
                        enabled: true,
                        available: Some(false),
                        ..Default::default()
                    });
                }
                if let Some(a) = p.agent_mut(&id) {
                    a.api_key = key;
                    a.available = Some(
                        a.api_key.is_some() || a.cursor_auth_mode.as_deref() == Some("cli-login"),
                    );
                }
            })
        });
        self.cursor_saved = true;
        self.refresh_default_choices(window, cx);
        cx.notify();
    }

    // ---- Codex ----

    fn codex_binary(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.path_info("codex")
            .and_then(|i| i.path)
            .or_else(|| self.agent("codex", cx).map(|a| a.command))
            .map(std::path::PathBuf::from)
    }

    pub fn refresh_codex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binary) = self
            .codex_binary(cx)
            .filter(|_| self.path_info("codex").is_some_and(|i| i.available))
        else {
            self.codex_status = None;
            cx.notify();
            return;
        };
        self.codex_checking = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let status =
                ops_core::rt::run(async move { agents::codex_login_status(&binary).await })
                    .await
                    .ok();
            let _ = this.update(cx, |this, cx| {
                this.codex_checking = false;
                this.codex_status = status;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn start_codex_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binary) = self.codex_binary(cx) else {
            return;
        };
        self.codex_error = None;
        self.codex_checking = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { agents::codex_login_start(&binary).await })
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.codex_checking = false;
                match result {
                    Ok(login) => {
                        this.codex_login_url = login.url.clone();
                        if let Some(url) = &login.url {
                            cx.open_url(url);
                        }
                        this.codex_login = Some(login);
                    }
                    Err(e) => this.codex_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn cancel_codex_login(&mut self, cx: &mut Context<Self>) {
        self.codex_login_url = None;
        if let Some(login) = self.codex_login.take() {
            cx.spawn(async move |_, _| {
                let _ = ops_core::rt::run(async move { login.cancel().await }).await;
            })
            .detach();
        }
        cx.notify();
    }

    pub fn codex_logout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binary) = self.codex_binary(cx) else {
            return;
        };
        self.codex_checking = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let status = ops_core::rt::run(async move { agents::codex_logout(&binary).await })
                .await
                .ok();
            let _ = this.update(cx, |this, cx| {
                this.codex_checking = false;
                this.codex_status = status;
                cx.notify();
            });
        })
        .detach();
    }

    fn set_runtime(&mut self, key: &str, value: &'static str, cx: &mut Context<Self>) {
        let id = agents::managed_id(key);
        let key = key.to_string();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                if let Some(a) = p.agent_mut(&id) {
                    match key.as_str() {
                        "codex" => a.codex_runtime = Some(value.to_string()),
                        _ => a.grok_runtime = Some(value.to_string()),
                    }
                }
            })
        });
    }

    // ---- rendering helpers ----

    fn status_line(&self, prefix: &str, row: &AgentRow, cx: &App) -> (String, Hsla) {
        let ui_colors = crate::theme::ui(cx);
        if row.resolving {
            (
                t!(&format!("ncai.{prefix}.detecting")),
                ui_colors.text_muted,
            )
        } else if row.info.as_ref().is_some_and(|i| i.available) {
            // Codex reports its login state instead of a plain "detected"
            (
                if prefix == "codex" {
                    t!("ncai.codex.statusUnknown")
                } else {
                    t!(&format!("ncai.{prefix}.detected"))
                },
                ui::green(),
            )
        } else {
            (t!(&format!("ncai.{prefix}.notFound")), ui::amber())
        }
    }

    /// Description + status, the detected path and the "custom path" row shared by every agent card.
    fn path_block(
        &self,
        prefix: &'static str,
        key: &'static str,
        status_override: Option<(String, Hsla)>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let ui_colors = crate::theme::ui(cx);
        let Some(row) = self.rows.iter().find(|r| r.key == key) else {
            return Vec::new();
        };
        let found = row.info.as_ref().is_some_and(|i| i.available);
        let (status, color) = status_override.unwrap_or_else(|| self.status_line(prefix, row, cx));
        let mut out: Vec<AnyElement> = vec![h_flex()
            .gap(px(16.))
            .items_start()
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .line_height(relative(1.5))
                    .text_color(ui_colors.text_muted)
                    .child(t!(&format!("ncai.{prefix}.description"))),
            )
            .child(
                div()
                    .id(SharedString::from(format!("ai-agent-status-{}", key)))
                    .test_support()
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .child(status),
            )
            .into_any_element()];
        if let Some(info) = row.info.as_ref().filter(|i| i.available) {
            out.push(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .min_w_0()
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .text_color(ui_colors.text_muted)
                            .child(t!(&format!("ncai.{prefix}.path"))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("ai-agent-path-{}", key)))
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family("Menlo")
                            .child(info.path.clone().unwrap_or_default()),
                    )
                    .when_some(info.version.clone(), |d, v| {
                        d.child(div().text_color(ui_colors.text_muted).child("|"))
                            .child(div().flex_none().text_color(ui_colors.text_muted).child(v))
                    })
                    .into_any_element(),
            );
        }
        if !row.resolving {
            let weak = cx.entity().downgrade();
            let (w1, w2) = (weak.clone(), weak);
            let has_custom = !row.path.read(cx).value().trim().is_empty();
            if !found {
                out.push(
                    ui::colored_text(t!(&format!("ncai.{prefix}.notFoundHint")), ui::amber())
                        .into_any_element(),
                );
            }
            out.push(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family("Menlo")
                            .child(Input::new(&row.path).small()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-agent-check-{}", key)))
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label(t!(&format!("ncai.{prefix}.check")))
                            .disabled(!has_custom)
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| this.check_path(key, window, cx));
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-agent-reset-{}", key)))
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::RotateCcw))
                            .label(t!(&format!("ncai.{prefix}.resetPath")))
                            .disabled(!has_custom)
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| this.reset_path(key, window, cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    fn agent_section(
        &self,
        key: &'static str,
        title: String,
        body: Vec<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        let ui_colors = crate::theme::ui(cx);
        let d = agents::managed_agent(key).unwrap();
        let badge = div()
            .size(px(20.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .bg(ui_colors.text_muted.opacity(0.14))
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .text_color(ui_colors.text_secondary)
            .child(d.name.chars().next().unwrap_or('?').to_string());
        v_flex()
            .id(SharedString::from(format!("ai-agent-section-{}", key)))
            .test_support()
            .gap(px(10.))
            .child(
                h_flex().gap(px(8.)).items_center().child(badge).child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ui_colors.text)
                        .child(title),
                ),
            )
            .child(ui::list_card(false, cx).children(body))
            .into_any_element()
    }

    fn switch_block(
        &self,
        id: &'static str,
        title: String,
        desc: String,
        checked: bool,
        badge: Option<(String, Hsla)>,
        on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let ui_colors = crate::theme::ui(cx);
        h_flex()
            .gap(px(16.))
            .items_start()
            .justify_between()
            .pt(px(10.))
            .border_t_1()
            .border_color(ui_colors.border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .when_some(badge, |d, (text, c)| d.child(ui::pill(text, c))),
                    )
                    .child(ui::muted_text(desc, cx)),
            )
            .child(ui::switch(id, checked, on_change))
            .into_any_element()
    }
}

impl Render for AgentsTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let ui_colors = crate::theme::ui(cx);
        let weak = cx.entity().downgrade();
        let mut sections: Vec<AnyElement> = Vec::new();

        // ---- Codex ----
        {
            let mut body = self.path_block("codex", "codex", None, cx);
            let found = self.path_info("codex").is_some_and(|i| i.available);
            let status = self.codex_status.clone();
            let line = if self.codex_checking {
                Some((t!("ncai.codex.detecting"), ui_colors.text_muted))
            } else if !found && self.path_info("codex").is_some() {
                Some((t!("ncai.codex.notFound"), ui::amber()))
            } else if self.codex_login.is_some() {
                Some((t!("ncai.codex.awaitingLogin"), ui::amber()))
            } else {
                status.as_ref().map(|s| match s.state {
                    CodexState::ConnectedChatGpt => {
                        (t!("ncai.codex.connectedChatGPT"), ui::green())
                    }
                    CodexState::ConnectedApiKey => (t!("ncai.codex.connectedApiKey"), ui::green()),
                    CodexState::ConnectedCustomConfig
                        if s.custom_config.as_ref().is_some_and(|c| {
                            c.env_key.is_some() && !c.env_key_present && !c.has_hardcoded_api_key
                        }) =>
                    {
                        (t!("ncai.codex.customConfigIncomplete"), ui::amber())
                    }
                    CodexState::ConnectedCustomConfig => {
                        (t!("ncai.codex.connectedCustomConfig"), ui::green())
                    }
                    CodexState::NotLoggedIn => {
                        (t!("ncai.codex.notConnected"), ui_colors.text_muted)
                    }
                    CodexState::Unknown => (t!("ncai.codex.statusUnknown"), ui_colors.text_muted),
                })
            };
            if let Some(line) = line {
                // the generic description row already shows a status; replace it with the login state
                body = self.path_block("codex", "codex", Some(line), cx);
            }
            if found {
                let w = weak.clone();
                let app_server = self
                    .agent("codex", cx)
                    .and_then(|a| a.codex_runtime)
                    .as_deref()
                    == Some("app-server");
                body.push(self.switch_block(
                    "ai-codex-appserver",
                    t!("ncai.codex.appServer.title"),
                    t!("ncai.codex.appServer.description"),
                    app_server,
                    Some((t!("ncai.codex.appServer.experimental"), ui::amber())),
                    move |on, _, cx| {
                        let _ = w.update(cx, |this, cx| {
                            this.set_runtime("codex", if on { "app-server" } else { "sdk" }, cx)
                        });
                    },
                    cx,
                ));
                let connected = status.as_ref().is_some_and(|s| s.is_connected());
                let busy = self.codex_checking;
                let login_running = self.codex_login.is_some();
                let mut buttons = h_flex().gap(px(8.)).flex_wrap();
                if login_running {
                    buttons = buttons
                        .child(
                            Button::new("ai-codex-open-login")
                                .small()
                                .primary()
                                .icon(Icon::new(IconName::ExternalLink))
                                .label(t!("ncai.codex.openLogin"))
                                .disabled(self.codex_login_url.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(url) = this.codex_login_url.clone() {
                                        cx.open_url(&url);
                                    }
                                })),
                        )
                        .child(
                            Button::new("ai-codex-cancel")
                                .small()
                                .label(t!("common.cancel"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_codex_login(cx)),
                                ),
                        );
                } else if connected
                    && status.as_ref().is_some_and(|s| {
                        matches!(
                            s.state,
                            CodexState::ConnectedChatGpt | CodexState::ConnectedApiKey
                        )
                    })
                {
                    buttons = buttons.child(
                        Button::new("ai-codex-logout")
                            .small()
                            .icon(Icon::new(IconName::LogOut))
                            .label(t!("ncai.codex.logout"))
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.codex_logout(window, cx)),
                            ),
                    );
                } else if !connected {
                    buttons = buttons.child(
                        Button::new("ai-codex-connect")
                            .small()
                            .primary()
                            .label(t!("ncai.codex.connectChatGPT"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_codex_login(window, cx)
                            })),
                    );
                }
                buttons = buttons.child(
                    Button::new("ai-codex-refresh")
                        .small()
                        .icon(Icon::new(IconName::RefreshCw))
                        .label(t!("ncai.codex.refreshStatus"))
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.refresh_codex(window, cx)),
                        ),
                );
                body.push(buttons.into_any_element());
                if let Some(custom) = status.as_ref().and_then(|s| s.custom_config.as_ref()) {
                    body.push(
                        ui::muted_text(
                            t!(
                                "ncai.codex.customConfigHint",
                                provider = custom.display_name.clone()
                            ),
                            cx,
                        )
                        .into_any_element(),
                    );
                    if let Some(env_key) = custom
                        .env_key
                        .clone()
                        .filter(|_| !custom.env_key_present && !custom.has_hardcoded_api_key)
                    {
                        body.push(
                            ui::colored_text(
                                t!("ncai.codex.customConfigMissingEnvKey", envKey = env_key),
                                ui::amber(),
                            )
                            .into_any_element(),
                        );
                    }
                }
                if let Some(e) = self.codex_error.clone() {
                    body.push(ui::colored_text(e, cx.theme().danger).into_any_element());
                }
            }
            sections.push(self.agent_section("codex", t!("ncai.codex"), body, cx));
        }

        // ---- Claude Code ----
        {
            let mut body = self.path_block("claude", "claude", None, cx);
            let open = self.claude_open;
            body.push(
                v_flex()
                    .gap(px(10.))
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(ui_colors.border)
                    .child(
                        h_flex()
                            .id("ai-claude-config-toggle")
                            .test_support()
                            .justify_between()
                            .items_center()
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ui_colors.text_muted)
                                    .child(t!("ncai.claude.configSection")),
                            )
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronUp
                                } else {
                                    IconName::ChevronDown
                                })
                                .size_3p5()
                                .text_color(ui_colors.text_muted),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.claude_open = !this.claude_open;
                                cx.notify();
                            })),
                    )
                    .when(open, |d| {
                        d.child(ui::stacked(
                            t!("ncai.claude.configDir"),
                            Some(t!("ncai.claude.configDir.hint")),
                            ui::mono_input(&self.claude_dir),
                            cx,
                        ))
                        .child(ui::stacked(
                            t!("ncai.claude.settings"),
                            Some(t!("ncai.claude.settings.hint")),
                            ui::mono_input(&self.claude_settings),
                            cx,
                        ))
                        .child(ui::stacked(
                            t!("ncai.claude.envVars"),
                            Some(t!("ncai.claude.envVars.hint")),
                            ui::textarea(&self.claude_env),
                            cx,
                        ))
                    })
                    .into_any_element(),
            );
            sections.push(self.agent_section("claude", t!("ncai.claude.title"), body, cx));
        }

        // ---- Copilot ----
        sections.push(self.agent_section(
            "copilot",
            t!("ncai.copilot.title"),
            self.path_block("copilot", "copilot", None, cx),
            cx,
        ));

        // ---- Cursor ----
        {
            let agent = self.agent("cursor", cx);
            let mode = agent
                .as_ref()
                .and_then(|a| a.cursor_auth_mode.clone())
                .unwrap_or_else(|| "api-key".into());
            let cli_mode = mode == "cli-login";
            let has_key = agent
                .as_ref()
                .is_some_and(|a| a.api_key.as_deref().is_some_and(|k| !k.is_empty()));
            let found = self.path_info("cursor").is_some_and(|i| i.available);
            let status = if self.rows.iter().any(|r| r.key == "cursor" && r.resolving) {
                (t!("ncai.cursor.detecting"), ui_colors.text_muted)
            } else if found || agent.as_ref().is_some_and(|a| a.available == Some(true)) || has_key
            {
                (t!("ncai.cursor.detected"), ui::green())
            } else {
                (t!("ncai.cursor.notFound"), ui::amber())
            };
            let mut body: Vec<AnyElement> = Vec::new();
            body.push(
                h_flex()
                    .gap(px(16.))
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(ui_colors.text_muted)
                            .child(t!("ncai.cursor.description")),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("ai-agent-status-{}", "cursor")))
                            .test_support()
                            .flex_none()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(status.1)
                            .child(status.0),
                    )
                    .into_any_element(),
            );
            let (w1, w2) = (weak.clone(), weak.clone());
            body.push(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("ai-cursor-mode-cli")
                            .small()
                            .when(cli_mode, |b| b.primary())
                            .label(t!("ncai.cursor.modeCli"))
                            .on_click(move |_, _, cx| {
                                let _ =
                                    w1.update(cx, |this, cx| this.set_cursor_mode("cli-login", cx));
                            }),
                    )
                    .child(
                        Button::new("ai-cursor-mode-key")
                            .small()
                            .when(!cli_mode, |b| b.primary())
                            .label(t!("ncai.cursor.modeApiKey"))
                            .on_click(move |_, _, cx| {
                                let _ =
                                    w2.update(cx, |this, cx| this.set_cursor_mode("api-key", cx));
                            }),
                    )
                    .into_any_element(),
            );
            body.push(
                ui::muted_text(
                    if cli_mode {
                        t!("ncai.cursor.modeCliHint")
                    } else {
                        t!("ncai.cursor.modeApiKeyHint")
                    },
                    cx,
                )
                .into_any_element(),
            );
            if cli_mode {
                body.push(ui::muted_text(t!("ncai.cursor.cliLoginHint"), cx).into_any_element());
            } else {
                body.push(
                    ui::stacked(
                        t!("ncai.cursor.apiKey"),
                        None,
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.cursor_key).mask_toggle()),
                            )
                            .child(
                                Button::new("ai-cursor-save")
                                    .small()
                                    .label(if self.cursor_saved {
                                        t!("ncai.cursor.saved")
                                    } else {
                                        t!("ncai.cursor.saveApiKey")
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_cursor_key(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("ai-cursor-check")
                                    .small()
                                    .icon(Icon::new(IconName::RefreshCw))
                                    .label(t!("ncai.cursor.check"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.check_path("cursor", window, cx)
                                    })),
                            ),
                        cx,
                    )
                    .into_any_element(),
                );
                body.push(
                    ui::colored_text(
                        if has_key {
                            t!("ncai.cursor.apiKeyConfigured")
                        } else {
                            t!("ncai.cursor.apiKeyMissing")
                        },
                        if has_key {
                            ui::green()
                        } else {
                            ui_colors.text_muted
                        },
                    )
                    .into_any_element(),
                );
            }
            sections.push(self.agent_section("cursor", t!("ncai.cursor.title"), body, cx));
        }

        // ---- CodeBuddy ----
        {
            let mut body = self.path_block("codebuddy", "codebuddy", None, cx);
            let agent = self.agent("codebuddy", cx);
            let options = agent
                .as_ref()
                .and_then(|a| a.codebuddy_options.clone())
                .unwrap_or_default();
            let persistence_supported = agent
                .as_ref()
                .map(|a| agents::codebuddy_persistence_supported(a.cli_version.as_deref()))
                .unwrap_or(false)
                || self
                    .path_info("codebuddy")
                    .and_then(|i| i.version)
                    .is_some_and(|v| agents::codebuddy_persistence_supported(Some(&v)));
            let (open, adv_open) = (self.cb_open, self.cb_advanced_open);
            let toggle = |id: &'static str, label: String, is_open: bool| {
                h_flex()
                    .id(id)
                    .test_support()
                    .justify_between()
                    .items_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ui_colors.text_muted)
                            .child(label),
                    )
                    .child(
                        Icon::new(if is_open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .size_3p5()
                        .text_color(ui_colors.text_muted),
                    )
            };
            let w = weak.clone();
            let flag = move |id: &'static str,
                             title: String,
                             hint: String,
                             checked: bool,
                             disabled: bool,
                             apply: fn(&mut CodebuddyOptions, bool)| {
                let w = w.clone();
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(ui_colors.text_muted)
                                    .child(hint),
                            ),
                    )
                    .child(
                        Switch::new(id)
                            .checked(checked)
                            .disabled(disabled)
                            .on_click(move |on, _, cx| {
                                let on = *on;
                                let _ = w.update(cx, |this, cx| {
                                    this.update_codebuddy_options(cx, |o| apply(o, on))
                                });
                            }),
                    )
            };
            body.push(
                v_flex()
                    .gap(px(10.))
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(ui_colors.border)
                    .child(
                        toggle(
                            "ai-cb-config-toggle",
                            t!("ncai.codebuddy.configSection"),
                            open,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cb_open = !this.cb_open;
                            cx.notify();
                        })),
                    )
                    .when(open, |d| {
                        d.child(ui::stacked(
                            t!("ncai.codebuddy.internetEnv"),
                            Some(t!("ncai.codebuddy.internetEnv.hint")),
                            div().w(px(220.)).child(Select::new(&self.cb_internet)),
                            cx,
                        ))
                        .child(ui::stacked(
                            t!("ncai.codebuddy.envVars"),
                            Some(t!("ncai.codebuddy.envVars.hint")),
                            ui::textarea(&self.cb_env),
                            cx,
                        ))
                    })
                    .child(
                        toggle(
                            "ai-cb-advanced-toggle",
                            t!("ncai.codebuddy.advancedSection"),
                            adv_open,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cb_advanced_open = !this.cb_advanced_open;
                            cx.notify();
                        })),
                    )
                    .when(adv_open, |d| {
                        d.child(ui::stacked(
                            t!("ncai.codebuddy.effort"),
                            Some(t!("ncai.codebuddy.effort.hint")),
                            div().w(px(220.)).child(Select::new(&self.cb_effort)),
                            cx,
                        ))
                        .child(ui::stacked(
                            t!("ncai.codebuddy.maxTurns"),
                            Some(t!("ncai.codebuddy.maxTurns.hint")),
                            ui::input(&self.cb_max_turns, 120.),
                            cx,
                        ))
                        .child(ui::stacked(
                            t!("ncai.codebuddy.maxBudget"),
                            Some(t!("ncai.codebuddy.maxBudget.hint")),
                            ui::input(&self.cb_max_budget, 120.),
                            cx,
                        ))
                        .child(flag(
                            "ai-cb-sandbox",
                            t!("ncai.codebuddy.sandbox"),
                            t!("ncai.codebuddy.sandbox.hint"),
                            options.sandbox_enabled.unwrap_or(false),
                            false,
                            |o, on| o.sandbox_enabled = Some(on),
                        ))
                        .child(flag(
                            "ai-cb-persist",
                            t!("ncai.codebuddy.persistSession"),
                            t!("ncai.codebuddy.persistSession.hint"),
                            options.persist_session.unwrap_or(true),
                            !persistence_supported,
                            |o, on| o.persist_session = Some(on),
                        ))
                        .child(flag(
                            "ai-cb-checkpoint",
                            t!("ncai.codebuddy.fileCheckpointing"),
                            t!("ncai.codebuddy.fileCheckpointing.hint"),
                            options.enable_file_checkpointing.unwrap_or(false),
                            false,
                            |o, on| o.enable_file_checkpointing = Some(on),
                        ))
                    })
                    .into_any_element(),
            );
            sections.push(self.agent_section("codebuddy", t!("ncai.codebuddy.title"), body, cx));
        }

        // ---- OpenCode / Grok / MiMo ----
        sections.push(self.agent_section(
            "opencode",
            t!("ncai.opencode.title"),
            self.path_block("opencode", "opencode", None, cx),
            cx,
        ));
        {
            let mut body = self.path_block("grok", "grok", None, cx);
            if self.path_info("grok").is_some_and(|i| i.available) {
                let acp = self
                    .agent("grok", cx)
                    .and_then(|a| a.grok_runtime)
                    .as_deref()
                    != Some("streaming-json");
                let w = weak.clone();
                let mut block = vec![self.switch_block(
                    "ai-grok-acp",
                    t!("ncai.grok.runtime.acp.title"),
                    t!("ncai.grok.runtime.acp.description"),
                    acp,
                    Some((t!("ncai.grok.runtime.acp.default"), ui_colors.primary)),
                    move |on, _, cx| {
                        let _ = w.update(cx, |this, cx| {
                            this.set_runtime("grok", if on { "acp" } else { "streaming-json" }, cx)
                        });
                    },
                    cx,
                )];
                if !acp {
                    block.push(
                        ui::muted_text(t!("ncai.grok.runtime.streamingJson.hint"), cx)
                            .into_any_element(),
                    );
                }
                body.extend(block);
            }
            sections.push(self.agent_section("grok", t!("ncai.grok.title"), body, cx));
        }
        sections.push(self.agent_section(
            "mimo",
            t!("ncai.mimo.title"),
            self.path_block("mimo", "mimo", None, cx),
            cx,
        ));

        // ---- default agent ----
        let options = agents::agent_options(self.store.read(cx).prefs());
        if options.len() > 1 {
            sections.push(
                ui::section(
                    "ai-default-agent",
                    t!("ncai.defaultAgent"),
                    None,
                    vec![ui::card(
                        vec![ui::setting_row(
                            None,
                            Some(t!("ncai.defaultAgent.description")),
                            div().w(px(256.)).child(Select::new(&self.default_agent)),
                            cx,
                        )],
                        false,
                        cx,
                    )
                    .into_any_element()],
                    cx,
                )
                .into_any_element(),
            );
        }

        v_flex()
            .id("ai-agents-tab")
            .test_support()
            .gap(px(24.))
            .children(sections)
    }
}
