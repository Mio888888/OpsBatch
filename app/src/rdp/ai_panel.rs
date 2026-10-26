//! The AI agent panel of the RDP window (port of `RdpAiPanel.tsx`): the model sees a screenshot, answers with an
//! `<RDP_PLAN>`, the user approves steps (or runs them all), and goal mode loops screenshot → plan → execute.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::parser::{ActionKind, ParsedPendingAction, RdpOperation};
use ops_ai::client::{ai_chat, ChatMessage, ChatMessageImage, ChatRequest};
use ops_ai::rdp_parser::strip_rdp_plan_block;
use ops_ai::rdp_verify::{build_verification_prompt, parse_verdict, verification_system_prompt};
use ops_i18n::t;

use super::window::RdpWindow;
use crate::ai::store::{ai_store, AiChatStore, AiScope, ChatMsg, Role};

const GOAL_MAX_ROUNDS: u32 = 8;
/// Follow-up questions to the model after a plain request before the assistant stops asking on its own.
const FOLLOW_UP_MAX_ROUNDS: u32 = 12;
const GOAL_SETTLE: Duration = Duration::from_millis(2500);
const GOAL_FINISH_DELAY: Duration = Duration::from_millis(1500);

/// The system prompt that teaches the model the `<RDP_PLAN>` format (verbatim from the Tauri build).
pub fn build_rdp_agent_context(width: u32, height: u32) -> String {
    [
        "你是一个 Windows 远程桌面操作助手（agent-rdp）。".to_string(),
        format!("当前远程桌面分辨率：{width}x{height}，坐标原点 (0,0) 在左上角。"),
        "当需要操作远程桌面时，必须输出一个完整的 <RDP_PLAN> JSON 块，不要输出空对象，不要省略 steps。".into(),
        "必须用 <RDP_PLAN> 和 </RDP_PLAN> 标签包裹，不要改用 ```json 代码块，也不要在标签外重复这段 JSON。".into(),
        "格式如下：".into(),
        "<RDP_PLAN>".into(),
        "{".into(),
        "  \"version\": 1,".into(),
        "  \"summary\": \"操作摘要\",".into(),
        "  \"steps\": [".into(),
        "    {".into(),
        "      \"description\": \"步骤描述\",".into(),
        "      \"intent\": \"click|type|key|scroll|navigate|launch|operate|interact|wait\",".into(),
        "      \"operations\": [".into(),
        "        {\"type\": \"click\", \"x\": 100, \"y\": 200},".into(),
        "        {\"type\": \"type\", \"text\": \"notepad\"},".into(),
        "        {\"type\": \"key\", \"keys\": [\"Enter\"]},".into(),
        "        {\"type\": \"key\", \"keys\": [\"Control\", \"s\"]},".into(),
        "        {\"type\": \"scroll\", \"x\": 400, \"y\": 300, \"delta\": -3},".into(),
        "        {\"type\": \"drag\", \"fromX\": 10, \"fromY\": 10, \"toX\": 500, \"toY\": 500},".into(),
        "        {\"type\": \"wait\", \"ms\": 1500}".into(),
        "      ],".into(),
        "      \"expected_outcome\": \"预期结果\"".into(),
        "    }".into(),
        "  ]".into(),
        "}".into(),
        "</RDP_PLAN>".into(),
        "操作类型说明：".into(),
        "- click: 单击坐标(x,y)，可选 button(0左/1中/2右)、doubleClick".into(),
        "- move: 移动鼠标到坐标".into(),
        "- drag: 从(fromX,fromY)拖拽到(toX,toY)".into(),
        "- scroll: 在(x,y)滚动，delta正数向上、负数向下".into(),
        "- type: 输入文本".into(),
        "- key: 按键，单键如 [\"Enter\"]，组合键如 [\"Control\",\"c\"]".into(),
        "- wait: 等待指定毫秒数，如 {\"type\":\"wait\",\"ms\":1500}；不要把 Wait 写成 key".into(),
        "支持按键：Enter, Tab, Escape, Backspace, Delete, Insert, Home, End, PageUp, PageDown, ArrowUp/Down/Left/Right, Space, Shift, Control, Alt, Meta, Win, F1-F12 等。".into(),
        "启动或打开应用（例如 Microsoft Edge、cmd、notepad）时，优先使用键盘路径：按 Win/Meta 打开开始菜单，搜索或 type 输入应用名/命令，再按 Enter；也可用 Win+R 运行框。不要优先依赖桌面图标坐标双击，除非用户明确要求点击当前屏幕上的某个图标。".into(),
        "普通字符直接用 type 输入。坐标必须基于实际分辨率，不要超出范围。".into(),
        "如果当前目标需要多步完成，只给出下一步可执行操作；这一步执行完并且画面稳定后，系统会把最新截图发给你，并询问请求是否已经完成，你再给出下一步，直到完成。".into(),
        "每个 step 必须包含非空 operations，不能只描述不操作。".into(),
        "系统按顺序逐个执行 operations：每个操作之后会等待远程画面稳定（弹窗/菜单出现、程序启动完成）再执行下一个，所以不需要在每个操作后手动加 wait；只有安装、下载、登录等需要明显更久的场景才加 wait。".into(),
        "如果后面的操作要点击的位置取决于前面操作的结果（比如先打开应用，再点击应用里的按钮），请把它们分成不同的 step：先只给出前一步，等看到新的画面再给出下一步，不要凭猜测给出坐标。".into(),
    ]
    .join("\n")
}

/// One-line description of an operation for the action card.
pub fn describe_operation(op: &RdpOperation) -> String {
    match op {
        RdpOperation::Click { x, y, button, double_click } => {
            let name = match button {
                2 => "右键",
                1 => "中键",
                _ => "左键",
            };
            if *double_click {
                format!("双击 ({x}, {y})")
            } else {
                format!("{name}点击 ({x}, {y})")
            }
        }
        RdpOperation::Move { x, y } => format!("移动到 ({x}, {y})"),
        RdpOperation::Drag { from_x, from_y, to_x, to_y, .. } => format!("拖拽 ({from_x}, {from_y}) → ({to_x}, {to_y})"),
        RdpOperation::Scroll { x, y, delta, vertical } => {
            format!("{}滚动 {} {} ({x}, {y})", if *vertical { "垂直" } else { "水平" }, if *delta > 0 { "↑" } else { "↓" }, delta.abs())
        }
        RdpOperation::Type { text } => {
            let shown = if text.chars().count() > 40 { format!("{}...", text.chars().take(37).collect::<String>()) } else { text.clone() };
            format!("输入文本: {shown}")
        }
        RdpOperation::Key { keys } => format!("按键: {}", keys.join(" + ")),
        RdpOperation::Wait { ms } => format!("等待 {} 秒", (f64::from(*ms) / 100.0).round() / 10.0),
    }
}

/// "Continue" / "next" replies keep the request that is already being worked on.
fn is_continuation(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['。', '.', '!', '！']).to_lowercase();
    matches!(t.as_str(), "继续" | "继续吧" | "接着" | "接着来" | "下一步" | "go on" | "continue" | "next")
}

/// What the model is asked after a step ran: look at the new screen, say whether the request is done, otherwise give
/// only the next step.
fn follow_up_prompt(request: &str, done: &[String], note: &str) -> String {
    let mut lines = vec![format!("用户请求：{request}")];
    lines.push(format!("刚才已执行：{}", done.join("；")));
    if !note.is_empty() {
        lines.push(format!("画面校验：{note}"));
    }
    lines.push("已附加执行后的 RDP 截图。请根据截图判断用户请求是否已经完成。如果已完成，直接回复“已完成”并简要说明结果，不要输出 <RDP_PLAN>；如果未完成，请只输出下一步需要的完整 <RDP_PLAN>（steps 和 operations 不能为空）。".to_string());
    lines.join("\n")
}

fn goal_prompt(target: &str, round: u32, check: bool, width: u32, height: u32) -> String {
    if check {
        format!("目标：{target}\n刚才执行了第 {round} 轮操作，并已附加当前 RDP 截图。请先根据截图判断目标是否已经完成。如果已完成，直接回复\"目标已完成\"并简要说明；如果未完成，请输出下一步完整 <RDP_PLAN>。")
    } else {
        format!("目标：{target}\n已附加当前 RDP 截图。请根据截图给出实现该目标的第一步操作计划。必须输出完整 <RDP_PLAN> JSON，steps 和 operations 不能为空。步骤要精确，坐标基于 {width}x{height} 分辨率。")
    }
}

/// What the model said about one executed step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Checking,
    Passed(String),
    Failed { observation: String, reason: String },
    /// The check itself could not be done (no model, network error, unreadable reply); the step stays unverified.
    Skipped(String),
}

/// How one step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepOutcome {
    Done,
    /// executed, but the screen says it went wrong
    Mistake,
    /// the input could not be played at all
    Failed,
}

#[derive(Default)]
struct Goal {
    active: bool,
    target: String,
    round: u32,
    busy: bool,
}

pub struct RdpAiPanel {
    win: WeakEntity<RdpWindow>,
    sid: String,
    store: Entity<AiChatStore>,
    input: Entity<TextareaState>,
    goal_input: Entity<InputState>,
    show_history: bool,
    goal_mode: bool,
    goal: Goal,
    /// Action currently being played by "run all" / goal mode.
    batch_id: Option<String>,
    /// look at the screen after every step and stop when it went wrong
    verify_steps: bool,
    /// after the steps ran, send the new screen to the model and ask for the next step (or "done")
    auto_continue: bool,
    /// what the user asked for (the text of the last request that was not just "continue")
    request: Option<String>,
    follow_ups: u32,
    verdicts: std::collections::HashMap<String, Verdict>,
    /// "run all" stopped because a step did not reach its goal
    batch_stopped: bool,
    running: HashSet<String>,
    scroll: ScrollHandle,
    tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl RdpAiPanel {
    pub fn new(win: WeakEntity<RdpWindow>, host_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ai_store(cx);
        let sid = format!("rdp-{host_id}");
        store.update(cx, |s, cx| {
            s.activate_session(&sid, AiScope::new("rdp_host", &host_id), cx);
            s.load_conversations(&sid, cx);
        });
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("rdp.aiInputPlaceholder")).auto_grow(1, 4).submit_on_enter(true));
        let goal_input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("rdp.aiGoalPlaceholder")));
        let subs = vec![
            cx.observe(&store, |this, _, cx| {
                this.scroll.scroll_to_bottom();
                this.drive_goal(cx);
                cx.notify();
            }),
            cx.observe(&input, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    let sid = this.sid.clone();
                    this.store.update(cx, |s, cx| s.set_input_text(&sid, text, cx));
                }
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&goal_input, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.start_goal(window, cx);
                }
            }),
        ];
        Self {
            win,
            sid,
            store,
            input,
            goal_input,
            show_history: false,
            goal_mode: false,
            goal: Goal::default(),
            batch_id: None,
            verify_steps: true,
            auto_continue: true,
            request: None,
            follow_ups: 0,
            verdicts: std::collections::HashMap::new(),
            batch_stopped: false,
            running: HashSet::new(),
            scroll: ScrollHandle::new(),
            tasks: Vec::new(),
            _subs: subs,
        }
    }

    pub fn session_key(&self) -> &str {
        &self.sid
    }
    pub fn input(&self) -> &Entity<TextareaState> {
        &self.input
    }
    pub fn goal_input(&self) -> &Entity<InputState> {
        &self.goal_input
    }
    pub fn goal_running(&self) -> bool {
        self.goal.active
    }
    pub fn goal_round(&self) -> u32 {
        self.goal.round
    }
    pub fn busy_count(&self) -> usize {
        self.running.len() + usize::from(self.batch_id.is_some())
    }

    fn desktop(&self, cx: &App) -> (u32, u32) {
        self.win.upgrade().map(|w| w.read(cx).desktop()).map(|(w, h)| (u32::from(w), u32::from(h))).unwrap_or((1280, 720))
    }

    fn screenshot_images(&self, cx: &App) -> Vec<String> {
        self.win.upgrade().and_then(|w| w.read(cx).screenshot()).into_iter().collect()
    }

    fn streaming(&self, cx: &App) -> bool {
        self.store.read(cx).session(&self.sid).is_some_and(|s| s.streaming())
    }

    fn rdp_actions(&self, cx: &App) -> Vec<ParsedPendingAction> {
        self.store
            .read(cx)
            .session(&self.sid)
            .map(|s| s.messages.iter().flat_map(|m| m.pending_actions.iter()).filter(|a| a.kind == ActionKind::RdpAction && !a.rejected).cloned().collect())
            .unwrap_or_default()
    }

    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if self.streaming(cx) || text.trim().is_empty() {
            return;
        }
        let (w, h) = self.desktop(cx);
        let images = self.screenshot_images(cx);
        let sid = self.sid.clone();
        let request_text = text.trim().to_string();
        let sent = self.store.update(cx, |s, cx| {
            s.set_rdp_desktop(&sid, Some((w, h)));
            s.set_input_text(&sid, text, cx);
            s.send_message(&sid, Some(build_rdp_agent_context(w, h)), images, cx)
        });
        if sent {
            if !(is_continuation(&request_text) && self.request.is_some()) {
                self.request = Some(request_text);
            }
            self.follow_ups = 0;
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
        }
    }

    /// Run one action's operations; resolves once they have been played, the screen has settled and (when enabled)
    /// the result has been checked against the screen.
    fn run_action(&mut self, action_id: String, cx: &mut Context<Self>) -> Task<StepOutcome> {
        let sid = self.sid.clone();
        let Some(action) = self.store.update(cx, |s, cx| s.begin_rdp_approval(&sid, &action_id, cx)) else { return Task::ready(StepOutcome::Failed) };
        let Some(ops) = action.rdp_operations.clone().filter(|o| !o.is_empty()) else { return Task::ready(StepOutcome::Failed) };
        let Some(win) = self.win.upgrade() else { return Task::ready(StepOutcome::Failed) };
        self.running.insert(action_id.clone());
        self.verdicts.remove(&action_id);
        self.batch_stopped = false;
        // the goal mode already looks at the screen after every round
        let verify = self.verify_steps && !self.goal.active;
        let before = if verify { win.read(cx).screenshot() } else { None };
        let (description, expected) = (action.description.clone(), action.expected_outcome.clone());
        let op_lines: Vec<String> = ops.iter().map(describe_operation).collect();
        cx.notify();
        let exec = win.update(cx, |w, cx| w.execute_operations(ops, cx));
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let result = exec.await;
            let ok = result.is_ok();
            let after = this
                .update(cx, |this, cx| {
                    this.running.remove(&action_id);
                    if ok {
                        let sid = this.sid.clone();
                        this.store.update(cx, |s, cx| s.mark_executed(&sid, &action_id, cx));
                        if verify {
                            this.verdicts.insert(action_id.clone(), Verdict::Checking);
                        }
                    } else if let Err(e) = &result {
                        log::warn!("rdp ai execution failed: {e}");
                    }
                    cx.notify();
                    this.win.upgrade().and_then(|w| w.read(cx).screenshot())
                })
                .ok()
                .flatten();
            if !ok {
                return StepOutcome::Failed;
            }
            if !verify {
                return StepOutcome::Done;
            }
            let verdict = match after {
                None => Verdict::Skipped("没有可用的截图".to_string()),
                Some(after) => {
                    let images: Vec<ChatMessageImage> = before.into_iter().chain(std::iter::once(after)).map(|data_url| ChatMessageImage { data_url }).collect();
                    let request = ChatRequest {
                        messages: vec![
                            ChatMessage { role: "system".into(), content: verification_system_prompt(), images: None },
                            ChatMessage { role: "user".into(), content: build_verification_prompt(&description, &op_lines, expected.as_deref()), images: Some(images) },
                        ],
                        model: None,
                        temperature: Some(0.0),
                        max_tokens: Some(400),
                    };
                    let reply = ops_core::rt::run(async move { ai_chat(&ctx.db, request).await }).await.and_then(|r| r);
                    match reply.and_then(|r| parse_verdict(&r.content)) {
                        Ok(v) if v.ok => Verdict::Passed(v.observation),
                        Ok(v) => Verdict::Failed { observation: v.observation, reason: v.reason },
                        Err(e) => Verdict::Skipped(e),
                    }
                }
            };
            let mistake = matches!(verdict, Verdict::Failed { .. });
            let _ = this.update(cx, |this, cx| {
                this.verdicts.insert(action_id.clone(), verdict);
                cx.notify();
            });
            if mistake {
                StepOutcome::Mistake
            } else {
                StepOutcome::Done
            }
        })
    }

    /// Ask the model to correct a step whose result did not match: it gets the current screen and what was wrong.
    pub fn fix_step(&mut self, action_id: &str, cx: &mut Context<Self>) {
        if self.streaming(cx) {
            return;
        }
        let Some(action) = self.rdp_actions(cx).into_iter().find(|a| a.id == action_id) else { return };
        let Some(Verdict::Failed { observation, reason }) = self.verdicts.get(action_id).cloned() else { return };
        let (w, h) = self.desktop(cx);
        let images = self.screenshot_images(cx);
        let sid = self.sid.clone();
        let text = format!("刚才执行的步骤“{}”没有达到预期。画面上看到：{observation}。问题：{reason}。请根据当前画面给出修正的 <RDP_PLAN>，只给出下一步需要的操作。", action.description);
        self.store.update(cx, |s, cx| {
            s.set_rdp_desktop(&sid, Some((w, h)));
            s.send_direct_message(&sid, text, Some(build_rdp_agent_context(w, h)), images, cx);
        });
    }

    pub fn verify_steps(&self) -> bool {
        self.verify_steps
    }

    pub fn set_verify_steps(&mut self, on: bool, cx: &mut Context<Self>) {
        self.verify_steps = on;
        cx.notify();
    }

    pub fn verdict(&self, action_id: &str) -> Option<&Verdict> {
        self.verdicts.get(action_id)
    }

    pub fn batch_stopped(&self) -> bool {
        self.batch_stopped
    }

    pub fn approve(&mut self, action_id: &str, cx: &mut Context<Self>) {
        if self.running.contains(action_id) || self.batch_id.as_deref() == Some(action_id) || self.streaming(cx) {
            return;
        }
        let task = self.run_action(action_id.to_string(), cx);
        self.tasks.push(cx.spawn(async move |this, cx| {
            if task.await == StepOutcome::Done {
                let _ = this.update(cx, |this, cx| this.continue_after_steps(cx));
            }
        }));
    }

    /// After the pending steps have all run (and passed the check): show the model the new screen and ask for the
    /// next step, so a request that needs several steps keeps going until the model says it is done.
    fn continue_after_steps(&mut self, cx: &mut Context<Self>) {
        if !self.auto_continue || self.goal.active || self.batch_id.is_some() || !self.running.is_empty() || self.streaming(cx) || self.follow_ups >= FOLLOW_UP_MAX_ROUNDS {
            return;
        }
        let Some(request) = self.request.clone() else { return };
        let actions = self.rdp_actions(cx);
        if actions.iter().any(|a| !a.executed) {
            return; // steps are still waiting for approval
        }
        // everything executed since the model's last plan message
        let latest: Vec<ParsedPendingAction> = self
            .store
            .read(cx)
            .session(&self.sid)
            .and_then(|s| s.messages.iter().rev().find(|m| m.pending_actions.iter().any(|a| a.kind == ActionKind::RdpAction)).map(|m| m.pending_actions.iter().filter(|a| a.kind == ActionKind::RdpAction && a.executed).cloned().collect()))
            .unwrap_or_default();
        let done: Vec<String> = latest.iter().map(|a| a.description.clone()).collect();
        let note = latest.iter().rev().find_map(|a| match self.verdicts.get(&a.id) {
            Some(Verdict::Passed(obs)) if !obs.is_empty() => Some(obs.clone()),
            _ => None,
        });
        if done.is_empty() {
            return;
        }
        self.follow_ups += 1;
        let (w, h) = self.desktop(cx);
        let prompt = follow_up_prompt(&request, &done, note.as_deref().unwrap_or(""));
        let images = self.screenshot_images(cx);
        let sid = self.sid.clone();
        self.store.update(cx, |s, cx| {
            s.set_rdp_desktop(&sid, Some((w, h)));
            s.send_direct_message(&sid, prompt, Some(build_rdp_agent_context(w, h)), images, cx);
        });
    }

    pub fn auto_continue(&self) -> bool {
        self.auto_continue
    }

    pub fn set_auto_continue(&mut self, on: bool, cx: &mut Context<Self>) {
        self.auto_continue = on;
        cx.notify();
    }

    pub fn follow_up_rounds(&self) -> u32 {
        self.follow_ups
    }

    pub fn reject(&mut self, action_id: &str, cx: &mut Context<Self>) {
        let sid = self.sid.clone();
        self.store.update(cx, |s, cx| s.reject_action(&sid, action_id, cx));
    }

    /// "Run all": the pending steps one after another; a failing step does not stop the queue.
    pub fn approve_all(&mut self, cx: &mut Context<Self>) {
        let queue: Vec<String> = self.rdp_actions(cx).into_iter().filter(|a| !a.executed).map(|a| a.id).collect();
        if queue.is_empty() || self.batch_id.is_some() {
            return;
        }
        self.tasks.push(cx.spawn(async move |this, cx| {
            for id in queue {
                let Ok(task) = this.update(cx, |this, cx| {
                    this.batch_id = Some(id.clone());
                    cx.notify();
                    this.run_action(id, cx)
                }) else {
                    return;
                };
                // a step that went wrong stops the queue: the later steps were planned for a screen that is not there
                if task.await == StepOutcome::Mistake {
                    let _ = this.update(cx, |this, cx| {
                        this.batch_stopped = true;
                        cx.notify();
                    });
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.batch_id = None;
                cx.notify();
                if !this.batch_stopped {
                    this.continue_after_steps(cx);
                }
            });
        }));
    }

    // ---- goal mode ----

    pub fn set_goal_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        self.goal_mode = on;
        if !on {
            self.stop_goal(cx);
        }
        cx.notify();
    }

    pub fn start_goal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let target = self.goal_input.read(cx).value().trim().to_string();
        if target.is_empty() || self.streaming(cx) || self.goal.active {
            return;
        }
        self.goal = Goal { active: true, target: target.clone(), round: 0, busy: false };
        self.send_goal_message(&target, 0, false, cx);
        cx.notify();
    }

    pub fn stop_goal(&mut self, cx: &mut Context<Self>) {
        self.goal.active = false;
        self.goal.busy = false;
        cx.notify();
    }

    fn send_goal_message(&mut self, target: &str, round: u32, check: bool, cx: &mut Context<Self>) {
        let (w, h) = self.desktop(cx);
        let prompt = goal_prompt(target, round, check, w, h);
        let images = self.screenshot_images(cx);
        let sid = self.sid.clone();
        self.store.update(cx, |s, cx| {
            s.set_rdp_desktop(&sid, Some((w, h)));
            s.send_direct_message(&sid, prompt, Some(build_rdp_agent_context(w, h)), images, cx);
        });
    }

    /// Called on every store change: run new plans automatically, or finish when the model stops proposing.
    fn drive_goal(&mut self, cx: &mut Context<Self>) {
        if !self.goal.active || self.goal.busy || self.streaming(cx) {
            return;
        }
        let pending: Vec<String> = self.rdp_actions(cx).into_iter().filter(|a| !a.executed).map(|a| a.id).collect();
        if !pending.is_empty() {
            self.goal.busy = true;
            self.tasks.push(cx.spawn(async move |this, cx| {
                for id in pending {
                    let Ok(Some(task)) = this.update(cx, |this, cx| {
                        if !this.goal.active {
                            return None;
                        }
                        this.batch_id = Some(id.clone());
                        cx.notify();
                        Some(this.run_action(id, cx))
                    }) else {
                        break;
                    };
                    task.await;
                }
                let _ = this.update(cx, |this, cx| {
                    this.batch_id = None;
                    cx.notify();
                });
                // Let the remote desktop react before looking at it again.
                cx.background_executor().timer(GOAL_SETTLE).await;
                let _ = this.update(cx, |this, cx| {
                    this.goal.busy = false;
                    if !this.goal.active {
                        return;
                    }
                    this.goal.round += 1;
                    if this.goal.round >= GOAL_MAX_ROUNDS {
                        this.goal.active = false;
                    } else {
                        let (target, round) = (this.goal.target.clone(), this.goal.round);
                        this.send_goal_message(&target, round, true, cx);
                    }
                    cx.notify();
                });
            }));
            return;
        }
        // A finished reply without any new operation means the goal is reached (or the model gave up).
        let finished_without_plan = self
            .store
            .read(cx)
            .session(&self.sid)
            .and_then(|s| s.messages.last())
            .is_some_and(|m| m.role == Role::Assistant && !m.streaming && m.pending_actions.is_empty());
        if finished_without_plan {
            self.tasks.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(GOAL_FINISH_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    let still_idle = !this.streaming(cx) && !this.goal.busy && this.rdp_actions(cx).iter().all(|a| a.executed);
                    if this.goal.active && still_idle {
                        this.goal.active = false;
                        cx.notify();
                    }
                });
            }));
        }
    }

    // ---- rendering ----

    fn message(&self, ix: usize, msg: &ChatMsg, cx: &App) -> Option<AnyElement> {
        if msg.role == Role::System {
            return None;
        }
        let theme = cx.theme();
        let user = msg.role == Role::User;
        let shown = if user { msg.content.clone() } else { strip_rdp_plan_block(&msg.content) };
        let empty = shown.is_empty();
        let has_actions = !msg.pending_actions.is_empty();
        let had_plan_block = !user && msg.content.to_lowercase().contains("<rdp_plan");
        let parse_failed = !user && !msg.streaming && !has_actions && (msg.rdp_parse_error.is_some() || (had_plan_block && empty));
        Some(
            v_flex()
                .id(ElementId::Name(format!("rdp-ai-msg-{ix}").into()))
                .test_support()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .when(user, |d| d.bg(theme.secondary))
                .child(div().text_size(px(10.)).text_color(theme.muted_foreground).child(if user { "我" } else { "AI" }))
                .when(!empty, |d| d.child(div().text_size(px(12.)).child(shown)))
                .when(empty && msg.streaming, |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(format!("{}…", t!("rdp.aiThinking")))))
                .when(empty && !msg.streaming && has_actions, |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("rdp.aiPlanReady"))))
                .when(parse_failed, |d| {
                    d.child(
                        v_flex()
                            .id(ElementId::Name(format!("rdp-ai-parse-error-{ix}").into()))
                            .test_support()
                            .text_size(px(11.))
                            .text_color(theme.warning)
                            .child(format!("⚠ {}", t!("rdp.aiParseFailed")))
                            .when_some(msg.rdp_parse_error.clone(), |d, e| d.child(div().text_color(theme.muted_foreground).child(e))),
                    )
                })
                .into_any_element(),
        )
    }

    fn action_card(&self, ix: usize, a: &ParsedPendingAction, streaming: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let running = self.running.contains(&a.id) || self.batch_id.as_deref() == Some(a.id.as_str());
        let (approve_id, reject_id) = (a.id.clone(), a.id.clone());
        v_flex()
            .id(ElementId::Name(format!("rdp-ai-action-{ix}").into()))
            .test_support()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .when(a.executed || a.rejected, |d| d.opacity(0.7))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().px_1().rounded(px(3.)).bg(theme.primary).text_color(theme.primary_foreground).text_size(px(9.)).child("RDP"))
                    .child(div().min_w_0().text_size(px(12.)).child(a.description.clone())),
            )
            .children(a.rdp_operations.iter().flatten().map(|op| div().pl_2().text_size(px(11.)).text_color(theme.muted_foreground).child(format!("• {}", describe_operation(op)))))
            .when_some(a.intent.clone(), |d, intent| {
                d.child(h_flex().gap_2().text_size(px(10.)).text_color(theme.muted_foreground).child(div().px_1().rounded(px(3.)).bg(theme.muted).child(intent)).when_some(a.expected_outcome.clone().filter(|e| !e.is_empty()), |d, e| d.child(e)))
            })
            .when_some(self.verdicts.get(&a.id).cloned(), |d, verdict| {
                let fix_id = a.id.clone();
                d.child(match verdict {
                    Verdict::Checking => div().id(ElementId::Name(format!("rdp-ai-verdict-{ix}").into())).test_support().text_size(px(11.)).text_color(theme.primary).child(format!("● {}", t!("rdp.aiVerifying"))).into_any_element(),
                    Verdict::Passed(obs) => div().id(ElementId::Name(format!("rdp-ai-verdict-{ix}").into())).test_support().text_size(px(11.)).text_color(theme.success).child(format!("✓ {}{}", t!("rdp.aiVerified"), if obs.is_empty() { String::new() } else { format!("：{obs}") })).into_any_element(),
                    Verdict::Skipped(why) => div().id(ElementId::Name(format!("rdp-ai-verdict-{ix}").into())).test_support().text_size(px(11.)).text_color(theme.muted_foreground).child(format!("{}：{why}", t!("rdp.aiVerifySkipped"))).into_any_element(),
                    Verdict::Failed { observation, reason } => v_flex()
                        .id(ElementId::Name(format!("rdp-ai-verdict-{ix}").into()))
                        .test_support()
                        .gap_1()
                        .child(div().text_size(px(11.)).text_color(theme.danger).child(format!("✗ {}：{}", t!("rdp.aiVerifyFailed"), if reason.is_empty() { observation.clone() } else { reason })))
                        .when(!observation.is_empty(), |d| d.child(div().text_size(px(10.)).text_color(theme.muted_foreground).child(observation)))
                        .child(Button::new(ElementId::Name(format!("rdp-ai-fix-{ix}").into())).small().disabled(streaming).label(t!("rdp.aiFix")).on_click(cx.listener(move |this, _, _, cx| this.fix_step(&fix_id, cx))))
                        .into_any_element(),
                })
            })
            .child(if a.executed {
                div().text_size(px(11.)).text_color(theme.success).child(format!("✓ {}", t!("ai.executed"))).into_any_element()
            } else if a.rejected {
                div().text_size(px(11.)).text_color(theme.muted_foreground).child(format!("✗ {}", t!("ai.rejectTitle"))).into_any_element()
            } else if running {
                div().text_size(px(11.)).text_color(theme.primary).child(format!("● {}", t!("ai.executing"))).into_any_element()
            } else {
                h_flex()
                    .gap_2()
                    .child(Button::new(ElementId::Name(format!("rdp-ai-approve-{ix}").into())).primary().small().label(format!("✓ {}", t!("ai.approve"))).disabled(streaming).on_click(cx.listener(move |this, _, _, cx| this.approve(&approve_id, cx))))
                    .child(Button::new(ElementId::Name(format!("rdp-ai-reject-{ix}").into())).small().label(format!("✗ {}", t!("ai.reject"))).on_click(cx.listener(move |this, _, _, cx| this.reject(&reject_id, cx))))
                    .into_any_element()
            })
            .into_any_element()
    }
}

impl Render for RdpAiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (messages, active_conv) = self.store.read(cx).session(&self.sid).map(|s| (s.messages.clone(), s.active_conversation_id.clone())).unwrap_or_default();
        let conversations = self.store.read(cx).conversations_of(&self.sid).to_vec();
        let streaming = self.streaming(cx);
        let actions = self.rdp_actions(cx);
        let pending = actions.iter().filter(|a| !a.executed).count();
        let (w, h) = self.desktop(cx);
        let has_session = self.win.upgrade().is_some_and(|win| win.read(cx).session_id().is_some());
        let has_text = !self.input.read(cx).value().trim().is_empty();
        let batch_running = self.batch_id.is_some();
        let sid = self.sid.clone();

        let examples: [(&'static str, String, &'static str); 3] = [
            ("rdp-ai-example-notepad", t!("rdp.aiExampleNotepad"), "打开记事本并输入\"你好世界\""),
            ("rdp-ai-example-cmd", t!("rdp.aiExampleCmd"), "打开开始菜单，搜索 cmd 并启动命令提示符"),
            ("rdp-ai-example-screenshot", t!("rdp.aiExampleScreenshot"), "截图当前桌面（Win+Shift+S）"),
        ];

        v_flex()
            .id("rdp-ai-panel")
            .test_support()
            .size_full()
            .child(
                h_flex()
                    .h(px(30.))
                    .flex_none()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(t!("rdp.aiTitle")))
                    .when(has_session, |d| d.child(div().text_size(px(10.)).text_color(theme.muted_foreground).child(format!("{w}×{h}"))))
                    .child(Button::new("rdp-ai-goal-toggle").ghost().xsmall().label("🎯").tooltip(t!("rdp.aiGoalMode")).selected(self.goal_mode).on_click(cx.listener(|this, _, _, cx| {
                        let on = !this.goal_mode;
                        this.set_goal_mode(on, cx);
                    })))
                    .child(Button::new("rdp-ai-new").ghost().xsmall().icon(Icon::new(IconName::Plus)).tooltip(t!("ai.newChat")).on_click(cx.listener(|this, _, _, cx| {
                        let sid = this.sid.clone();
                        this.store.update(cx, |s, cx| s.new_conversation(&sid, cx));
                    })))
                    .child(Button::new("rdp-ai-history-toggle").ghost().xsmall().icon(Icon::new(IconName::Menu)).tooltip(t!("ai.chatHistory")).selected(self.show_history).on_click(cx.listener(|this, _, _, cx| {
                        this.show_history = !this.show_history;
                        cx.notify();
                    })))
                    .child(Button::new("rdp-ai-close").ghost().xsmall().icon(Icon::new(IconName::X)).tooltip(t!("rdp.aiClose")).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(win) = this.win.upgrade() {
                            win.update(cx, |w, cx| w.set_ai_open(false, cx));
                        }
                    }))),
            )
            .when(self.show_history, |d| {
                d.child(
                    v_flex()
                        .id("rdp-ai-history")
                        .test_support()
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .border_b_1()
                        .border_color(theme.border)
                        .when(conversations.is_empty(), |d| d.child(div().p_2().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("ai.noHistory"))))
                        .children(conversations.iter().enumerate().map(|(i, c)| {
                            let (open_id, del_id, open_sid, del_sid) = (c.id.clone(), c.id.clone(), sid.clone(), sid.clone());
                            h_flex()
                                .id(ElementId::Name(format!("rdp-ai-history-item-{i}").into()))
                                .test_support()
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .cursor_pointer()
                                .when(active_conv.as_deref() == Some(c.id.as_str()), |d| d.bg(theme.accent))
                                .hover(|d| d.bg(theme.accent))
                                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(if c.title.is_empty() { t!("ai.noTitle") } else { c.title.clone() }))
                                .child(Button::new(ElementId::Name(format!("rdp-ai-history-del-{i}").into())).ghost().xsmall().icon(Icon::new(IconName::X)).on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let id = del_id.clone();
                                    this.store.update(cx, |s, cx| s.delete_conversation(&del_sid, id, cx));
                                })))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = open_id.clone();
                                    this.store.update(cx, |s, cx| s.open_conversation(&open_sid, id, cx));
                                    this.show_history = false;
                                    cx.notify();
                                }))
                        })),
                )
            })
            .when(self.goal_mode, |d| {
                d.child(
                    h_flex()
                        .id("rdp-ai-goal-bar")
                        .test_support()
                        .gap_2()
                        .p_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(if self.goal.active {
                            h_flex()
                                .flex_1()
                                .gap_2()
                                .items_center()
                                .child(div().text_size(px(12.)).child(format!("🎯 {}", t!("rdp.aiGoalRunning"))))
                                .child(div().id("rdp-ai-goal-round").test_support().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("rdp.aiGoalRound", round = self.goal.round, max = GOAL_MAX_ROUNDS)))
                                .child(div().flex_1())
                                .child(Button::new("rdp-ai-goal-stop").small().danger().label(format!("⏹ {}", t!("rdp.aiGoalStop"))).on_click(cx.listener(|this, _, _, cx| this.stop_goal(cx))))
                                .into_any_element()
                        } else {
                            h_flex()
                                .flex_1()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(Input::new(&self.goal_input).small()))
                                .child(Button::new("rdp-ai-goal-start").primary().small().label(format!("▶ {}", t!("rdp.aiGoalStart"))).disabled(streaming || self.goal_input.read(cx).value().trim().is_empty()).on_click(cx.listener(|this, _, window, cx| this.start_goal(window, cx))))
                                .into_any_element()
                        }),
                )
            })
            .child(if messages.is_empty() {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .p_3()
                    .child(div().text_size(px(26.)).child("🖥️"))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("rdp.aiEmptyTitle")))
                    .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("rdp.aiEmptyHint")))
                    .children(examples.into_iter().map(|(id, label, text)| {
                        let sid = sid.clone();
                        Button::new(id).small().label(label).on_click(cx.listener(move |this, _, window, cx| {
                            this.input.update(cx, |i, cx| i.set_value(text.to_string(), window, cx));
                            this.store.update(cx, |s, cx| s.set_input_text(&sid, text.to_string(), cx));
                        }))
                    }))
                    .into_any_element()
            } else {
                v_flex()
                    .id("rdp-ai-messages")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .p_2()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(messages.iter().enumerate().filter_map(|(i, m)| self.message(i, m, cx)))
                    .into_any_element()
            })
            .when(!actions.is_empty(), |d| {
                d.child(
                    v_flex()
                        .id("rdp-ai-actions")
                        .test_support()
                        .flex_none()
                        .max_h(px(260.))
                        .border_t_1()
                        .border_color(theme.border)
                        .child(
                            h_flex()
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(t!("rdp.aiActions")))
                                .child(
                                    Checkbox::new("rdp-ai-verify")
                                        .label(t!("rdp.aiVerify"))
                                        .checked(self.verify_steps)
                                        .tooltip(t!("rdp.aiVerifyHint"))
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_verify_steps(*checked, cx))),
                                )
                                .child(
                                    Checkbox::new("rdp-ai-continue")
                                        .label(t!("rdp.aiAutoContinue"))
                                        .checked(self.auto_continue)
                                        .tooltip(t!("rdp.aiAutoContinueHint"))
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_auto_continue(*checked, cx))),
                                )
                                .when(self.batch_stopped, |d| d.child(div().id("rdp-ai-batch-stopped").test_support().text_size(px(11.)).text_color(theme.warning).child(format!("⚠ {}", t!("rdp.aiBatchStopped")))))
                                .when(pending > 0, |d| d.child(div().id("rdp-ai-badge").test_support().px_1p5().rounded_full().bg(theme.primary).text_color(theme.primary_foreground).text_size(px(10.)).child(pending.to_string())))
                                .when(pending > 1 && !batch_running, |d| d.child(Button::new("rdp-ai-run-all").small().label(format!("▶ {}", t!("rdp.aiRunAll", count = pending))).on_click(cx.listener(|this, _, _, cx| this.approve_all(cx)))))
                                .when(batch_running, |d| d.child(div().id("rdp-ai-batch-running").test_support().text_size(px(11.)).text_color(theme.primary).child(format!("● {}", t!("rdp.aiBatchRunning"))))),
                        )
                        .child(v_flex().id("rdp-ai-actions-list").flex_1().min_h_0().gap_2().p_2().overflow_y_scroll().children(actions.iter().enumerate().map(|(i, a)| self.action_card(i, a, streaming, cx)))),
                )
            })
            .child({
                // `.rdp-ai-input-bar`: 10px padding, 6px gap, bottom aligned; the input is a 36px+ box with the soft
                // background, the send button a 36x36 green square with a ➤ glyph
                let ui = crate::theme::ui(cx);
                let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
                let green: Hsla = rgb(0x35b779).into();
                let off = streaming || !has_text;
                h_flex()
                    .flex_none()
                    .gap(px(6.))
                    .p(px(10.))
                    .items_end()
                    .border_t_1()
                    .border_color(ui.border)
                    .bg(ui.card)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h(px(36.))
                            .px(px(6.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if focused { green } else { ui.border })
                            .bg(if focused { ui.card } else { ui.main })
                            .child(Textarea::new(&self.input).xsmall().appearance(false).text_size(px(12.)).line_height(relative(1.4))),
                    )
                    .child(
                        div()
                            .id("rdp-ai-send")
                            .test_support()
                            .flex_none()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(green)
                            .text_color(white())
                            .text_size(px(16.))
                            .when(off, |d| d.opacity(0.5))
                            .when(!off, |d| d.cursor_pointer().hover(|d| d.bg(rgb(0x2a9d63))))
                            .child(if streaming { "…" } else { "➤" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !off {
                                    this.send(window, cx);
                                }
                            })),
                    )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{build_rdp_agent_context, describe_operation, goal_prompt, RdpOperation};

    #[test]
    fn operations_are_described_for_the_action_card() {
        let d = |op: RdpOperation| describe_operation(&op);
        assert_eq!(d(RdpOperation::Click { x: 1, y: 2, button: 0, double_click: false }), "左键点击 (1, 2)");
        assert_eq!(d(RdpOperation::Click { x: 1, y: 2, button: 2, double_click: false }), "右键点击 (1, 2)");
        assert_eq!(d(RdpOperation::Click { x: 1, y: 2, button: 0, double_click: true }), "双击 (1, 2)");
        assert_eq!(d(RdpOperation::Drag { from_x: 1, from_y: 2, to_x: 3, to_y: 4, button: 0 }), "拖拽 (1, 2) → (3, 4)");
        assert_eq!(d(RdpOperation::Scroll { x: 5, y: 6, delta: -3, vertical: true }), "垂直滚动 ↓ 3 (5, 6)");
        assert_eq!(d(RdpOperation::Key { keys: vec!["Control".into(), "c".into()] }), "按键: Control + c");
        assert_eq!(d(RdpOperation::Wait { ms: 1500 }), "等待 1.5 秒");
        assert!(d(RdpOperation::Type { text: "x".repeat(60) }).ends_with("..."));
    }

    #[test]
    fn the_agent_prompt_states_resolution_and_the_keyboard_first_rule() {
        let p = build_rdp_agent_context(1280, 720);
        assert!(p.contains("1280x720"));
        assert!(p.contains("启动或打开应用") && p.contains("Win/Meta") && p.contains("不要优先依赖桌面图标坐标双击"));
        assert!(p.contains("<RDP_PLAN>") && p.contains("</RDP_PLAN>"));
    }

    #[test]
    fn goal_prompts() {
        assert!(goal_prompt("装 Edge", 0, false, 1280, 720).contains("1280x720"));
        let check = goal_prompt("装 Edge", 2, true, 1280, 720);
        assert!(check.contains("第 2 轮") && check.contains("目标已完成"));
    }
}
