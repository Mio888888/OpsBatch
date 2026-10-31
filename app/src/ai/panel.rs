//! The "AI" tab of the terminal bottom panel (port of `AiChatPanel.tsx`): chat on the left, command approval on
//! the right. Approved commands run in the live terminal and their result is sent back to the model.

use std::collections::HashSet;
use std::time::Duration;

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::client::{AiActionDecision, AiActionRiskLevel};
use ops_ai::follow_up::{
    classify_terminal_follow_up, format_terminal_command_result, TerminalCommandResult,
};
use ops_ai::parser::{
    extract_empty_command_plan_notice, strip_command_plan_block, ActionKind, ParsedPendingAction,
};
use ops_i18n::t;

use super::prompt;
use super::store::{ai_store, AiChatStore, AiScope, ChatMsg, Role};
use crate::terminal::executions::ExecStatus;
use crate::terminal::pane::TerminalPane;
use ops_ai::prefs::quick::{self, SlashItem, SlashKind};
use ops_ai::prefs::user_skills::{self, UserSkill};

/// A terminal command started from the AI may run this long before it is reported as timed out.

/// What the panel is currently attached to: the active terminal tab.
#[derive(Clone)]
pub struct AiTarget {
    pub session_id: String,
    pub scope: AiScope,
    /// `(name, ip)` of the connected host (none for a local shell).
    pub host: Option<(String, String)>,
    /// Name of the terminal tab (the original shows it next to the toolbar, also for the local shell).
    pub label: String,
    pub pane: WeakEntity<TerminalPane>,
}

pub struct AiChatPanel {
    store: Entity<AiChatStore>,
    target: Option<AiTarget>,
    input: Entity<TextareaState>,
    show_history: bool,
    executing: HashSet<String>,
    scroll: ScrollHandle,
    /// User skills offered by the `/` list (read when the list opens).
    skills: Vec<UserSkill>,
    _subs: Vec<Subscription>,
}

fn exec_status(s: ExecStatus) -> &'static str {
    match s {
        ExecStatus::Completed => "completed",
        ExecStatus::Closed => "closed",
        ExecStatus::Timeout => "timeout",
        ExecStatus::WriteFailed => "write_failed",
    }
}

impl AiChatPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ai_store(cx);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("ai.inputPlaceholder"))
                .auto_grow(1, 5)
                .submit_on_enter(true)
        });
        let subs = vec![
            cx.observe(&store, |this, _, cx| {
                this.scroll.scroll_to_bottom();
                this.auto_approve(cx);
                cx.notify();
            }),
            cx.observe(&input, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &input,
                window,
                |this, input, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        // the `/` list shows the skills as they are on disk right now
                        if text == "/" {
                            this.reload_skills(cx);
                        }
                        if let Some(t) = this.target.as_ref() {
                            let sid = t.session_id.clone();
                            this.store
                                .update(cx, |s, cx| s.set_input_text(&sid, text, cx));
                        }
                    }
                    InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            store,
            target: None,
            input,
            show_history: false,
            executing: HashSet::new(),
            scroll: ScrollHandle::new(),
            skills: Vec::new(),
            _subs: subs,
        }
    }

    pub fn target(&self) -> Option<&AiTarget> {
        self.target.as_ref()
    }

    /// Attach to a terminal tab (idempotent for the same tab).
    pub fn set_target(
        &mut self,
        target: Option<AiTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same = match (&self.target, &target) {
            (Some(a), Some(b)) => a.session_id == b.session_id && a.scope == b.scope,
            (None, None) => true,
            _ => false,
        };
        if let Some(t) = &target {
            self.target = Some(t.clone());
        } else {
            self.target = None;
        }
        if same {
            return;
        }
        self.show_history = false;
        let Some(t) = target else {
            cx.notify();
            return;
        };
        let saved = self.store.update(cx, |s, cx| {
            s.activate_session(&t.session_id, t.scope.clone(), cx);
            s.load_conversations(&t.session_id, cx);
            s.session(&t.session_id)
                .map(|s| s.input_text.clone())
                .unwrap_or_default()
        });
        self.input
            .update(cx, |i, cx| i.set_value(saved, window, cx));
        cx.notify();
    }

    /// "AI analyze error" on a terminal selection: starts a turn in the active session right away.
    pub fn analyze_selection(&mut self, selection: &str, cx: &mut Context<Self>) {
        let Some(t) = self.target.clone() else { return };
        let host = t.host.as_ref().map(|(n, i)| (n.as_str(), i.as_str()));
        let (context, message) = prompt::analyze_selection(selection, host);
        self.store.update(cx, |s, cx| {
            s.activate_session(&t.session_id, t.scope.clone(), cx);
            s.reset_follow_ups(&t.session_id);
            s.send_direct_message(&t.session_id, message, Some(context), Vec::new(), cx);
        });
    }

    fn context(&self, cx: &App) -> String {
        let Some(t) = &self.target else {
            return String::new();
        };
        let buffer = t
            .pane
            .upgrade()
            .map(|p| p.read(cx).buffer_text())
            .unwrap_or_default();
        prompt::build_context(
            t.host.as_ref().map(|(n, i)| (n.as_str(), i.as_str())),
            None,
            &buffer,
        )
    }

    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.is_none() {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        // while the `/` list is open Enter picks its entry instead of sending
        if let Some(query) = quick::slash_query(&text) {
            let items = self.slash_items(query, cx);
            if let Some(item) = items
                .iter()
                .find(|i| i.slug == query)
                .or(items.first())
                .cloned()
            {
                self.apply_slash(item, window, cx);
                return;
            }
        }
        if let Some((slug, rest)) = quick::parse_slash_command(&text) {
            if slug == "search" && crate::ai::runtime::limits(cx).web_search_ready {
                self.send_with_search(rest.to_string(), window, cx);
                return;
            }
            if self.skills.is_empty() {
                self.reload_skills(cx);
            }
            if let Some(skill) = self
                .skills
                .iter()
                .find(|s| s.ready && s.slug == slug)
                .cloned()
            {
                let message = if rest.is_empty() {
                    t!("ai.useSkill", name = skill.name.clone())
                } else {
                    rest.to_string()
                };
                self.dispatch(
                    message,
                    Some(format!("[用户技能：{}]\n{}", skill.name, skill.body)),
                    window,
                    cx,
                );
                return;
            }
        }
        self.dispatch(text, None, window, cx);
    }

    /// Sends `text` (with optional extra context in front of the terminal context) and clears the input.
    fn dispatch(
        &mut self,
        text: String,
        extra: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut context = self.context(cx);
        if let Some(extra) = extra {
            context = format!("{extra}\n\n{context}");
        }
        let sid = self
            .target
            .as_ref()
            .map(|t| t.session_id.clone())
            .unwrap_or_default();
        let sent = self.store.update(cx, |s, cx| {
            s.set_input_text(&sid, text, cx);
            s.send_message(&sid, Some(context), Vec::new(), cx)
        });
        if sent {
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
        }
    }

    /// `/search <query>`: look the query up, then ask with the results as context.
    fn send_with_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        if query.is_empty() {
            return;
        }
        let ctx = crate::services::ctx(cx);
        let q = query.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move {
                let prefs = ops_ai::prefs::load_prefs(&ctx.db)?;
                let config = prefs
                    .web_search
                    .ok_or_else(|| "网页搜索尚未配置".to_string())?;
                ops_ai::prefs::search::web_search(&config, &q).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(rows) => {
                    let context = quick::format_search_context(&query, &rows);
                    this.dispatch(query, Some(context), window, cx);
                }
                Err(e) => {
                    crate::widgets::toast::error(window, cx, t!("ai.searchFailed", error = e))
                }
            });
        })
        .detach();
    }

    fn reload_skills(&mut self, cx: &mut Context<Self>) {
        let dir = crate::services::ctx(cx).core.data_dir.clone();
        self.skills = user_skills::scan(&dir)
            .map(|s| s.skills.into_iter().filter(|k| k.ready).collect())
            .unwrap_or_default();
    }

    /// The entries of the `/` list for what is typed after the slash.
    pub fn slash_items(&self, query: &str, cx: &App) -> Vec<SlashItem> {
        let limits = crate::ai::runtime::limits(cx);
        let skills: Vec<quick::SkillEntry> = self
            .skills
            .iter()
            .map(|s| (s.slug.clone(), s.name.clone(), s.description.clone()))
            .collect();
        let streaming = self
            .target
            .as_ref()
            .and_then(|t| self.store.read(cx).session(&t.session_id))
            .is_some_and(|s| s.streaming());
        quick::build_slash_items(
            &limits.quick_messages,
            &skills,
            query,
            limits.web_search_ready,
            streaming,
        )
    }

    /// Picks an entry of the `/` list.
    pub fn apply_slash(&mut self, item: SlashItem, window: &mut Window, cx: &mut Context<Self>) {
        let value = match &item.kind {
            SlashKind::Quick(m) => m.content.clone(),
            SlashKind::Skill => format!("/{} ", item.slug),
            SlashKind::Search => "/search ".to_string(),
            SlashKind::Stop => {
                let conversation = self
                    .target
                    .as_ref()
                    .and_then(|t| self.store.read(cx).session(&t.session_id))
                    .and_then(|s| s.active_conversation_id.clone());
                if let Some(id) = conversation {
                    let _ = ops_ai::client::ai_chat_cancel(id);
                }
                String::new()
            }
        };
        self.input.update(cx, |i, cx| {
            i.set_value(value, window, cx);
            i.focus(window, cx);
        });
        cx.notify();
    }

    /// "Auto" permission mode (and commands a permission grant covers) run without a click; blocked commands never do.
    fn auto_approve(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.as_ref() else {
            return;
        };
        let auto =
            crate::ai::runtime::limits(cx).permission_mode == ops_ai::prefs::PermissionMode::Auto;
        let ready: Vec<String> = self
            .store
            .read(cx)
            .session(&target.session_id)
            .map(|s| {
                s.pending_actions()
                    .into_iter()
                    .filter(|a| {
                        a.kind == ActionKind::Command
                            && !a.approved
                            && !a.executed
                            && !a.rejected
                            && !a.assessment_loading
                    })
                    .filter(|a| {
                        a.assessment.as_ref().is_some_and(|x| {
                            x.decision != AiActionDecision::Block
                                && (auto || x.matched_rule == "PERMISSION_GRANT")
                        })
                    })
                    .filter(|a| !self.executing.contains(&a.id))
                    .map(|a| a.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in ready {
            self.approve(&id, cx);
        }
    }

    /// Approve one pending command: run it in the terminal, wait for the result, then ask the model to follow up.
    pub fn approve(&mut self, action_id: &str, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        if self.executing.contains(action_id) {
            return;
        }
        let Some(action) = self
            .store
            .read(cx)
            .session(&target.session_id)
            .and_then(|s| {
                s.pending_actions()
                    .into_iter()
                    .find(|a| a.id == action_id)
                    .cloned()
            })
        else {
            return;
        };
        if action.assessment_loading {
            return;
        }
        let sid = target.session_id.clone();
        let approved = self.store.update(cx, |s, cx| {
            s.begin_approval(&sid, action_id, Some(&sid), cx)
        });
        let Some(action) = approved else { return };
        let Some(command) = action.command.clone() else {
            return;
        };
        let Some(pane) = target.pane.upgrade() else {
            return;
        };

        self.executing.insert(action.id.clone());
        let follow_up_context = self.context(cx);
        let limits = crate::ai::runtime::limits(cx);
        let rx = pane.update(cx, |p, cx| {
            p.execute_command(
                &command,
                Some(Duration::from_secs(limits.command_timeout as u64)),
                cx,
            )
        });
        cx.notify();
        let action_id = action.id.clone();
        cx.spawn(async move |this, cx| {
            let result = rx.await.ok();
            let _ = this.update(cx, |this, cx| {
                this.executing.remove(&action_id);
                let Some(result) = result else {
                    cx.notify();
                    return;
                };
                let formatted = format_terminal_command_result(&TerminalCommandResult {
                    status: exec_status(result.status).into(),
                    output: result.output,
                    exit_code: result.exit_code,
                });
                let decision = classify_terminal_follow_up(&formatted);
                let body = if decision.output.is_empty() {
                    formatted
                } else {
                    decision.output
                };
                let text = prompt::follow_up_message(&body);
                let max_iterations = limits.max_iterations;
                this.store.update(cx, |s, cx| {
                    s.mark_executed(&sid, &action_id, cx);
                    s.send_follow_up(&sid, text, Some(follow_up_context), max_iterations, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn reject(&mut self, action_id: &str, cx: &mut Context<Self>) {
        let Some(sid) = self.target.as_ref().map(|t| t.session_id.clone()) else {
            return;
        };
        self.store
            .update(cx, |s, cx| s.reject_action(&sid, action_id, cx));
    }

    pub fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.show_history = !self.show_history;
        cx.notify();
    }

    pub fn input(&self) -> &Entity<TextareaState> {
        &self.input
    }

    pub fn executing_count(&self) -> usize {
        self.executing.len()
    }

    // ---- rendering ----

    fn message(&self, ix: usize, msg: &ChatMsg, cx: &App) -> Option<AnyElement> {
        if msg.role == Role::System {
            return None;
        }
        let theme = cx.theme();
        let user = msg.role == Role::User;
        let fallback = if msg.command_plan_notice.is_none() && !msg.streaming && !user {
            extract_empty_command_plan_notice(&msg.content)
        } else {
            None
        };
        let stripped = if user {
            msg.content.clone()
        } else {
            strip_command_plan_block(&msg.content)
        };
        let content = fallback
            .as_ref()
            .map(|f| f.display_content.clone())
            .unwrap_or(stripped);
        let notice = msg
            .command_plan_notice
            .clone()
            .or_else(|| fallback.and_then(|f| f.command_plan_notice));
        let has_actions = msg.pending_actions.iter().any(|a| !a.rejected);
        let empty = content.trim().is_empty() && notice.is_none();

        Some(
            v_flex()
                .id(ElementId::Name(format!("ai-msg-{ix}").into()))
                .test_support()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .when(user, |d| d.bg(theme.secondary))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground)
                        .child(if user {
                            t!("ai.roleUser")
                        } else {
                            t!("ai.roleAI")
                        }),
                )
                .when(!content.is_empty(), |d| {
                    d.child(div().text_size(px(12.)).child(content.clone()))
                })
                .when_some(notice, |d, n| {
                    d.child(
                        h_flex()
                            .gap_2()
                            .p_2()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.success)
                            .child(div().text_color(theme.success).child("✓"))
                            .child(
                                v_flex()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("ai.noCommandPlanTitle")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.muted_foreground)
                                            .child(n.summary),
                                    ),
                            ),
                    )
                })
                .when(empty && msg.streaming, |d| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(format!("{}…", t!("ai.thinking"))),
                    )
                })
                .when(empty && !msg.streaming && has_actions, |d| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("ai.commandPlanReady")),
                    )
                })
                .into_any_element(),
        )
    }

    fn command_item(
        &self,
        ix: usize,
        action: &ParsedPendingAction,
        state: ItemState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let assessment = action.assessment.clone();
        let blocked = assessment
            .as_ref()
            .is_some_and(|a| a.decision == AiActionDecision::Block);
        let id = action.id.clone();
        let reject_id = action.id.clone();
        let risk_color = |r: &AiActionRiskLevel| match r {
            AiActionRiskLevel::Low => theme.success,
            AiActionRiskLevel::Medium => theme.warning,
            AiActionRiskLevel::High | AiActionRiskLevel::Critical => theme.danger,
        };
        v_flex()
            .id(ElementId::Name(
                format!("ai-cmd-{}-{ix}", state.name()).into(),
            ))
            .test_support()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(if blocked { theme.danger } else { theme.border })
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .size(px(16.))
                            .flex_none()
                            .rounded_full()
                            .bg(theme.muted)
                            .text_size(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child((ix + 1).to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(12.))
                            .child(action.description.clone()),
                    ),
            )
            .when_some(action.command.clone(), |d, c| {
                d.child(
                    div()
                        .p_1p5()
                        .rounded(px(4.))
                        .bg(theme.muted)
                        .font_family("Menlo")
                        .text_size(px(11.))
                        .child(c),
                )
            })
            .when(state == ItemState::Pending, |d| {
                d.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_size(px(11.))
                        .when(action.assessment_loading, |d| {
                            d.child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("ai.assessingRisk")),
                            )
                        })
                        .when_some(assessment.clone(), |d, a| {
                            let color = risk_color(&a.risk_level);
                            d.child(div().text_color(color).child(t!(&format!(
                                "ai.risk.{}",
                                format!("{:?}", a.risk_level).to_lowercase()
                            ))))
                            .child(div().text_color(color).child(t!(&format!(
                                "ai.decision.{}",
                                format!("{:?}", a.decision).to_lowercase()
                            ))))
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child(a.matched_rule),
                            )
                        }),
                )
                .when_some(
                    assessment
                        .as_ref()
                        .map(|a| a.reason.clone())
                        .filter(|r| !r.is_empty()),
                    |d, r| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(r),
                        )
                    },
                )
                .when_some(action.assessment_error.clone(), |d, e| {
                    d.child(div().text_size(px(11.)).text_color(theme.danger).child(e))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(ElementId::Name(format!("ai-approve-{ix}").into()))
                                .primary()
                                .small()
                                .label(format!("↵ {}", t!("ai.approveAndInject")))
                                .tooltip(if blocked {
                                    t!("ai.blockedTitle")
                                } else {
                                    t!("ai.approveTitle")
                                })
                                .disabled(action.assessment_loading || blocked)
                                .on_click(cx.listener(move |this, _, _, cx| this.approve(&id, cx))),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("ai-reject-{ix}").into()))
                                .small()
                                .label(format!("✗ {}", t!("ai.reject")))
                                .tooltip(t!("ai.rejectTitle"))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.reject(&reject_id, cx)),
                                ),
                        ),
                )
            })
            .when(state == ItemState::Executing, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.primary)
                        .child(format!("● {}", t!("ai.executing"))),
                )
            })
            .when(state == ItemState::Done, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.success)
                        .child(format!("✓ {}", t!("ai.executed"))),
                )
            })
            .into_any_element()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Pending,
    Executing,
    Done,
}

impl ItemState {
    fn name(self) -> &'static str {
        match self {
            ItemState::Pending => "pending",
            ItemState::Executing => "executing",
            ItemState::Done => "done",
        }
    }
}

impl Render for AiChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let store = self.store.read(cx);
        let session = self
            .target
            .as_ref()
            .and_then(|t| store.session(&t.session_id))
            .cloned();
        let conversations = self
            .target
            .as_ref()
            .map(|t| store.conversations_of(&t.session_id).to_vec())
            .unwrap_or_default();
        let messages = session
            .as_ref()
            .map(|s| s.messages.clone())
            .unwrap_or_default();
        let streaming = session.as_ref().is_some_and(|s| s.streaming());
        let pending: Vec<ParsedPendingAction> = session
            .as_ref()
            .map(|s| s.pending_actions().into_iter().cloned().collect())
            .unwrap_or_default();
        let done: Vec<ParsedPendingAction> = session
            .as_ref()
            .map(|s| s.executed_actions().into_iter().cloned().collect())
            .unwrap_or_default();
        let active_conv = session
            .as_ref()
            .and_then(|s| s.active_conversation_id.clone());
        let has_text = !self.input.read(cx).value().trim().is_empty();
        let typed = self.input.read(cx).value().to_string();
        let slash_items = quick::slash_query(&typed)
            .map(|q| self.slash_items(q, cx))
            .unwrap_or_default();
        let unfinished = !pending.is_empty();
        let (executing, waiting): (Vec<_>, Vec<_>) = pending
            .iter()
            .cloned()
            .partition(|a| self.executing.contains(&a.id));
        let title = self
            .target
            .as_ref()
            .map(|t| t.label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| t!("ai.assistantTitle"));

        let history = self.show_history.then(|| {
            v_flex()
                .id("ai-history")
                .test_support()
                .max_h(px(140.))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(theme.border)
                .when(conversations.is_empty(), |d| {
                    d.child(
                        div()
                            .p_2()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("ai.noHistory")),
                    )
                })
                .children(conversations.iter().enumerate().map(|(i, c)| {
                    let open_id = c.id.clone();
                    let del_id = c.id.clone();
                    h_flex()
                        .id(ElementId::Name(format!("ai-history-item-{i}").into()))
                        .test_support()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(active_conv.as_deref() == Some(c.id.as_str()), |d| {
                            d.bg(theme.accent)
                        })
                        .hover(|d| d.bg(theme.accent))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .child(if c.title.is_empty() {
                                    t!("ai.noTitle")
                                } else {
                                    c.title.clone()
                                }),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("ai-history-del-{i}").into()))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let id = del_id.clone();
                                    if let Some(sid) =
                                        this.target.as_ref().map(|t| t.session_id.clone())
                                    {
                                        this.store.update(cx, |s, cx| {
                                            s.delete_conversation(&sid, id, cx)
                                        });
                                    }
                                })),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = open_id.clone();
                            if let Some(sid) = this.target.as_ref().map(|t| t.session_id.clone()) {
                                this.store
                                    .update(cx, |s, cx| s.open_conversation(&sid, id, cx));
                            }
                            this.show_history = false;
                            cx.notify();
                        }))
                }))
                .into_any_element()
        });

        let messages_view = if messages.is_empty() {
            // `.ai-chat-empty`: ⌨ at 24px, two weak lines, 4px apart
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .text_color(ui.text_muted)
                .child(div().text_size(px(24.)).mb(px(4.)).child("⌨"))
                .child(div().text_size(px(11.)).child(t!("ai.emptyPrompt")))
                .child(div().text_size(px(10.)).child(t!("ai.emptyHint")))
                .into_any_element()
        } else {
            v_flex()
                .id("ai-messages")
                .test_support()
                .flex_1()
                .min_h_0()
                .gap_1()
                .p_2()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .children(
                    messages
                        .iter()
                        .enumerate()
                        .filter_map(|(i, m)| self.message(i, m, cx)),
                )
                .into_any_element()
        };

        let left = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_r_1()
            .border_color(ui.term_border)
            .child(
                h_flex()
                    .flex_none()
                    .px(px(8.))
                    .py(px(3.))
                    .gap(px(4.))
                    .items_center()
                    .border_b_1()
                    .border_color(ui.term_border)
                    .child(
                        div()
                            .id("ai-new")
                            .test_support()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(ui.text_secondary)
                            .hover(move |d| d.bg(ui.term_border))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(t!("ai.newChat"))
                                    .build(window, cx)
                            })
                            .child("＋")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(sid) =
                                    this.target.as_ref().map(|t| t.session_id.clone())
                                {
                                    this.store.update(cx, |s, cx| s.new_conversation(&sid, cx));
                                }
                            })),
                    )
                    .child(
                        div()
                            .id("ai-history-toggle")
                            .test_support()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(ui.text_secondary)
                            .when(self.show_history, |d| d.bg(ui.term_border))
                            .hover(move |d| d.bg(ui.term_border))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(t!("ai.chatHistory"))
                                    .build(window, cx)
                            })
                            .child("☰")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx))),
                    )
                    .child(
                        div()
                            .ml(px(8.))
                            .text_size(px(10.))
                            .text_color(ui.text_secondary)
                            .truncate()
                            .child(title),
                    ),
            )
            .when_some(history, |d, h| d.child(h))
            .child(messages_view)
            .when(!slash_items.is_empty(), |d| {
                let weak = cx.entity().downgrade();
                d.child(
                    v_flex()
                        .id("ai-slash-list")
                        .test_support()
                        .flex_none()
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .mx(px(8.))
                        .mb(px(2.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(ui.term_border)
                        .bg(ui.terminal_panel_bg)
                        .children(slash_items.into_iter().enumerate().map(|(ix, item)| {
                            let w = weak.clone();
                            let description = match &item.kind {
                                SlashKind::Search => t!("ai.slashSearchDesc"),
                                SlashKind::Stop => t!("ai.slashStopDesc"),
                                _ => item.description.clone(),
                            };
                            let title = match &item.kind {
                                SlashKind::Search => t!("ai.slashSearch"),
                                SlashKind::Stop => t!("ai.slashStop"),
                                _ => item.title.clone(),
                            };
                            let picked = item.clone();
                            h_flex()
                                .id(SharedString::from(format!("ai-slash-item-{ix}")))
                                .test_support()
                                .gap(px(8.))
                                .px(px(8.))
                                .py(px(3.))
                                .items_baseline()
                                .cursor_pointer()
                                .text_size(px(11.))
                                .when(ix == 0, |d| d.bg(ui.term_border))
                                .hover(move |d| d.bg(ui.term_border))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family("Menlo")
                                        .text_color(ui.term_accent)
                                        .child(format!("/{}", item.slug)),
                                )
                                .child(div().text_color(ui.text).truncate().child(title))
                                .when(!description.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(ui.text_secondary)
                                            .child(description),
                                    )
                                })
                                .on_click(move |_, window, cx| {
                                    let picked = picked.clone();
                                    let _ = w.update(cx, |this, cx| {
                                        this.apply_slash(picked, window, cx)
                                    });
                                })
                        })),
                )
            })
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(4.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .px(px(8.))
                    .items_end()
                    .border_t_1()
                    .border_color(ui.term_border)
                    .child(
                        // `.ai-chat-input`: 4px radius, hairline border, filled with the terminal border colour
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(ui.term_border)
                            .bg(ui.term_border)
                            .px(px(4.))
                            .py(px(4.))
                            // `rows={1}`: one line (25.5px with the border), longer text scrolls inside
                            .h(px(25.5))
                            .overflow_hidden()
                            .child(
                                Textarea::new(&self.input)
                                    .xsmall()
                                    .appearance(false)
                                    .h(px(15.))
                                    .text_size(px(11.))
                                    .line_height(relative(1.4))
                                    .disabled(streaming || unfinished),
                            ),
                    )
                    .child({
                        // `.ai-chat-send`: accent-coloured ➤ on a faint blue fill, 30% opacity when disabled
                        let off = streaming || unfinished || !has_text;
                        let accent = ui.term_accent;
                        div()
                            .id("ai-send")
                            .test_support()
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(gpui_kit::rgba(0x80caff26))
                            .bg(gpui_kit::rgba(0x80caff1a))
                            .text_color(accent)
                            .text_size(px(12.))
                            .when(off, |d| d.opacity(0.3))
                            .when(!off, |d| {
                                d.cursor_pointer()
                                    .hover(|d| d.bg(gpui_kit::rgba(0x80caff2e)))
                            })
                            .child(if streaming { "…" } else { "➤" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !off {
                                    this.send(window, cx)
                                }
                            }))
                    }),
            );

        // `.ai-cmd-panel`: 280px (min 220), terminal panel colour
        let right = v_flex()
            .w(px(280.))
            .min_w(px(220.))
            .flex_none()
            .h_full()
            .child(
                h_flex()
                    .flex_none()
                    .px(px(8.))
                    .py(px(3.))
                    .gap(px(6.))
                    .items_center()
                    .border_b_1()
                    .border_color(ui.term_border)
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text_secondary)
                            .child(t!("ai.commandConfirm")),
                    )
                    .when(!pending.is_empty(), |d| {
                        d.child(
                            div()
                                .id("ai-cmd-badge")
                                .test_support()
                                .px(px(6.))
                                .rounded(px(8.))
                                .bg(gpui_kit::rgb(0xc084fc))
                                .text_color(gpui_kit::rgb(0x11161c))
                                .text_size(px(9.))
                                .font_weight(FontWeight::BOLD)
                                .line_height(px(16.))
                                .child(pending.len().to_string()),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("ai-cmd-list")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .p_2()
                    .overflow_y_scroll()
                    .when(
                        executing.is_empty() && waiting.is_empty() && done.is_empty(),
                        |d| {
                            // `.ai-cmd-empty`: ⏳ at 18px over a 10px weak line
                            d.items_center()
                                .justify_center()
                                .gap(px(4.))
                                .text_color(ui.text_muted)
                                .text_size(px(10.))
                                .child(div().text_size(px(18.)).child("⏳"))
                                .child(t!("ai.waitingForCommand"))
                        },
                    )
                    .children(
                        executing
                            .iter()
                            .enumerate()
                            .map(|(i, a)| self.command_item(i, a, ItemState::Executing, cx)),
                    )
                    .children(waiting.iter().enumerate().map(|(i, a)| {
                        self.command_item(executing.len() + i, a, ItemState::Pending, cx)
                    }))
                    .children(
                        done.iter()
                            .enumerate()
                            .map(|(i, a)| self.command_item(i, a, ItemState::Done, cx)),
                    ),
            );

        // `.ai-chat-panel`: terminal panel colour, terminal font at 11px
        h_flex()
            .id("ai-panel")
            .test_support()
            .size_full()
            .bg(ui.terminal_panel_bg)
            .text_color(ui.term_fg)
            .font_family("Menlo")
            .text_size(px(11.))
            .line_height(relative(1.4))
            .child(left)
            .child(right)
    }
}
