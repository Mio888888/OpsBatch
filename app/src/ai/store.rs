//! AI chat state (port of `stores/aiChat.ts`): one chat per terminal session, streamed through the `ai-stream-chunk`
//! event, with the assistant's commands parsed into approvable actions that are risk-assessed by the Rust policy.

use std::collections::HashMap;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use ops_ai::client::{
    ai_assess_action, ai_chat_stream, ai_delete_conversation, ai_get_conversation,
    ai_list_conversations, ai_record_action_event, AiActionAssessment, AiActionAuditEvent,
    AiActionDecision, AiActionRiskLevel, ChatMessage, ChatMessageImage, Conversation,
    StreamChatRequest,
};
use ops_ai::parser::{
    parse_ai_pending_actions_for, ActionKind, CommandPlanNotice, ParsedPendingAction,
};
use ops_i18n::t;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "user" => Role::User,
            "system" => Role::System,
            _ => Role::Assistant,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMsg {
    pub id: String,
    pub role: Role,
    pub content: String,
    /// `data:` URLs of pasted images.
    pub images: Vec<String>,
    pub model: Option<String>,
    pub streaming: bool,
    pub pending_actions: Vec<ParsedPendingAction>,
    pub command_plan_notice: Option<CommandPlanNotice>,
    pub rdp_parse_error: Option<String>,
    pub rdp_parse_diagnostic: Option<String>,
}

impl ChatMsg {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.into(),
            images: Vec::new(),
            model: None,
            streaming: false,
            pending_actions: Vec::new(),
            command_plan_notice: None,
            rdp_parse_error: None,
            rdp_parse_diagnostic: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiScope {
    pub scope: String,
    pub scope_id: String,
}

impl AiScope {
    pub fn new(scope: &str, scope_id: &str) -> Self {
        let scope = scope.trim();
        Self {
            scope: if scope.is_empty() {
                "global".into()
            } else {
                scope.into()
            },
            scope_id: scope_id.trim().into(),
        }
    }

    pub fn global() -> Self {
        Self::new("global", "")
    }

    /// Terminal scopes get one command at a time.
    pub fn single_command(&self) -> bool {
        self.scope == "ssh_host" || self.scope == "terminal_session"
    }
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub messages: Vec<ChatMsg>,
    pub active_conversation_id: Option<String>,
    pub streaming_message_id: Option<String>,
    pub input_text: String,
    pub scope: AiScope,
    pub active_request_id: Option<String>,
    /// Size of the remote desktop when this chat drives an RDP session (enables `<RDP_PLAN>` parsing).
    pub rdp_desktop: Option<(u32, u32)>,
    /// Automatic follow-ups since the user's last message (capped by the "max iterations" safety setting).
    pub follow_ups: u32,
}

impl SessionState {
    fn new(scope: AiScope) -> Self {
        Self {
            messages: Vec::new(),
            active_conversation_id: None,
            streaming_message_id: None,
            input_text: String::new(),
            scope,
            active_request_id: None,
            rdp_desktop: None,
            follow_ups: 0,
        }
    }

    fn reset_conversation(&mut self) {
        self.active_conversation_id = None;
        self.messages.clear();
        self.streaming_message_id = None;
        self.active_request_id = None;
    }

    pub fn streaming(&self) -> bool {
        self.streaming_message_id.is_some()
    }

    /// Actions the user has neither approved nor rejected yet.
    pub fn pending_actions(&self) -> Vec<&ParsedPendingAction> {
        self.messages
            .iter()
            .flat_map(|m| m.pending_actions.iter())
            .filter(|a| !a.rejected && !a.approved && !a.executed)
            .collect()
    }

    pub fn executed_actions(&self) -> Vec<&ParsedPendingAction> {
        self.messages
            .iter()
            .flat_map(|m| m.pending_actions.iter())
            .filter(|a| !a.rejected && a.executed)
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub delta: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub client_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AiStoreEvent {
    /// Every stream chunk, for views that run their own request (the ⌘I panel).
    Chunk(StreamChunk),
}

pub struct AiChatStore {
    pub active_session: Option<String>,
    pub sessions: HashMap<String, SessionState>,
    /// Conversation lists per chat session.
    pub conversations: HashMap<String, Vec<Conversation>>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<AiStoreEvent> for AiChatStore {}

pub struct AiChatHandle(pub Entity<AiChatStore>);
impl Global for AiChatHandle {}

pub fn ai_store(cx: &App) -> Entity<AiChatStore> {
    cx.global::<AiChatHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let store = cx.new(AiChatStore::new);
    cx.set_global(AiChatHandle(store));
}

fn blocked_assessment(error: &str) -> AiActionAssessment {
    AiActionAssessment {
        decision: AiActionDecision::Block,
        risk_level: AiActionRiskLevel::Critical,
        risk_score: 100,
        matched_rule: "ASSESSMENT_FAILED".into(),
        reason: error.into(),
        capabilities: vec!["policy_error".into()],
    }
}

fn history_message(m: ops_ai::client::ConversationMessage) -> ChatMsg {
    let mut msg = ChatMsg::new(Role::parse(&m.role), m.content);
    msg.id = m.id;
    if !m.model.is_empty() {
        msg.model = Some(m.model);
    }
    msg
}

impl AiChatStore {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ctx = crate::services::ctx(cx);
        let events = ctx.events.subscribe("ai-stream-chunk");
        let task = cx.spawn(async move |this, cx| {
            while let Ok(ev) = events.recv_async().await {
                let Some(chunk) = ev.parse::<StreamChunk>() else {
                    continue;
                };
                if this
                    .update(cx, |this, cx| this.on_chunk(chunk, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            active_session: None,
            sessions: HashMap::new(),
            conversations: HashMap::new(),
            _tasks: vec![task],
        }
    }

    // ---- accessors ----

    pub fn session(&self, id: &str) -> Option<&SessionState> {
        self.sessions.get(id)
    }

    pub fn conversations_of(&self, sid: &str) -> &[Conversation] {
        self.conversations
            .get(sid)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut SessionState> {
        self.sessions.get_mut(id)
    }

    // ---- sessions ----

    pub fn activate_session(&mut self, session_id: &str, scope: AiScope, cx: &mut Context<Self>) {
        match self.sessions.get_mut(session_id) {
            None => {
                self.sessions
                    .insert(session_id.to_string(), SessionState::new(scope));
            }
            Some(existing) if existing.scope != scope => {
                existing.scope = scope;
                existing.reset_conversation();
            }
            Some(_) => {}
        }
        self.active_session = Some(session_id.to_string());
        cx.notify();
    }

    pub fn clear_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.sessions.remove(session_id);
        if self.active_session.as_deref() == Some(session_id) {
            self.active_session = None;
        }
        cx.notify();
    }

    pub fn set_input_text(&mut self, sid: &str, text: String, cx: &mut Context<Self>) {
        if let Some(s) = self.session_mut(sid) {
            s.input_text = text;
            cx.notify();
        }
    }

    pub fn set_rdp_desktop(&mut self, sid: &str, desktop: Option<(u32, u32)>) {
        if let Some(s) = self.session_mut(sid) {
            s.rdp_desktop = desktop;
        }
    }

    // ---- conversations ----

    pub fn load_conversations(&mut self, sid: &str, cx: &mut Context<Self>) {
        let Some(scope) = self.sessions.get(sid).map(|s| s.scope.clone()) else {
            return;
        };
        let ctx = crate::services::ctx(cx);
        let (req, sid) = (scope.clone(), sid.to_string());
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                ai_list_conversations(&ctx.db, Some(req.scope), Some(req.scope_id)).await
            })
            .await
            .and_then(|r| r);
            let Ok(list) = result else { return };
            let _ = this.update(cx, |this, cx| {
                // The session may have switched scope while the query ran.
                if this.sessions.get(&sid).is_some_and(|s| s.scope == scope) {
                    this.conversations.insert(sid, list);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_conversation(&mut self, sid: &str, id: String, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        let sid = sid.to_string();
        cx.spawn(async move |this, cx| {
            let cid = id.clone();
            let result = ops_core::rt::run(async move { ai_get_conversation(&ctx.db, cid).await })
                .await
                .and_then(|r| r);
            let Ok((conversation, msgs)) = result else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(&sid) else {
                    return;
                };
                if session.scope.scope != conversation.scope
                    || session.scope.scope_id != conversation.scope_id
                {
                    return;
                }
                session.active_conversation_id = Some(id);
                session.messages = msgs.into_iter().map(history_message).collect();
                session.streaming_message_id = None;
                session.active_request_id = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn new_conversation(&mut self, sid: &str, cx: &mut Context<Self>) {
        if let Some(s) = self.session_mut(sid) {
            s.reset_conversation();
            cx.notify();
        }
    }

    pub fn delete_conversation(&mut self, sid: &str, id: String, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        let sid = sid.to_string();
        cx.spawn(async move |this, cx| {
            let cid = id.clone();
            let result =
                ops_core::rt::run(async move { ai_delete_conversation(&ctx.db, cid).await })
                    .await
                    .and_then(|r| r);
            if result.is_err() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = this.session_mut(&sid) {
                    if s.active_conversation_id.as_deref() == Some(id.as_str()) {
                        s.reset_conversation();
                    }
                }
                this.load_conversations(&sid, cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- sending ----

    /// Send session `sid`'s input box; `false` when there was nothing to send or a reply is still streaming.
    pub fn send_message(
        &mut self,
        sid: &str,
        context: Option<String>,
        images: Vec<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.sessions.get(sid) else {
            return false;
        };
        let text = session.input_text.trim().to_string();
        if session.streaming() || text.is_empty() {
            return false;
        }
        if let Some(session) = self.sessions.get_mut(sid) {
            session.follow_ups = 0;
        }
        self.start_stream(sid, text, context, images, true, cx);
        true
    }

    /// The automatic message after a command ran. Refuses (with a note in the chat) once `max_iterations` follow-ups
    /// happened since the user's last message, so a model that keeps proposing commands cannot loop forever.
    pub fn send_follow_up(
        &mut self,
        sid: &str,
        text: String,
        context: Option<String>,
        max_iterations: u32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(sid) else {
            return false;
        };
        if session.follow_ups >= max_iterations {
            session.messages.push(ChatMsg::new(
                Role::System,
                t!("ai.maxIterationsReached", count = max_iterations),
            ));
            cx.notify();
            return false;
        }
        session.follow_ups += 1;
        self.send_direct_message(sid, text, context, Vec::new(), cx);
        true
    }

    /// A fresh turn that was not typed (e.g. "analyze selection") starts the iteration count over.
    pub fn reset_follow_ups(&mut self, sid: &str) {
        if let Some(session) = self.sessions.get_mut(sid) {
            session.follow_ups = 0;
        }
    }

    /// Send `text` into session `sid` without touching its input box.
    pub fn send_direct_message(
        &mut self,
        sid: &str,
        text: String,
        context: Option<String>,
        images: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.sessions.get(sid) else {
            return;
        };
        if session.streaming() {
            return;
        }
        self.start_stream(sid, text, context, images, false, cx);
    }

    fn start_stream(
        &mut self,
        sid: &str,
        text: String,
        context: Option<String>,
        images: Vec<String>,
        clear_input: bool,
        cx: &mut Context<Self>,
    ) {
        let client_request_id = uuid::Uuid::new_v4().to_string();
        let mut user = ChatMsg::new(Role::User, text);
        user.images = images;
        let mut assistant = ChatMsg::new(Role::Assistant, "");
        assistant.streaming = true;
        let assistant_id = assistant.id.clone();

        let Some(session) = self.sessions.get_mut(sid) else {
            return;
        };
        let to_api = |m: &ChatMsg| ChatMessage {
            role: m.role.as_str().into(),
            content: m.content.clone(),
            images: if m.images.is_empty() {
                None
            } else {
                Some(
                    m.images
                        .iter()
                        .map(|d| ChatMessageImage {
                            data_url: d.clone(),
                        })
                        .collect(),
                )
            },
        };
        let mut api_messages: Vec<ChatMessage> = session.messages.iter().map(to_api).collect();
        api_messages.push(to_api(&user));
        if let Some(context) = context.filter(|c| !c.is_empty()) {
            api_messages.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    content: context,
                    images: None,
                },
            );
        }
        let request = StreamChatRequest {
            messages: api_messages,
            model: None,
            temperature: None,
            max_tokens: None,
            conversation_id: session.active_conversation_id.clone(),
            scope: Some(session.scope.scope.clone()),
            scope_id: Some(session.scope.scope_id.clone()),
            client_request_id: Some(client_request_id.clone()),
        };

        session.messages.push(user);
        session.messages.push(assistant);
        session.streaming_message_id = Some(assistant_id.clone());
        session.active_request_id = Some(client_request_id.clone());
        if clear_input {
            session.input_text.clear();
        }
        cx.notify();

        let ctx = crate::services::ctx(cx);
        let sid = sid.to_string();
        cx.spawn(async move |this, cx| {
            let result =
                ops_core::rt::run(async move { ai_chat_stream(&ctx.core, &ctx.db, request).await })
                    .await
                    .and_then(|r| r);
            if let Err(e) = result {
                let _ = this.update(cx, |this, cx| {
                    let Some(s) = this.session_mut(&sid) else {
                        return;
                    };
                    if s.active_request_id.as_deref() != Some(client_request_id.as_str()) {
                        return;
                    }
                    if let Some(m) = s.messages.iter_mut().find(|m| m.id == assistant_id) {
                        m.content = format!("错误: {e}");
                        m.streaming = false;
                    }
                    s.streaming_message_id = None;
                    s.active_request_id = None;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // ---- stream events ----

    fn on_chunk(&mut self, chunk: StreamChunk, cx: &mut Context<Self>) {
        cx.emit(AiStoreEvent::Chunk(chunk.clone()));
        let sid = match &chunk.client_request_id {
            Some(rid) => self
                .sessions
                .iter()
                .find(|(_, s)| s.active_request_id.as_deref() == Some(rid.as_str()))
                .map(|(id, _)| id.clone()),
            None => self.active_session.clone(),
        };
        let Some(sid) = sid else { return };
        let Some(session) = self.sessions.get_mut(&sid) else {
            return;
        };
        let Some(streaming_id) = session.streaming_message_id.clone() else {
            return;
        };

        if !chunk.done {
            // The conversation id is only known once the first chunk arrives for a new chat.
            if session.active_conversation_id.is_none() && !chunk.conversation_id.is_empty() {
                session.active_conversation_id = Some(chunk.conversation_id.clone());
            }
            if let Some(m) = session.messages.iter_mut().find(|m| m.id == streaming_id) {
                m.content.push_str(&chunk.delta);
            }
            cx.notify();
            return;
        }

        let raw = session
            .messages
            .iter()
            .find(|m| m.id == streaming_id)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        if let Some(m) = session.messages.iter_mut().find(|m| m.id == streaming_id) {
            m.streaming = false;
            m.model = Some(chunk.model.clone());
        }
        session.streaming_message_id = None;
        session.active_request_id = None;
        if session.active_conversation_id.is_none() {
            session.active_conversation_id = Some(chunk.conversation_id.clone());
        }
        let single_command = session.scope.single_command();
        let rdp_desktop = session.rdp_desktop;

        let parsed = parse_ai_pending_actions_for(&raw, rdp_desktop, single_command);
        let mut actions = parsed.actions;
        for a in &mut actions {
            a.assessment_loading = a.kind == ActionKind::Command;
        }
        let to_assess: Vec<(String, String)> = actions
            .iter()
            .filter_map(|a| a.command.clone().map(|c| (a.id.clone(), c)))
            .collect();
        if let Some(m) = session.messages.iter_mut().find(|m| m.id == streaming_id) {
            m.content = parsed.display_content;
            m.pending_actions = actions;
            m.command_plan_notice = parsed.command_plan_notice;
            m.rdp_parse_error = parsed.rdp_parse_error.clone();
            m.rdp_parse_diagnostic = parsed.rdp_parse_diagnostic.clone();
        }
        cx.notify();

        if let Some(error) = &parsed.rdp_parse_error {
            let ctx = crate::services::ctx(cx);
            let detail = format!(
                "messageId={streaming_id}\nconversationId={}\nerror={error}\n{}",
                chunk.conversation_id,
                parsed
                    .rdp_parse_diagnostic
                    .as_deref()
                    .unwrap_or("diagnostic=<empty>")
            );
            ops_core::log::emit_log(&ctx.core, "warn", "rdp.ai.parse", &detail, "frontend");
        }
        for (action_id, command) in to_assess {
            self.assess_action(&sid, &streaming_id, action_id, command, cx);
        }
        self.load_conversations(&sid, cx);
    }

    fn assess_action(
        &mut self,
        sid: &str,
        message_id: &str,
        action_id: String,
        command: String,
        cx: &mut Context<Self>,
    ) {
        let ctx = crate::services::ctx(cx);
        let (sid, message_id) = (sid.to_string(), message_id.to_string());
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { ai_assess_action(&ctx.db, command) })
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                let Some(action) = this.find_action_mut(&sid, &message_id, &action_id) else {
                    return;
                };
                action.assessment_loading = false;
                match result {
                    Ok(a) => {
                        action.assessment = Some(a);
                        action.assessment_error = None;
                    }
                    Err(e) => {
                        let error = format!("策略评估失败: {e}");
                        action.assessment = Some(blocked_assessment(&error));
                        action.assessment_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn find_action_mut(
        &mut self,
        sid: &str,
        message_id: &str,
        action_id: &str,
    ) -> Option<&mut ParsedPendingAction> {
        self.sessions
            .get_mut(sid)?
            .messages
            .iter_mut()
            .find(|m| m.id == message_id)?
            .pending_actions
            .iter_mut()
            .find(|a| a.id == action_id)
    }

    fn find_action(&self, sid: &str, action_id: &str) -> Option<&ParsedPendingAction> {
        self.sessions
            .get(sid)?
            .messages
            .iter()
            .flat_map(|m| m.pending_actions.iter())
            .find(|a| a.id == action_id)
    }

    // ---- approval ----

    /// Policy gate before a command runs: records `blocked` and refuses for BLOCK decisions, otherwise records
    /// `approved` and returns the action. Nothing is marked executed yet (see `mark_executed`).
    pub fn begin_approval(
        &mut self,
        sid: &str,
        action_id: &str,
        terminal_session: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<ParsedPendingAction> {
        let action = self.find_action(sid, action_id)?.clone();
        if action.approved || action.executed {
            return None;
        }
        let conversation = self
            .sessions
            .get(sid)
            .and_then(|s| s.active_conversation_id.clone());
        if action
            .assessment
            .as_ref()
            .is_some_and(|a| a.decision == AiActionDecision::Block)
        {
            self.record_event(&action, "blocked", terminal_session, conversation, cx);
            return None;
        }
        self.record_event(&action, "approved", terminal_session, conversation, cx);
        Some(action)
    }

    /// Remote-desktop actions are not audited (the original only records shell commands).
    pub fn begin_rdp_approval(
        &mut self,
        sid: &str,
        action_id: &str,
        cx: &App,
    ) -> Option<ParsedPendingAction> {
        // observer mode is read-only: the assistant may look at the desktop but not operate it
        if crate::ai::runtime::limits(cx).permission_mode == ops_ai::prefs::PermissionMode::Observer
        {
            return None;
        }
        let action = self.find_action(sid, action_id)?;
        (action.kind == ActionKind::RdpAction
            && !action.approved
            && !action.executed
            && !action.rejected)
            .then(|| action.clone())
    }

    pub fn mark_executed(&mut self, sid: &str, action_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get_mut(sid) else {
            return;
        };
        for a in session
            .messages
            .iter_mut()
            .flat_map(|m| m.pending_actions.iter_mut())
            .filter(|a| a.id == action_id)
        {
            a.approved = true;
            a.executed = true;
        }
        cx.notify();
    }

    pub fn reject_action(&mut self, sid: &str, action_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get_mut(sid) else {
            return;
        };
        let conversation = session.active_conversation_id.clone();
        let mut rejected = None;
        for a in session
            .messages
            .iter_mut()
            .flat_map(|m| m.pending_actions.iter_mut())
            .filter(|a| a.id == action_id)
        {
            a.approved = false;
            a.executed = false;
            a.rejected = true;
            rejected = Some(a.clone());
        }
        cx.notify();
        if let Some(action) = rejected {
            self.record_event(&action, "rejected", None, conversation, cx);
        }
    }

    fn record_event(
        &self,
        action: &ParsedPendingAction,
        event: &str,
        session_id: Option<&str>,
        conversation: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (Some(command), Some(assessment)) = (action.command.clone(), action.assessment.clone())
        else {
            return;
        };
        if action.kind != ActionKind::Command {
            return;
        }
        let ctx = crate::services::ctx(cx);
        let audit = AiActionAuditEvent {
            action_id: action.id.clone(),
            event: event.to_string(),
            command,
            conversation_id: conversation,
            session_id: session_id.map(str::to_string),
            host: None,
            assessment,
        };
        cx.background_spawn(async move {
            let _ = ops_core::rt::run(async move { ai_record_action_event(&ctx.db, audit) }).await;
        })
        .detach();
    }
}
