//! Tool access tab (`SettingsAITab` "tools" section): chat shortcuts, MCP / skills mode, privacy, the external MCP
//! bridge, user skills and quick messages.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::TextareaState;
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::prefs::external_mcp;
use ops_ai::prefs::privacy;
use ops_ai::prefs::user_skills::{self, SkillsStatus};
use ops_ai::prefs::ToolIntegrationMode;
use ops_i18n::t;

use super::mcp::ExternalMcpCard;
use super::quick_messages::QuickMessagesSection;
use super::ui::{self, CopyBlock};
use crate::ai::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

pub struct ToolsTab {
    store: Entity<PrefsStore>,
    mode: Entity<ChoiceSelect>,
    guidance: Entity<CopyBlock>,
    // privacy draft (applied with the button)
    privacy_enabled: bool,
    privacy_private_ips: bool,
    cidrs: Entity<TextareaState>,
    patterns: Entity<TextareaState>,
    privacy_applied: bool,
    privacy_error: Option<String>,
    // skills
    skills: Option<Result<SkillsStatus, String>>,
    skills_loading: bool,
    mcp: Entity<ExternalMcpCard>,
    quick: Entity<QuickMessagesSection>,
    _subs: Vec<Subscription>,
}

fn mode_choices() -> Vec<Choice> {
    vec![
        Choice::new("mcp", t!("ncai.toolAccess.mode.mcp")),
        Choice::new("skills", t!("ncai.toolAccess.mode.skills")),
    ]
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

impl ToolsTab {
    pub fn new(store: Entity<PrefsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mode = cx.new(|cx| new_select(mode_choices(), "mcp", window, cx));
        let guidance = cx.new(|_| {
            CopyBlock::new(
                "ai-tool-guidance",
                Some(t!("ncai.toolAccess.mcpPrompt.title")),
                String::new(),
            )
        });
        let cidrs = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("192.0.2.0/24\n2001:db8::/32")
                .auto_grow(3, 8)
        });
        let patterns = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("INTERNAL_[A-Z0-9]+")
                .auto_grow(3, 8)
        });
        let mcp = cx.new(|cx| ExternalMcpCard::new(store.clone(), window, cx));
        let quick = cx.new(|cx| QuickMessagesSection::new(store.clone(), window, cx));
        let subs = vec![
            cx.subscribe_in(&store, window, |this, _, ev: &PrefsEvent, window, cx| {
                if matches!(ev, PrefsEvent::Loaded) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&store, |this, _, cx| {
                this.refresh_guidance(cx);
                cx.notify();
            }),
            cx.subscribe_in(&mode, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    let mode = ToolIntegrationMode::parse(&v);
                    this.store
                        .update(cx, |s, cx| s.update(cx, |p| p.tool_integration_mode = mode));
                }
            }),
        ];
        let mut this = Self {
            store,
            mode,
            guidance,
            privacy_enabled: false,
            privacy_private_ips: false,
            cidrs,
            patterns,
            privacy_applied: false,
            privacy_error: None,
            skills: None,
            skills_loading: false,
            mcp,
            quick,
            _subs: subs,
        };
        this.sync(window, cx);
        this.reload_skills(cx);
        this
    }

    // ---- accessors for tests ----

    pub fn mcp(&self) -> &Entity<ExternalMcpCard> {
        &self.mcp
    }
    pub fn quick(&self) -> &Entity<QuickMessagesSection> {
        &self.quick
    }
    pub fn cidrs(&self) -> &Entity<TextareaState> {
        &self.cidrs
    }
    pub fn patterns(&self) -> &Entity<TextareaState> {
        &self.patterns
    }
    pub fn privacy_error(&self) -> Option<&str> {
        self.privacy_error.as_deref()
    }
    pub fn privacy_applied(&self) -> bool {
        self.privacy_applied
    }
    pub fn skills(&self) -> Option<&Result<SkillsStatus, String>> {
        self.skills.as_ref()
    }
    pub fn skills_loading(&self) -> bool {
        self.skills_loading
    }
    pub fn guidance_text(&self, cx: &App) -> String {
        self.guidance.read(cx).text().to_string()
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = self.store.read(cx).prefs().clone();
        self.mode.update(cx, |s, cx| {
            s.set_selected_value(
                &prefs.tool_integration_mode.as_str().to_string(),
                window,
                cx,
            )
        });
        self.privacy_enabled = prefs.privacy.enabled;
        self.privacy_private_ips = prefs.privacy.redact_private_ips;
        ui::set_textarea(&self.cidrs, &prefs.privacy.cidrs.join("\n"), window, cx);
        ui::set_textarea(
            &self.patterns,
            &prefs.privacy.patterns.join("\n"),
            window,
            cx,
        );
        self.refresh_guidance(cx);
        cx.notify();
    }

    fn refresh_guidance(&mut self, cx: &mut Context<Self>) {
        let mode = self.store.read(cx).prefs().tool_integration_mode;
        let (label, text) = match mode {
            ToolIntegrationMode::Mcp => (
                t!("ncai.toolAccess.mcpPrompt.title"),
                external_mcp::onboarding_prompt(None, None),
            ),
            // the bundled skill file and CLI of Netcatty are not part of this build
            ToolIntegrationMode::Skills => (t!("ncai.toolAccess.skills.file"), String::new()),
        };
        self.guidance.update(cx, |g, cx| {
            g.set_label(Some(label), cx);
            g.set_text(text, cx);
        });
    }

    pub fn set_show_selection_action(&mut self, on: bool, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.show_terminal_selection_ai_action = on)
        });
    }

    // ---- privacy ----

    /// Validates and stores the privacy draft (the "apply" button).
    pub fn apply_privacy(&mut self, cx: &mut Context<Self>) -> bool {
        self.privacy_applied = false;
        self.privacy_error = None;
        let cidrs = lines(self.cidrs.read(cx).value().as_ref());
        let patterns: Vec<String> = self
            .patterns
            .read(cx)
            .value()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        if let Err(e) =
            privacy::validate_cidrs(&cidrs).and_then(|_| privacy::validate_patterns(&patterns))
        {
            self.privacy_error = Some(e);
            cx.notify();
            return false;
        }
        let (enabled, private_ips) = (self.privacy_enabled, self.privacy_private_ips);
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                p.privacy = ops_ai::prefs::PrivacySettings {
                    enabled,
                    redact_private_ips: private_ips,
                    cidrs,
                    patterns,
                }
            })
        });
        self.privacy_applied = true;
        cx.notify();
        true
    }

    // ---- skills ----

    pub fn reload_skills(&mut self, cx: &mut Context<Self>) {
        self.skills_loading = true;
        cx.notify();
        let dir = crate::services::ctx(cx).core.data_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { user_skills::scan(&dir) })
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.skills_loading = false;
                if let Ok(status) = &result {
                    let slugs = user_skills::reserved_slugs(status);
                    this.quick.update(cx, |q, cx| q.set_reserved(slugs, cx));
                }
                this.skills = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_skills_folder(&mut self, cx: &mut Context<Self>) {
        let dir = crate::services::ctx(cx).core.data_dir.clone();
        if let Ok(path) = user_skills::ensure_dir(&dir) {
            let _ = ui::open_folder(&path);
        }
        self.reload_skills(cx);
    }
}

impl Render for ToolsTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let prefs = self.store.read(cx).prefs().clone();
        let weak = cx.entity().downgrade();

        // chat shortcuts
        let w = weak.clone();
        let shortcuts = ui::section(
            "ai-chat-shortcuts",
            t!("ncai.chatShortcuts.title"),
            None,
            vec![ui::card(
                vec![ui::setting_row(
                    Some(t!("ncai.chatShortcuts.selectionAction")),
                    Some(t!("ncai.chatShortcuts.selectionAction.description")),
                    ui::switch(
                        "ai-selection-action",
                        prefs.show_terminal_selection_ai_action,
                        move |on, _, cx| {
                            let _ = w.update(cx, |this, cx| this.set_show_selection_action(on, cx));
                        },
                    ),
                    cx,
                )],
                true,
                cx,
            )
            .into_any_element()],
            cx,
        );

        // tool access mode + guidance
        let skills_mode = prefs.tool_integration_mode == ToolIntegrationMode::Skills;
        let access = ui::section(
            "ai-tool-access-mode",
            t!("ncai.toolAccess.title"),
            None,
            vec![ui::padded_card(
                vec![
                    ui::setting_row(
                        None,
                        Some(t!("ncai.toolAccess.description")),
                        div().w(px(192.)).child(Select::new(&self.mode)),
                        cx,
                    ),
                    v_flex()
                        .gap(px(8.))
                        .child(ui::muted_text(
                            if skills_mode {
                                t!("ncai.toolAccess.skills.description")
                            } else {
                                t!("ncai.toolAccess.mcpPrompt.description")
                            },
                            cx,
                        ))
                        .child(self.guidance.clone())
                        .child(ui::colored_text(
                            if skills_mode {
                                t!("ncai.toolAccess.skills.unavailable")
                            } else {
                                t!("ncai.toolAccess.mcpPrompt.enableHint")
                            },
                            ui::amber(),
                        ))
                        .into_any_element(),
                ],
                cx,
            )
            .into_any_element()],
            cx,
        );

        // privacy
        let (w1, w2) = (weak.clone(), weak.clone());
        let applied = self.privacy_applied;
        let privacy_section = ui::section(
            "ai-privacy",
            t!("ncai.privacy.title"),
            None,
            vec![
                ui::muted_text(t!("ncai.privacy.description"), cx).into_any_element(),
                ui::muted_text(t!("ncai.privacy.boundary"), cx).into_any_element(),
                ui::card(
                    vec![
                        ui::setting_row(
                            Some(t!("ncai.privacy.enable")),
                            None,
                            ui::switch(
                                "ai-privacy-enabled",
                                self.privacy_enabled,
                                move |on, _, cx| {
                                    let _ = w1.update(cx, |this, cx| {
                                        this.privacy_enabled = on;
                                        this.privacy_applied = false;
                                        cx.notify();
                                    });
                                },
                            ),
                            cx,
                        ),
                        ui::setting_row(
                            Some(t!("ncai.privacy.privateIps")),
                            Some(t!("ncai.privacy.privateIps.description")),
                            ui::switch(
                                "ai-privacy-private-ips",
                                self.privacy_private_ips,
                                move |on, _, cx| {
                                    let _ = w2.update(cx, |this, cx| {
                                        this.privacy_private_ips = on;
                                        this.privacy_applied = false;
                                        cx.notify();
                                    });
                                },
                            ),
                            cx,
                        ),
                    ],
                    true,
                    cx,
                )
                .into_any_element(),
                ui::padded_card(
                    vec![
                        ui::stacked(
                            t!("ncai.privacy.cidrs"),
                            Some(t!("ncai.privacy.cidrs.description")),
                            ui::textarea(&self.cidrs),
                            cx,
                        )
                        .into_any_element(),
                        ui::stacked(
                            t!("ncai.privacy.patterns"),
                            Some(t!("ncai.privacy.patterns.description")),
                            ui::textarea(&self.patterns),
                            cx,
                        )
                        .into_any_element(),
                        ui::muted_text(t!("ncai.privacy.connectionScope"), cx).into_any_element(),
                        h_flex()
                            .gap(px(12.))
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.))
                                    .text_color(ui::amber())
                                    .child(t!("ncai.privacy.reconnect")),
                            )
                            .child(
                                Button::new("ai-privacy-apply")
                                    .small()
                                    .label(t!("ncai.privacy.apply"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.apply_privacy(cx);
                                    })),
                            )
                            .into_any_element(),
                        div()
                            .id("ai-privacy-result")
                            .test_support()
                            .when_some(self.privacy_error.clone(), |d, e| {
                                d.child(ui::colored_text(
                                    format!("{} {}", t!("ncai.privacy.failed"), e),
                                    theme.danger,
                                ))
                            })
                            .when(self.privacy_error.is_none() && applied, |d| {
                                d.child(ui::colored_text(t!("ncai.privacy.applied"), ui::green()))
                            })
                            .into_any_element(),
                    ],
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        );

        // external MCP
        let mcp = ui::section(
            "ai-external-mcp-section",
            t!("ncai.externalMcp.title"),
            None,
            vec![self.mcp.clone().into_any_element()],
            cx,
        );

        // user skills
        let loading = self.skills_loading;
        let skills_actions = h_flex()
            .gap(px(8.))
            .child(
                Button::new("ai-skills-reload")
                    .small()
                    .icon(Icon::new(IconName::RefreshCw))
                    .label(t!("ncai.userSkills.reload"))
                    .disabled(loading)
                    .on_click(cx.listener(|this, _, _, cx| this.reload_skills(cx))),
            )
            .child(
                Button::new("ai-skills-open")
                    .small()
                    .icon(Icon::new(IconName::FolderOpen))
                    .label(t!("ncai.userSkills.openFolder"))
                    .disabled(loading)
                    .on_click(cx.listener(|this, _, _, cx| this.open_skills_folder(cx))),
            );
        let mut skill_children: Vec<AnyElement> =
            vec![ui::muted_text(t!("ncai.userSkills.description"), cx).into_any_element()];
        match &self.skills {
            _ if loading => skill_children
                .push(ui::muted_text(t!("ncai.userSkills.loading"), cx).into_any_element()),
            Some(Ok(status)) => {
                skill_children.push(
                    ui::muted_text(
                        format!(
                            "{}: {}",
                            t!("ncai.userSkills.location"),
                            status.directory.display()
                        ),
                        cx,
                    )
                    .into_any_element(),
                );
                skill_children.push(
                    ui::muted_text(
                        t!(
                            "ncai.userSkills.summary",
                            ready = status.ready_count,
                            warnings = status.warning_count
                        ),
                        cx,
                    )
                    .into_any_element(),
                );
                if status.skills.is_empty() {
                    skill_children
                        .push(ui::muted_text(t!("ncai.userSkills.empty"), cx).into_any_element());
                }
                skill_children.extend(status.skills.iter().enumerate().map(|(ix, s)| {
                    v_flex()
                        .id(("ai-skill-row", ix))
                        .test_support()
                        .gap(px(4.))
                        .pt(px(8.))
                        .border_t_1()
                        .border_color(ui_colors.border)
                        .child(
                            h_flex()
                                .justify_between()
                                .gap(px(12.))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(s.name.clone()),
                                        )
                                        .child(ui::muted_text(s.description.clone(), cx))
                                        .child(
                                            div()
                                                .font_family("Menlo")
                                                .text_size(px(11.))
                                                .text_color(ui_colors.text_muted)
                                                .child(s.directory_name.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(if s.ready { ui::green() } else { ui::amber() })
                                        .child(if s.ready {
                                            t!("ncai.userSkills.status.ready")
                                        } else {
                                            t!("ncai.userSkills.status.warning")
                                        }),
                                ),
                        )
                        .children(
                            s.warnings
                                .iter()
                                .map(|w| ui::colored_text(format!("⚠ {w}"), ui::amber())),
                        )
                        .into_any_element()
                }));
            }
            Some(Err(e)) => {
                skill_children.push(ui::colored_text(e.clone(), theme.danger).into_any_element())
            }
            None => skill_children
                .push(ui::muted_text(t!("ncai.userSkills.unavailable"), cx).into_any_element()),
        }
        let skills = ui::section(
            "ai-user-skills",
            t!("ncai.userSkills.title"),
            Some(skills_actions.into_any_element()),
            vec![ui::padded_card(skill_children, cx).into_any_element()],
            cx,
        );

        v_flex()
            .id("ai-tools-tab")
            .test_support()
            .gap(px(24.))
            .child(shortcuts)
            .child(access)
            .child(privacy_section)
            .child(mcp)
            .child(skills)
            .child(self.quick.clone())
    }
}
