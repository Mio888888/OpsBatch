//! Batch command execution window. Port of `pages/Commands/CommandsPage.tsx`.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use ops_ai::client::{ai_assess_action, AiActionAssessment, AiActionRiskLevel};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, TitleBar, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_ssh::execution::{ExecutionHistory, HistoryDetail};
use ops_terminal::danger::{check_dangerous_command, compile_danger_rules, default_compiled_danger_rules, CompiledDangerRule};

use super::player::AsciinemaPlayer;
use super::{apply_output, apply_start, mark_cancelled, reset_for_retry, stats, DoneEvent, HostRun, OutputEvent, Status};
use crate::library::exec;
use crate::library::store::{library_store, LibraryStore};
use crate::stores::assets::AssetsStore;
use crate::widgets::toast;
use crate::window::{self, WindowKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Execute,
    History,
}

pub struct BatchCommandsWindow {
    assets: Entity<AssetsStore>,
    library: Entity<LibraryStore>,
    pub command: Entity<TextareaState>,
    concurrency: Entity<InputState>,
    timeout: Entity<InputState>,
    runs: Vec<HostRun>,
    active_task: Option<String>,
    executing: bool,
    tab: Tab,
    history: Vec<ExecutionHistory>,
    danger_rules: Vec<CompiledDangerRule>,
    last_params: (Vec<String>, u32, u64),
    risk: Option<AiActionAssessment>,
    risk_loading: bool,
    run_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl BatchCommandsWindow {
    pub fn open(initial_command: Option<String>, cx: &mut App) {
        let options = window::window_options(WindowKind::BatchCommands, &t!("commands.title"), cx);
        window::open_or_focus("batch-commands", options, cx, move |window, cx| cx.new(|cx| Self::new(initial_command, window, cx)));
    }

    pub fn new(initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let assets = cx.global::<crate::ui_state::UiState>().assets.clone();
        let library = library_store(cx);
        let command = cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("commands.inputPlaceholder")).auto_grow(3, 8));
        if let Some(c) = initial {
            command.update(cx, |s, cx| s.set_value(c, window, cx));
        }
        let concurrency = cx.new(|cx| InputState::new(window, cx).default_value("10"));
        let timeout = cx.new(|cx| InputState::new(window, cx).default_value("30"));
        let subs = vec![
            cx.observe(&assets, |_, _, cx| cx.notify()),
            cx.observe(&command, |_, _, cx| cx.notify()),
            cx.subscribe(&command, |this, _, ev: &InputEvent, _| {
                if let InputEvent::Change = ev {
                    this.risk = None;
                }
            }),
        ];
        assets.update(cx, |a, cx| a.reload(cx));
        library.update(cx, |l, cx| l.load_commands(cx));
        let mut this = Self {
            assets,
            library,
            command,
            concurrency,
            timeout,
            runs: Vec::new(),
            active_task: None,
            executing: false,
            tab: Tab::Execute,
            history: Vec::new(),
            danger_rules: default_compiled_danger_rules(),
            last_params: (Vec::new(), 10, 30),
            risk: None,
            risk_loading: false,
            run_task: None,
            _subs: subs,
            _tasks: Vec::new(),
        };
        this.load_rules(cx);
        this.load_history(cx);
        this
    }

    // ---- accessors (tests) ----

    pub fn runs(&self) -> &[HostRun] {
        &self.runs
    }
    pub fn is_executing(&self) -> bool {
        self.executing
    }
    pub fn risk(&self) -> Option<&AiActionAssessment> {
        self.risk.as_ref()
    }

    /// "Assess" button: run the command through the Rust policy and show the risk level and reason.
    pub fn assess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.command.read(cx).value().trim().to_string();
        if command.is_empty() || self.risk_loading {
            return;
        }
        self.risk = None;
        self.risk_loading = true;
        let ctx = crate::services::ctx(cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { ai_assess_action(&ctx.db, command) }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.risk_loading = false;
                match result {
                    Ok(a) => this.risk = Some(a),
                    Err(e) => toast::error(window, cx, t!("commands.riskAssessmentFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn history(&self) -> &[ExecutionHistory] {
        &self.history
    }
    pub fn tab(&self) -> Tab {
        self.tab
    }
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == Tab::History {
            self.load_history(cx);
        }
        cx.notify();
    }
    pub fn set_command(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.risk = None;
        self.command.update(cx, |s, cx| s.set_value(text, window, cx));
    }
    pub fn set_limits(&mut self, concurrency: u32, timeout: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.concurrency.update(cx, |s, cx| s.set_value(concurrency.to_string(), window, cx));
        self.timeout.update(cx, |s, cx| s.set_value(timeout.to_string(), window, cx));
    }

    fn load_rules(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let rules = ops_core::rt::run(async move { ops_core::settings::list_danger_rules(&ctx.db).await }).await.and_then(|r| r);
            if let Ok(rules) = rules {
                let _ = this.update(cx, |this, _| this.danger_rules = compile_danger_rules(&rules));
            }
        }));
    }

    pub fn load_history(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let rows = ops_core::rt::run(async move { ops_ssh::execution::list_execution_history(&ctx.db).await }).await.and_then(|r| r).unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.history = rows;
                cx.notify();
            });
        }));
    }

    fn selected(&self, cx: &App) -> Vec<String> {
        self.assets.read(cx).selected.iter().cloned().collect()
    }

    fn limits(&self, cx: &App) -> (u32, u64) {
        let c = self.concurrency.read(cx).value().trim().parse::<u32>().unwrap_or(10).clamp(1, 100);
        let t = self.timeout.read(cx).value().trim().parse::<u64>().unwrap_or(30).clamp(1, 3600);
        (c, t)
    }

    // ---- execution ----

    pub fn execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.command.read(cx).value().to_string();
        if command.trim().is_empty() {
            toast::warning(window, cx, t!("commands.enterCommand"));
            return;
        }
        if self.selected(cx).is_empty() {
            toast::warning(window, cx, t!("commands.selectHostsFirst"));
            return;
        }
        let matched = check_dangerous_command(&self.danger_rules, &command);
        if matched.is_empty() {
            self.do_execute(None, window, cx);
            return;
        }
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            let (rules, shown) = (matched.clone(), command.clone());
            alert
                .confirm()
                .title(t!("commands.dangerWarning"))
                .description(format!("{} {}\n\n{}\n\n{}", t!("commands.dangerMatched"), rules.join(", "), shown, t!("commands.confirmContinue")))
                .show_cancel(true)
                .ok_text(t!("commands.confirmExecute"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.do_execute(None, window, cx));
                    true
                })
        });
    }

    /// Run on the selected hosts, or only on `only` (retry of failed hosts).
    fn do_execute(&mut self, only: Option<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.command.read(cx).value().to_string();
        let hosts = only.clone().unwrap_or_else(|| self.selected(cx));
        let (concurrency, timeout) = self.limits(cx);
        let task_id = uuid::Uuid::new_v4().to_string();
        if only.is_none() {
            let a = self.assets.read(cx);
            self.runs = hosts
                .iter()
                .map(|id| {
                    let h = a.host(id);
                    HostRun::pending(id, h.map(|h| h.name.as_str()).unwrap_or(""), h.map(|h| h.ip.as_str()).unwrap_or(""))
                })
                .collect();
        } else {
            reset_for_retry(&mut self.runs, &hosts);
        }
        self.executing = true;
        self.active_task = Some(task_id.clone());
        self.last_params = (hosts.clone(), concurrency, timeout);
        cx.notify();

        let ctx = crate::services::ctx(cx);
        let (start, output, done) = (
            ctx.events.subscribe(format!("exec:{task_id}:start")),
            ctx.events.subscribe(format!("exec:{task_id}:output")),
            ctx.events.subscribe(format!("exec:{task_id}:done")),
        );
        let count = hosts.len();
        let tid = task_id.clone();
        self.run_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Err(e) = exec::dispatch(ctx, tid, hosts, command, concurrency, timeout, None).await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.executing = false;
                    this.active_task = None;
                    toast::error(window, cx, t!("commands.executeFailed", error = e));
                    cx.notify();
                });
                return;
            }
            let _ = this.update_in(cx, |_, window, cx| toast::info(window, cx, t!("commands.dispatchedToast", count = count)));
            loop {
                futures::select_biased! {
                    ev = start.recv_async() => {
                        let Ok(ev) = ev else { break };
                        if let Some(id) = ev.payload.as_str().map(str::to_string) {
                            if this.update(cx, |this, cx| { apply_start(&mut this.runs, &id); cx.notify(); }).is_err() { break; }
                        }
                    }
                    ev = output.recv_async() => {
                        let Ok(ev) = ev else { break };
                        if let Some(o) = ev.parse::<OutputEvent>() {
                            if this.update(cx, |this, cx| { apply_output(&mut this.runs, &o); cx.notify(); }).is_err() { break; }
                        }
                    }
                    ev = done.recv_async() => {
                        let Ok(ev) = ev else { break };
                        // Outputs are delivered before `done`; drain anything still queued first.
                        while let Ok(o) = output.try_recv() {
                            if let Some(o) = o.parse::<OutputEvent>() {
                                let _ = this.update(cx, |this, _| apply_output(&mut this.runs, &o));
                            }
                        }
                        if let Some(d) = ev.parse::<DoneEvent>() {
                            let _ = this.update_in(cx, |this, window, cx| {
                                this.executing = false;
                                this.active_task = None;
                                toast::success(window, cx, t!("commands.completedToast", success = d.success_count, failed = d.fail_count, duration = d.duration));
                                this.load_history(cx);
                                cx.notify();
                            });
                        }
                        break;
                    }
                }
            }
        }));
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.active_task.take() else { return };
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |_, _| {
            let _ = ops_core::rt::run(async move { ops_ssh::execution::cancel_execution(&ctx.db, task).await }).await;
        })
        .detach();
        self.run_task = None;
        self.executing = false;
        mark_cancelled(&mut self.runs, &t!("commands.cancelled"));
        cx.notify();
    }

    pub fn retry_failed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let failed: Vec<String> = self.runs.iter().filter(|r| r.status.is_failure()).map(|r| r.host_id.clone()).collect();
        if failed.is_empty() || self.executing {
            return;
        }
        let n = failed.len();
        self.do_execute(Some(failed), window, cx);
        toast::info(window, cx, t!("commands.reExecuteFailedHosts", count = n));
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.run_task = None;
        self.runs.clear();
        self.active_task = None;
        self.executing = false;
        cx.notify();
    }

    // ---- dialogs ----

    fn pick_from_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let library = self.library.clone();
        let view = cx.new(|cx| LibraryPicker::new(library, weak, window, cx));
        window.open_dialog(cx, move |d, _, _| d.title(t!("batchExec.selectFromLibraryTitle")).w(px(600.)).child(view.clone()));
    }

    fn replay(&mut self, history_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, id) = (crate::services::ctx(cx), history_id);
        cx.spawn_in(window, async move |_, cx| {
            let details = ops_core::rt::run(async move { ops_ssh::execution::get_execution_detail(&ctx.db, id).await }).await.and_then(|r| r);
            let _ = cx.update(|window, cx| match details {
                Ok(d) => {
                    let view = cx.new(|_| ReplayDialog { details: d });
                    window.open_dialog(cx, move |dlg, _, _| dlg.title(t!("batchExec.replayTitle")).w(px(800.)).child(view.clone()));
                }
                Err(e) => toast::error(window, cx, e),
            });
        })
        .detach();
    }

    // ---- rendering ----

    fn output_text(r: &HostRun, theme: &gpui_kit::component::Theme) -> AnyElement {
        if r.output.is_empty() {
            let msg = match r.status {
                Status::Pending => t!("commands.waitingExecute"),
                Status::Running => t!("commands.runningOutput"),
                _ => t!("common.noOutput"),
            };
            return div().text_color(theme.muted_foreground).child(msg).into_any_element();
        }
        let a = crate::ansi::parse(&r.output);
        let runs: Vec<(std::ops::Range<usize>, HighlightStyle)> = a.spans.iter().map(|(g, c)| (g.clone(), HighlightStyle { color: Some(rgb(*c).into()), ..Default::default() })).collect();
        StyledText::new(a.text).with_highlights(runs).into_any_element()
    }

    fn execute_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (total, ok, failed, running) = stats(&self.runs);
        let selected = self.selected(cx);
        let names: Vec<String> = {
            let a = self.assets.read(cx);
            selected.iter().filter_map(|id| a.host(id)).map(|h| if h.name.is_empty() { h.ip.clone() } else { h.name.clone() }).collect()
        };
        let has_command = !self.command.read(cx).value().trim().is_empty();
        let stat = |label: String, value: usize, color: Option<Hsla>| {
            v_flex().flex_1().p_2().rounded(px(6.)).border_1().border_color(theme.border).child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(label)).child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).when_some(color, |d, c| d.text_color(c)).child(value.to_string()))
        };
        let cards: Vec<AnyElement> = self
            .runs
            .iter()
            .enumerate()
            .map(|(ix, r)| {
                let (color, label) = match r.status {
                    Status::Success => (theme.success, t!(r.status.label_key())),
                    Status::Failed => (theme.danger, t!(r.status.label_key())),
                    Status::Timeout => (theme.warning, t!(r.status.label_key())),
                    Status::Running => (theme.primary, t!(r.status.label_key())),
                    Status::Pending => (theme.muted_foreground, t!(r.status.label_key())),
                };
                v_flex()
                    .id(("bx-result", ix))
                    .test_support()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_3()
                            .px_3()
                            .py_1p5()
                            .items_center()
                            .child(div().w(px(180.)).truncate().font_weight(FontWeight::MEDIUM).child(r.display_name().to_string()))
                            .child(h_flex().gap_1().items_center().text_color(color).child(div().size(px(6.)).rounded_full().bg(color)).child(label))
                            .child(div().flex_1())
                            .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(r.duration.map(|d| format!("{d}ms")).unwrap_or_else(|| "-".into()))),
                    )
                    .child(div().id(("bx-output", ix)).max_h(px(200.)).overflow_y_scroll().mx_3().mb_2().p_2().rounded(px(6.)).bg(theme.muted).font_family("Menlo").text_size(px(12.)).child(Self::output_text(r, &theme)))
                    .into_any_element()
            })
            .collect();

        v_flex()
            .gap_3()
            .p_3()
            .size_full()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(Textarea::new(&self.command))
                            .when_some(self.risk.clone(), |d, a| {
                                let color = match a.risk_level {
                                    AiActionRiskLevel::Low => theme.success,
                                    AiActionRiskLevel::Medium => theme.warning,
                                    AiActionRiskLevel::High | AiActionRiskLevel::Critical => theme.danger,
                                };
                                d.child(
                                    h_flex()
                                        .id("bx-risk")
                                        .test_support()
                                        .gap_2()
                                        .items_center()
                                        .text_size(px(12.))
                                        .child(div().px_1p5().rounded(px(3.)).border_1().border_color(color).text_color(color).child(t!("commands.riskLevel", level = t!(&format!("ai.risk.{}", format!("{:?}", a.risk_level).to_lowercase())))))
                                        .child(div().text_color(theme.muted_foreground).child(a.reason)),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(h_flex().gap_2().items_center().child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("commands.concurrency"))).child(div().w(px(70.)).child(crate::widgets::form::spin(Input::new(&self.concurrency).small(), "bx-concurrency", &self.concurrency, 1, Some(100), crate::theme::ui(cx)))))
                            .child(h_flex().gap_2().items_center().child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("commands.timeout"))).child(div().w(px(70.)).child(crate::widgets::form::spin(Input::new(&self.timeout).small(), "bx-timeout", &self.timeout, 1, Some(3600), crate::theme::ui(cx)))).child(div().text_size(px(12.)).child(t!("commands.seconds"))))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Button::new("bx-assess").small().icon(Icon::new(IconName::Info)).label(t!("commands.assess")).tooltip(t!("commands.aiRiskAssessment")).loading(self.risk_loading).disabled(!has_command).on_click(cx.listener(|this, _, window, cx| this.assess(window, cx))))
                                    .child(Button::new("bx-execute").primary().small().icon(Icon::new(IconName::CirclePlay)).label(t!("commands.execute")).loading(self.executing).disabled(self.active_task.is_some() || !has_command).on_click(cx.listener(|this, _, window, cx| this.execute(window, cx))))
                                    .when(self.active_task.is_some(), |d| d.child(Button::new("bx-stop").danger().small().icon(Icon::new(IconName::Square)).label(t!("commands.stop")).on_click(cx.listener(|this, _, _, cx| this.stop(cx))))),
                            ),
                    ),
            )
            .when(!names.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("commands.targetHosts", hosts = names.join(", ")))))
            .when(total > 0, |d| {
                d.child(h_flex().id("bx-stats").test_support().gap_3().child(stat(t!("commands.total"), total, None)).child(stat(t!("commands.success"), ok, Some(theme.success))).child(stat(t!("commands.failed"), failed, Some(theme.danger))).child(stat(t!("commands.running"), running, Some(theme.primary))))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .px_3()
                            .py_1p5()
                            .justify_between()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("commands.results")))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .when(failed > 0, |d| d.child(Button::new("bx-retry").danger().xsmall().icon(Icon::new(IconName::CirclePlay)).label(t!("commands.retryFailed", count = failed)).disabled(self.executing).on_click(cx.listener(|this, _, window, cx| this.retry_failed(window, cx)))))
                                    .child(Button::new("bx-clear").xsmall().label(t!("common.clear")).on_click(cx.listener(|this, _, _, cx| this.clear(cx)))),
                            ),
                    )
                    .child(if cards.is_empty() {
                        div().p_6().flex().justify_center().text_color(theme.muted_foreground).child(t!("commands.viewResultsAfterExecute")).into_any_element()
                    } else {
                        v_flex().id("bx-results").test_support().flex_1().min_h_0().overflow_y_scroll().children(cards).into_any_element()
                    }),
            )
            .into_any_element()
    }

    fn history_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("bx-history")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .children(self.history.iter().enumerate().map(|(ix, h)| {
                let (cmd1, cmd2, id1, id2) = (h.command.clone(), h.command.clone(), h.id.clone(), h.id.clone());
                h_flex()
                    .id(("bx-history-row", ix))
                    .test_support()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().min_w_0().truncate().font_family("Menlo").text_size(px(12.)).child(cmd1))
                    .child(div().w(px(60.)).text_size(px(12.)).child(format!("{}", h.host_count)))
                    .child(h_flex().gap_1().w(px(120.)).child(div().text_color(theme.success).child(format!("{} {}", h.success_count, t!("commands.success")))).when(h.fail_count > 0, |d| d.child(div().text_color(theme.danger).child(format!("{} {}", h.fail_count, t!("commands.failed"))))))
                    .child(div().w(px(160.)).text_size(px(11.)).text_color(theme.muted_foreground).child(h.started_at.clone()))
                    .child(div().w(px(70.)).text_size(px(11.)).child(format!("{}ms", h.duration)))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Button::new(("bx-reuse", ix)).xsmall().label(t!("commands.reuse")).on_click(cx.listener(move |this, _, window, cx| {
                                this.tab = Tab::Execute;
                                this.set_command(&cmd2, window, cx);
                                toast::info(window, cx, t!("commands.commandFilled"));
                                cx.notify();
                            })))
                            .child(Button::new(("bx-replay", ix)).xsmall().label(t!("commands.tableReplay")).on_click(cx.listener(move |this, _, window, cx| this.replay(id1.clone(), window, cx))))
                            .child(Button::new(("bx-recording", ix)).ghost().xsmall().label(t!("commands.recordingReplay")).on_click(move |_, window, cx| {
                                AsciinemaPlayer::open(id2.clone(), window, cx);
                            })),
                    )
            }))
            .into_any_element()
    }
}

impl Render for BatchCommandsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let n = self.selected(cx).len();
        let body = match self.tab {
            Tab::Execute => self.execute_tab(cx),
            Tab::History => self.history_tab(cx),
        };
        v_flex()
            .id("batch-commands-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .gap_2()
                        .items_center()
                        .child(div().font_weight(FontWeight::BOLD).child(t!("commands.title")))
                        .child(div().px_2().rounded_full().bg(Hsla { a: 0.12, ..theme.primary }).text_color(theme.primary).text_size(px(11.)).child(t!("commands.selectedHosts", count = n)))
                        .child(div().flex_1())
                        .child(Button::new("bx-upload").xsmall().icon(Icon::new(IconName::Upload)).label(t!("commands.batchUpload")).disabled(n == 0).on_click(cx.listener(|this, _, _, cx| {
                            let ids = this.selected(cx);
                            crate::transfer::window::BatchTransferWindow::open(ids, cx);
                        })))
                        .child(Button::new("bx-library").xsmall().label(t!("commands.selectFromLibrary")).on_click(cx.listener(|this, _, window, cx| this.pick_from_library(window, cx)))),
                ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .px_3()
                    .pt_2()
                    .child(Button::new("bx-tab-execute").ghost().small().label(t!("commands.executeTab")).selected(self.tab == Tab::Execute).on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Execute, cx))))
                    .child(Button::new("bx-tab-history").ghost().small().label(t!("commands.historyTab")).selected(self.tab == Tab::History).on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::History, cx)))),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

// ---- library picker ----

struct LibraryPicker {
    library: Entity<LibraryStore>,
    window: WeakEntity<BatchCommandsWindow>,
    search: Entity<InputState>,
    _sub: Subscription,
}

impl LibraryPicker {
    fn new(library: Entity<LibraryStore>, window_ref: WeakEntity<BatchCommandsWindow>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("batchExec.searchLibrary")));
        let sub = cx.observe(&search, |_, _, cx| cx.notify());
        Self { library, window: window_ref, search, _sub: sub }
    }
}

impl Render for LibraryPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let q = self.search.read(cx).value().trim().to_lowercase();
        let rows: Vec<_> = self.library.read(cx).commands.iter().filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q) || c.command.to_lowercase().contains(&q)).cloned().collect();
        v_flex()
            .id("bx-library-picker")
            .test_support()
            .gap_2()
            .child(Input::new(&self.search).small())
            .child(v_flex().id("bx-library-list").max_h(px(400.)).overflow_y_scroll().gap_1().children(rows.into_iter().enumerate().map(|(ix, c)| {
                let (cmd, name) = (c.command.clone(), c.name.clone());
                h_flex()
                    .id(("bx-library-row", ix))
                    .test_support()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.list_hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let (cmd, name) = (cmd.clone(), name.clone());
                        let _ = this.window.update(cx, |w, cx| w.set_command(&cmd, window, cx));
                        toast::success(window, cx, t!("batchExec.selected", name = name));
                        window.close_dialog(cx);
                    }))
                    .child(v_flex().min_w_0().child(div().font_weight(FontWeight::MEDIUM).child(c.name.clone())).child(div().truncate().font_family("Menlo").text_size(px(11.)).text_color(theme.muted_foreground).child(c.command.clone())))
                    .child(crate::library::ui::risk_chip(c.risk, cx))
            })))
    }
}

// ---- history replay (table) ----

struct ReplayDialog {
    details: Vec<HistoryDetail>,
}

impl Render for ReplayDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("bx-replay-dialog")
            .test_support()
            .gap_2()
            .max_h(px(480.))
            .child(if self.details.is_empty() {
                div().p_6().flex().justify_center().text_color(theme.muted_foreground).child(t!("commands.noDetails")).into_any_element()
            } else {
                v_flex()
                    .id("bx-replay-list")
                    .overflow_y_scroll()
                    .children(self.details.iter().enumerate().map(|(ix, d)| {
                        let ok = d.status == "success";
                        let a = crate::ansi::parse(&d.output);
                        let runs: Vec<(std::ops::Range<usize>, HighlightStyle)> = a.spans.iter().map(|(g, c)| (g.clone(), HighlightStyle { color: Some(rgb(*c).into()), ..Default::default() })).collect();
                        v_flex()
                            .id(("bx-replay-row", ix))
                            .gap_1()
                            .py_1p5()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(div().w(px(200.)).truncate().font_weight(FontWeight::MEDIUM).child(d.host_name.clone()))
                                    .child(div().w(px(80.)).text_color(if ok { theme.success } else { theme.danger }).child(if ok { t!("commands.success") } else { t!("commands.failed") }))
                                    .child(div().w(px(90.)).text_size(px(12.)).text_color(theme.muted_foreground).child(format!("{}: {}", t!("batchExec.exitCodeCol"), d.exit_code)))
                                    .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(if d.duration > 0 { format!("{}ms", d.duration) } else { "-".into() })),
                            )
                            .child(div().max_h(px(200.)).overflow_hidden().p_2().rounded(px(6.)).bg(theme.muted).font_family("Menlo").text_size(px(12.)).child(if d.output.is_empty() { StyledText::new(t!("batchExec.noOutputText")) } else { StyledText::new(a.text).with_highlights(runs) }))
                    }))
                    .into_any_element()
            })
            .child(h_flex().justify_end().child(Button::new("bx-replay-close").label(t!("batchExec.close")).on_click(|_, window, cx| window.close_dialog(cx))))
    }
}
