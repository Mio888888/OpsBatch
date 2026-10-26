//! External MCP card of the tool-access tab (`ExternalMcpCard` + `ExternalMcpHttpSection`): the switches and limits of
//! the bridge that lets an outside agent drive the app, and the client setup snippets.
//!
//! The settings are stored and the snippets generated here. The bridge itself (`crate::mcp_bridge`) serves the
//! Streamable HTTP transport; the stdio launcher is not part of this build, so there is no launcher path for the
//! stdio snippets.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::assets::IconName;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::prefs::external_mcp::{self as mcp, ApprovalAlert, Mode};
use ops_ai::prefs::PermissionMode;
use ops_i18n::t;

use super::ui::{self, CopyBlock};
use crate::ai::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Codex,
    Claude,
    Grok,
    Cursor,
    Other,
}

impl Client {
    const ALL: [Client; 5] = [Client::Codex, Client::Claude, Client::Grok, Client::Cursor, Client::Other];

    fn key(self) -> &'static str {
        match self {
            Client::Codex => "codex",
            Client::Claude => "claude",
            Client::Grok => "grok",
            Client::Cursor => "cursor",
            Client::Other => "other",
        }
    }
}

pub struct ExternalMcpCard {
    store: Entity<PrefsStore>,
    client: Client,
    mode: Entity<ChoiceSelect>,
    approval: Entity<ChoiceSelect>,
    idle_timeout: Entity<InputState>,
    session_idle_timeout: Entity<InputState>,
    port: Entity<InputState>,
    listen_address: Entity<InputState>,
    client_host: Entity<InputState>,
    /// Path of the stdio launcher the snippets point to; the bridge is not part of this build.
    launcher: Option<String>,
    discovery: Option<String>,
    http_errors: Vec<&'static str>,
    prompt: Entity<CopyBlock>,
    snippets: Vec<Entity<CopyBlock>>,
    http_blocks: Vec<Entity<CopyBlock>>,
    _subs: Vec<Subscription>,
}

fn mode_choices() -> Vec<Choice> {
    vec![Choice::new("persistent", t!("ncai.externalMcp.mode.persistent")), Choice::new("temporary", t!("ncai.externalMcp.mode.temporary"))]
}

fn alert_choices() -> Vec<Choice> {
    ApprovalAlert::ALL.iter().map(|a| Choice::new(a.as_str(), t!(&format!("ncai.externalMcp.approvalAlert.{}", a.as_str())))).collect()
}

impl ExternalMcpCard {
    pub fn new(store: Entity<PrefsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mode = cx.new(|cx| new_select(mode_choices(), "persistent", window, cx));
        let approval = cx.new(|cx| new_select(alert_choices(), "unfocused", window, cx));
        let idle_timeout = cx.new(|cx| InputState::new(window, cx));
        let session_idle_timeout = cx.new(|cx| InputState::new(window, cx));
        let port = cx.new(|cx| InputState::new(window, cx));
        let listen_address = cx.new(|cx| InputState::new(window, cx).placeholder(t!("ncai.externalMcp.http.listenAddress.placeholder")));
        let client_host = cx.new(|cx| InputState::new(window, cx).placeholder(t!("ncai.externalMcp.http.clientHost.placeholder")));
        let prompt = cx.new(|_| CopyBlock::new("ai-mcp-prompt", Some(t!("ncai.externalMcp.universalSetupPrompt")), String::new()));
        let snippets = (0..3).map(|i| cx.new(|_| CopyBlock::new(format!("ai-mcp-snippet-{i}"), None, String::new()))).collect();
        let http_blocks = (0..5).map(|i| cx.new(|_| CopyBlock::new(format!("ai-mcp-http-{i}"), None, String::new()))).collect();
        let subs = vec![
            cx.subscribe_in(&store, window, |this, _, ev: &PrefsEvent, window, cx| {
                if matches!(ev, PrefsEvent::Loaded) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&store, |this, _, cx| {
                this.refresh_blocks(cx);
                cx.notify();
            }),
            cx.observe(&crate::mcp_bridge::bridge(cx), |_, _, cx| cx.notify()),
            cx.subscribe_in(&mode, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.update(cx, |m| m.mode = if v == "temporary" { Mode::Temporary } else { Mode::Persistent });
                }
            }),
            cx.subscribe_in(&approval, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    let alert = ApprovalAlert::ALL.into_iter().find(|a| a.as_str() == v).unwrap_or_default();
                    this.update(cx, |m| m.approval_alert = alert);
                }
            }),
            ui::on_commit(&idle_timeout, window, cx, |this, text, window, cx| {
                if let Some(n) = ui::parse_int(&text) {
                    this.update(cx, |m| m.idle_timeout_minutes = n.clamp(1, 1440) as u32);
                }
                this.sync(window, cx);
            }),
            ui::on_commit(&session_idle_timeout, window, cx, |this, text, window, cx| {
                if let Some(n) = ui::parse_int(&text) {
                    this.update(cx, |m| m.session_idle_timeout_minutes = n.clamp(1, 1440) as u32);
                }
                this.sync(window, cx);
            }),
            ui::on_commit(&port, window, cx, |this, text, _, cx| this.commit_port(&text, cx)),
            ui::on_commit(&listen_address, window, cx, |this, text, _, cx| this.commit_listen_address(&text, cx)),
            ui::on_commit(&client_host, window, cx, |this, text, _, cx| this.commit_client_host(&text, cx)),
        ];
        let mut this = Self { store, client: Client::Codex, mode, approval, idle_timeout, session_idle_timeout, port, listen_address, client_host, launcher: None, discovery: None, http_errors: Vec::new(), prompt, snippets, http_blocks, _subs: subs };
        this.sync(window, cx);
        this
    }

    // ---- accessors for tests ----

    pub fn settings(&self, cx: &App) -> mcp::ExternalMcpSettings {
        self.store.read(cx).prefs().external_mcp.clone()
    }
    pub fn http_errors(&self) -> &[&'static str] {
        &self.http_errors
    }
    pub fn prompt_text(&self, cx: &App) -> String {
        self.prompt.read(cx).text().to_string()
    }
    pub fn http_texts(&self, cx: &App) -> Vec<String> {
        self.http_blocks.iter().map(|b| b.read(cx).text().to_string()).collect()
    }
    pub fn port(&self) -> &Entity<InputState> {
        &self.port
    }
    pub fn listen_address(&self) -> &Entity<InputState> {
        &self.listen_address
    }
    pub fn idle_timeout(&self) -> &Entity<InputState> {
        &self.idle_timeout
    }

    fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut mcp::ExternalMcpSettings)) {
        self.store.update(cx, |s, cx| s.update(cx, |p| f(&mut p.external_mcp)));
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.settings(cx);
        self.mode.update(cx, |sel, cx| sel.set_selected_value(&s.mode.as_str().to_string(), window, cx));
        self.approval.update(cx, |sel, cx| sel.set_selected_value(&s.approval_alert.as_str().to_string(), window, cx));
        ui::set_input(&self.idle_timeout, &s.idle_timeout_minutes.to_string(), window, cx);
        ui::set_input(&self.session_idle_timeout, &s.session_idle_timeout_minutes.to_string(), window, cx);
        ui::set_input(&self.port, &s.http.port.to_string(), window, cx);
        ui::set_input(&self.listen_address, &s.http.listen_address, window, cx);
        ui::set_input(&self.client_host, &s.http.client_host, window, cx);
        self.refresh_blocks(cx);
        cx.notify();
    }

    /// Rewrites the snippets for the selected client and the HTTP transport.
    fn refresh_blocks(&mut self, cx: &mut Context<Self>) {
        let s = self.settings(cx);
        let (launcher, discovery) = (self.launcher.clone(), self.discovery.clone());
        self.prompt.update(cx, |b, cx| b.set_text(mcp::onboarding_prompt(launcher.as_deref(), discovery.as_deref()), cx));
        let texts: Vec<(Option<String>, String)> = match (&launcher, self.client) {
            (None, _) => Vec::new(),
            (Some(l), Client::Codex) => vec![(Some(t!("ncai.externalMcp.cliCommand")), mcp::codex_add_command(l, discovery.as_deref())), (Some(t!("ncai.externalMcp.configSnippet")), mcp::toml_snippet(l, discovery.as_deref()))],
            (Some(l), Client::Claude) => vec![(Some(t!("ncai.externalMcp.cliCommand")), mcp::claude_add_command(l, discovery.as_deref())), (Some(t!("ncai.externalMcp.configSnippet")), mcp::json_snippet(l, discovery.as_deref()))],
            (Some(l), Client::Grok) => vec![(Some(t!("ncai.externalMcp.cliCommand")), mcp::grok_add_command(l, discovery.as_deref())), (Some(t!("ncai.externalMcp.configSnippet")), mcp::toml_snippet(l, discovery.as_deref()))],
            (Some(l), Client::Cursor | Client::Other) => vec![(Some(t!("ncai.externalMcp.configSnippet")), mcp::json_snippet(l, discovery.as_deref()))],
        };
        for (i, block) in self.snippets.clone().into_iter().enumerate() {
            let (label, text) = texts.get(i).cloned().unwrap_or((None, String::new()));
            block.update(cx, |b, cx| {
                b.set_label(label, cx);
                b.set_text(text, cx);
            });
        }
        let url = mcp::http_url(&s.http.listen_address, s.http.port, &s.http.client_host);
        let rows: Vec<(String, String)> = if url.is_empty() || s.http.token.is_empty() {
            Vec::new()
        } else {
            vec![
                (t!("ncai.externalMcp.http.url"), url.clone()),
                (t!("ncai.externalMcp.http.codexSnippet"), mcp::codex_http_toml(&url)),
                (t!("ncai.externalMcp.http.envLine"), mcp::codex_http_env_line(&s.http.token)),
                (t!("ncai.externalMcp.http.claudeCommand"), mcp::claude_http_add_command(&url, &s.http.token)),
                (t!("ncai.externalMcp.http.genericSnippet"), mcp::generic_http_json(&url, &s.http.token)),
            ]
        };
        for (i, block) in self.http_blocks.clone().into_iter().enumerate() {
            let (label, text) = rows.get(i).cloned().map(|(l, t)| (Some(l), t)).unwrap_or((None, String::new()));
            block.update(cx, |b, cx| {
                b.set_label(label, cx);
                b.set_text(text, cx);
            });
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.update(cx, |m| m.enabled = enabled);
    }

    pub fn set_http_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.update(cx, |m| {
            m.http.enabled = enabled;
            if enabled && m.http.token.is_empty() {
                m.http.token = mcp::generate_token();
            }
        });
    }

    pub fn regenerate_token(&mut self, cx: &mut Context<Self>) {
        self.update(cx, |m| m.http.token = mcp::generate_token());
    }

    fn commit_port(&mut self, text: &str, cx: &mut Context<Self>) {
        self.http_errors.retain(|e| *e != "port");
        match mcp::parse_port(text) {
            Some(p) => self.update(cx, |m| m.http.port = p),
            None => self.http_errors.push("port"),
        }
        cx.notify();
    }

    fn commit_listen_address(&mut self, text: &str, cx: &mut Context<Self>) {
        self.http_errors.retain(|e| *e != "address");
        let address = text.trim().to_string();
        if mcp::is_ip_address(&address) {
            self.update(cx, |m| m.http.listen_address = address);
        } else {
            self.http_errors.push("address");
        }
        cx.notify();
    }

    fn commit_client_host(&mut self, text: &str, cx: &mut Context<Self>) {
        self.http_errors.retain(|e| *e != "clientHost");
        let host = text.trim().to_string();
        if host.is_empty() || (mcp::is_ip_address(&host) && !mcp::is_wildcard_address(&host)) {
            self.update(cx, |m| m.http.client_host = host);
        } else {
            self.http_errors.push("clientHost");
        }
        cx.notify();
    }

    pub fn set_allow_custom_bind(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.update(cx, |m| m.http.allow_custom_bind = on);
        self.sync(window, cx);
    }
}

impl Render for ExternalMcpCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let s = self.settings(cx);
        let mode = self.store.read(cx).prefs().global_permission_mode;
        let weak = cx.entity().downgrade();

        let bridge_status = crate::mcp_bridge::bridge(cx).read(cx).status().clone();
        let (status_text, status_color) = match &bridge_status {
            _ if !s.enabled => (t!("ncai.externalMcp.status.disabled"), ui_colors.text_muted),
            crate::mcp_bridge::BridgeStatus::Running(_) => (t!("ncai.externalMcp.status.running"), ui::green()),
            crate::mcp_bridge::BridgeStatus::Starting => (t!("ncai.externalMcp.status.starting"), ui::amber()),
            crate::mcp_bridge::BridgeStatus::Error(_) => (t!("ncai.externalMcp.status.error"), theme.danger),
            _ => (t!("ncai.externalMcp.status.unavailable"), ui::amber()),
        };
        let bridge_note: Option<(String, Hsla)> = match (&bridge_status, s.enabled) {
            (_, false) => Some((t!("ncai.externalMcp.bridgeUnavailable"), ui_colors.text_muted)),
            (crate::mcp_bridge::BridgeStatus::NeedsHttp, _) => Some((t!("mcp.needHttp"), ui::amber())),
            (crate::mcp_bridge::BridgeStatus::Running(url), _) => Some((t!("mcp.listening", url = url.clone()), ui::green())),
            (crate::mcp_bridge::BridgeStatus::Error(e), _) => Some((match e.as_str() {
                "port_in_use" => t!("mcp.portInUse"),
                "invalid_address" => t!("mcp.invalidAddress"),
                other => other.to_string(),
            }, theme.danger)),
            _ => None,
        };
        let w = weak.clone();
        let enable_row = ui::setting_row(
            Some(t!("ncai.externalMcp.title")),
            None,
            h_flex().gap(px(12.)).items_center().child(div().id("ai-mcp-status").test_support().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(status_color).child(status_text)).child(ui::switch("ai-mcp-enabled", s.enabled, move |on, _, cx| {
                let _ = w.update(cx, |this, cx| this.set_enabled(on, cx));
            })),
            cx,
        );
        let w = weak.clone();
        let mut rows = vec![
            enable_row,
            ui::setting_row(Some(t!("ncai.externalMcp.mode")), Some(t!("ncai.externalMcp.mode.description")), div().w(px(160.)).child(Select::new(&self.mode)), cx),
        ];
        if s.mode == Mode::Temporary {
            rows.push(ui::setting_row(
                Some(t!("ncai.externalMcp.idleTimeout")),
                Some(t!("ncai.externalMcp.idleTimeout.description")),
                h_flex().gap(px(8.)).items_center().child(ui::input(&self.idle_timeout, 80.)).child(div().text_size(px(12.)).text_color(ui_colors.text_muted).child(t!("ncai.externalMcp.idleTimeout.minutes"))),
                cx,
            ));
        }
        rows.push(ui::setting_row(
            Some(t!("ncai.externalMcp.sessionIdleTimeout")),
            Some(t!("ncai.externalMcp.sessionIdleTimeout.description")),
            h_flex().gap(px(8.)).items_center().child(ui::input(&self.session_idle_timeout, 80.)).child(div().text_size(px(12.)).text_color(ui_colors.text_muted).child(t!("ncai.externalMcp.idleTimeout.minutes"))),
            cx,
        ));
        rows.push(ui::setting_row(Some(t!("ncai.externalMcp.silentSessions")), Some(t!("ncai.externalMcp.silentSessions.description")), ui::switch("ai-mcp-silent", s.silent_sessions, {
            let w = w.clone();
            move |on, _, cx| {
                let _ = w.update(cx, |this, cx| this.update(cx, |m| m.silent_sessions = on));
            }
        }), cx));
        rows.push(ui::setting_row(Some(t!("ncai.externalMcp.focusOnHostOpen")), Some(t!("ncai.externalMcp.focusOnHostOpen.description")), ui::switch("ai-mcp-focus", s.focus_on_host_open, {
            let w = w.clone();
            move |on, _, cx| {
                let _ = w.update(cx, |this, cx| this.update(cx, |m| m.focus_on_host_open = on));
            }
        }), cx));
        rows.push(ui::setting_row(Some(t!("ncai.externalMcp.approvalAlert")), Some(t!("ncai.externalMcp.approvalAlert.description")), div().w(px(160.)).child(Select::new(&self.approval)), cx));
        let mode_label = match mode {
            PermissionMode::Observer => t!("ncai.safety.permissionMode.observer"),
            PermissionMode::Confirm => t!("ncai.safety.permissionMode.confirm"),
            PermissionMode::Auto => t!("ncai.safety.permissionMode.auto"),
        };
        rows.push(
            v_flex()
                .gap(px(2.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(t!("ncai.externalMcp.permissionMode", mode = mode_label)))
                .child(ui::muted_text(t!("ncai.externalMcp.permissionMode.hint"), cx))
                .into_any_element(),
        );

        // client setup
        let client_tabs: Vec<(Client, String)> = Client::ALL.iter().map(|c| (*c, t!(&format!("ncai.externalMcp.client.{}", c.key())))).collect();
        let w2 = weak.clone();
        let tabs = ui::tab_bar("ai-mcp-clients", &client_tabs, self.client, move |c, _, cx| {
            let _ = w2.update(cx, |this, cx| {
                this.client = c;
                this.refresh_blocks(cx);
                cx.notify();
            });
        }, cx);
        let mut client_body: Vec<AnyElement> = Vec::new();
        if self.launcher.is_none() {
            client_body.push(ui::colored_text(t!("ncai.externalMcp.enableForLauncher"), ui::amber()).into_any_element());
        }
        client_body.extend(self.snippets.iter().filter(|b| !b.read(cx).text().is_empty()).map(|b| b.clone().into_any_element()));
        if matches!(self.client, Client::Other) || self.launcher.is_none() {
            client_body.push(self.prompt.clone().into_any_element());
        }
        let clients = ui::padded_card(
            vec![
                v_flex().child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(t!("ncai.externalMcp.clientConfiguration"))).child(ui::muted_text(t!("ncai.externalMcp.clientConfiguration.description"), cx)).into_any_element(),
                h_flex().child(tabs).into_any_element(),
                v_flex().gap(px(10.)).children(client_body).into_any_element(),
            ],
            cx,
        );

        // HTTP transport
        let http = &s.http;
        let url = mcp::http_url(&http.listen_address, http.port, &http.client_host);
        let wildcard = mcp::is_wildcard_address(&http.listen_address);
        let err = |k: &str, text: String| self.http_errors.contains(&k).then(|| ui::colored_text(text, theme.danger).into_any_element());
        let mut http_rows: Vec<AnyElement> = vec![ui::setting_row(
            Some(t!("ncai.externalMcp.http.enable")),
            Some(t!("ncai.externalMcp.http.description")),
            ui::switch("ai-mcp-http-enabled", http.enabled, {
                let w = weak.clone();
                move |on, _, cx| {
                    let _ = w.update(cx, |this, cx| this.set_http_enabled(on, cx));
                }
            }),
            cx,
        )];
        if http.enabled {
            http_rows.push(ui::setting_row(Some(t!("ncai.externalMcp.http.port")), Some(t!("ncai.externalMcp.http.portHint")), ui::input(&self.port, 100.), cx));
            http_rows.push(ui::setting_row(Some(t!("ncai.externalMcp.http.allowCustomBind")), Some(t!("ncai.externalMcp.http.allowCustomBind.description")), ui::switch("ai-mcp-http-bind", http.allow_custom_bind, {
                let w = weak.clone();
                move |on, window, cx| {
                    let _ = w.update(cx, |this, cx| this.set_allow_custom_bind(on, window, cx));
                }
            }), cx));
            if http.allow_custom_bind {
                http_rows.push(ui::setting_row(Some(t!("ncai.externalMcp.http.listenAddress")), None, ui::input(&self.listen_address, 200.), cx));
            }
            if http.allow_custom_bind && wildcard {
                http_rows.push(ui::setting_row(Some(t!("ncai.externalMcp.http.clientHost")), Some(t!("ncai.externalMcp.http.clientHost.description")), ui::input(&self.client_host, 200.), cx));
            }
            for (k, key) in [("port", "ncai.externalMcp.http.invalidPort"), ("address", "ncai.externalMcp.http.invalidAddress"), ("clientHost", "ncai.externalMcp.http.invalidClientHost")] {
                if let Some(e) = err(k, t!(key)) {
                    http_rows.push(e);
                }
            }
        }
        let mut http_card_children: Vec<AnyElement> = vec![ui::card(http_rows, true, cx).into_any_element()];
        if http.enabled {
            if !s.enabled {
                http_card_children.push(ui::colored_text(t!("ncai.externalMcp.http.requiresExternalMcp"), ui::amber()).into_any_element());
            }
            if wildcard && http.client_host.trim().is_empty() {
                http_card_children.push(ui::colored_text(t!("ncai.externalMcp.http.clientHostRequired"), ui::amber()).into_any_element());
            }
            if !url.is_empty() {
                http_card_children.extend(self.http_blocks.iter().filter(|b| !b.read(cx).text().is_empty()).map(|b| b.clone().into_any_element()));
                http_card_children.push(
                    h_flex()
                        .gap(px(10.))
                        .items_center()
                        .child(Button::new("ai-mcp-regenerate").small().icon(Icon::new(IconName::RefreshCw)).label(t!("ncai.externalMcp.http.regenerate")).on_click(cx.listener(|this, _, _, cx| this.regenerate_token(cx))))
                        .child(div().flex_1().text_size(px(11.)).text_color(ui_colors.text_muted).child(t!("ncai.externalMcp.http.regenerateHint")))
                        .into_any_element(),
                );
            }
            http_card_children.push(ui::muted_text(t!("ncai.externalMcp.http.legacySseNote"), cx).into_any_element());
            http_card_children.push(ui::muted_text(t!("ncai.externalMcp.http.wslHint"), cx).into_any_element());
            if http.allow_custom_bind && !mcp::is_loopback_or_default(&http.listen_address) {
                http_card_children.push(ui::colored_text(t!("ncai.externalMcp.http.warning"), ui::amber()).into_any_element());
            }
        }

        let usage = ["usage.keepRunning", "usage.localhost", "usage.permissions", "usage.capabilities"];
        v_flex()
            .id("ai-external-mcp")
            .test_support()
            .gap(px(12.))
            .child(ui::muted_text(t!("ncai.externalMcp.description"), cx))
            .child(ui::card(rows, true, cx))
            .children(bridge_note.map(|(text, color)| div().id("ai-mcp-note").test_support().child(ui::colored_text(text, color))))
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(ui_colors.text_secondary).child(t!("ncai.externalMcp.usage.title")))
                    .children(usage.iter().map(|k| ui::muted_text(t!(&format!("ncai.externalMcp.{k}")), cx))),
            )
            .child(clients)
            .child(v_flex().gap(px(8.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(t!("ncai.externalMcp.http.title"))).children(http_card_children))
    }
}
