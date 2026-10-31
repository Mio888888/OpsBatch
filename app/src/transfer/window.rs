//! Batch upload window. Port of `pages/Transfer/BatchTransferWindow.tsx`.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, TitleBar,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_core::pathnames::basename_from_path;
use ops_i18n::t;
use ops_ssh::hosts::Host;
use ops_ssh::transfer::TransferRequest;

use super::{format_size, merge_results, summarize, HostResult, QUICK_FILLS};
use crate::sftp::store::blocking;
use crate::widgets::toast;
use crate::window::{self, WindowKind};

const MAX_COLLAPSED_HOSTS: usize = 3;

pub struct BatchTransferWindow {
    host_ids: Vec<String>,
    hosts: Vec<Host>,
    hosts_loading: bool,
    local_path: Entity<InputState>,
    remote_path: Entity<InputState>,
    concurrency: Entity<InputState>,
    timeout: Entity<InputState>,
    transferring: bool,
    active_task: Option<String>,
    results: Vec<HostResult>,
    targets_expanded: bool,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl BatchTransferWindow {
    /// Open the window for these hosts (a previous batch window is replaced).
    pub fn open(host_ids: Vec<String>, cx: &mut App) {
        let options = window::window_options(
            WindowKind::BatchTransfer,
            &t!("assets.batchUploadWindowTitle", count = host_ids.len()),
            cx,
        );
        window::replace_window("batch-transfer", options, cx, move |window, cx| {
            cx.new(|cx| Self::new(host_ids, window, cx))
        });
    }

    pub fn new(host_ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |cx: &mut Context<Self>, window: &mut Window, ph: String, v: &str| {
            let v = v.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(ph).default_value(v))
        };
        let local_path = input(cx, window, "/path/to/local/file".into(), "");
        let remote_path = input(cx, window, t!("batchTransfer.remotePlaceholder"), "");
        let concurrency = input(cx, window, "5".into(), "5");
        let timeout = input(cx, window, "120".into(), "120");
        let subs = [&local_path, &remote_path, &concurrency, &timeout]
            .map(|i| cx.observe(i, |_, _, cx| cx.notify()))
            .into_iter()
            .collect::<Vec<_>>();

        let ctx = crate::services::ctx(cx);
        let ids = host_ids.clone();
        let load = cx.spawn(async move |this, cx| {
            let all = ops_core::rt::run(async move { ops_ssh::hosts::list_hosts(&ctx.db).await })
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.hosts_loading = false;
                match all {
                    Ok(all) => {
                        this.hosts = all.into_iter().filter(|h| ids.contains(&h.id)).collect()
                    }
                    Err(e) => log::warn!("transfer: could not load hosts: {e}"),
                }
                cx.notify();
            });
        });
        let _ = window;
        Self {
            host_ids,
            hosts: Vec::new(),
            hosts_loading: true,
            local_path,
            remote_path,
            concurrency,
            timeout,
            transferring: false,
            active_task: None,
            results: Vec::new(),
            targets_expanded: false,
            _subs: subs,
            _tasks: vec![load],
        }
    }

    // ---- accessors (tests) ----

    pub fn inputs(&self) -> (Entity<InputState>, Entity<InputState>) {
        (self.local_path.clone(), self.remote_path.clone())
    }
    pub fn results(&self) -> &[HostResult] {
        &self.results
    }
    pub fn is_transferring(&self) -> bool {
        self.transferring
    }
    pub fn hosts_loaded(&self) -> usize {
        self.hosts.len()
    }

    fn is_ready(&self, cx: &App) -> bool {
        !self.host_ids.is_empty()
            && !self.local_path.read(cx).value().trim().is_empty()
            && !self.remote_path.read(cx).value().trim().is_empty()
    }

    fn pick(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        let p = p.to_string_lossy().to_string();
                        this.local_path
                            .update(cx, |s, cx| s.set_value(p, window, cx));
                    });
                }
            }
        })
        .detach();
    }

    /// Start the upload to every target host.
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host_ids.is_empty() {
            toast::warning(window, cx, t!("batchTransfer.noTargets"));
            return;
        }
        let local = self.local_path.read(cx).value().trim().to_string();
        let remote = self.remote_path.read(cx).value().trim().to_string();
        if local.is_empty() || remote.is_empty() {
            toast::warning(window, cx, t!("batchTransfer.msgFillPaths"));
            return;
        }
        let timeout: u64 = self
            .timeout
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(120)
            .clamp(1, 3600);
        self.transferring = true;
        self.results.clear();
        let task_id = uuid::Uuid::new_v4().to_string();
        self.active_task = Some(task_id.clone());
        cx.notify();

        let ctx = crate::services::ctx(cx);
        // Listen before starting so no event is missed.
        let progress = ctx.events.subscribe(format!("transfer:{task_id}:progress"));
        let done = ctx.events.subscribe(format!("transfer:{task_id}:done"));
        let (hosts, count) = (self.host_ids.clone(), self.host_ids.len());
        let request = TransferRequest {
            task_id: Some(task_id.clone()),
            direction: "upload".into(),
            host_ids: hosts,
            local_path: local,
            remote_path: remote,
            remote_paths: None,
            timeout: Some(timeout),
        };
        let c = ctx.clone();
        cx.spawn_in(window, async move |this, cx| {
            let start = blocking(move || ops_ssh::transfer::file_transfer(c.clone(), &c.db, request)).await;
            match start {
                Ok(_) => {
                    let _ = this.update_in(cx, |_, window, cx| toast::info(window, cx, t!("batchTransfer.msgStarted", count = count)));
                }
                Err(e) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.transferring = false;
                        this.active_task = None;
                        toast::error(window, cx, t!("batchTransfer.msgFailed", error = e));
                        cx.notify();
                    });
                    return;
                }
            }
            loop {
                futures::select_biased! {
                    ev = done.recv_async() => {
                        let Ok(ev) = ev else { break };
                        let results: Vec<HostResult> = ev.payload.get("results").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default();
                        let _ = this.update_in(cx, |this, window, cx| {
                            let failed = results.iter().filter(|r| !r.success).count();
                            merge_results(&mut this.results, results);
                            this.transferring = false;
                            this.active_task = None;
                            if failed > 0 {
                                toast::error(window, cx, t!("batchTransfer.msgDoneFailed", count = failed));
                            } else {
                                toast::success(window, cx, t!("batchTransfer.msgDone"));
                            }
                            cx.notify();
                        });
                        break;
                    }
                    ev = progress.recv_async() => {
                        let Ok(ev) = ev else { break };
                        if let Some(r) = ev.parse::<HostResult>() {
                            if this.update(cx, |this, cx| { merge_results(&mut this.results, [r]); cx.notify(); }).is_err() { break; }
                        }
                    }
                }
            }
        })
        .detach();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.results.clear();
        self.active_task = None;
        self.transferring = false;
        cx.notify();
    }
}

impl Render for BatchTransferWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let total = self.host_ids.len();

        if self.hosts_loading {
            return v_flex()
                .id("batch-transfer-loading")
                .test_support()
                .size_full()
                .bg(theme.background)
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("batchTransfer.loadingTitle")),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(t!("batchTransfer.loadingSub")),
                )
                .into_any_element();
        }

        let (ok, failed, pending, percent) = summarize(&self.results, total);
        let done = ok + failed;
        let hidden = self.hosts.len().saturating_sub(MAX_COLLAPSED_HOSTS);
        let visible: Vec<Host> = if self.targets_expanded {
            self.hosts.clone()
        } else {
            self.hosts
                .iter()
                .take(MAX_COLLAPSED_HOSTS)
                .cloned()
                .collect()
        };
        let source_name = basename_from_path(&self.local_path.read(cx).value(), "");
        let ready = self.is_ready(cx);

        let strip = h_flex()
            .id("batch-transfer-targets")
            .test_support()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(t!("batchTransfer.targets")),
            )
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(self.hosts.len().to_string()),
            )
            .children(visible.into_iter().enumerate().map(|(ix, h)| {
                div()
                    .id(("batch-target", ix))
                    .px_2()
                    .rounded_full()
                    .bg(theme.muted)
                    .text_size(px(11.))
                    .child(if h.name.is_empty() {
                        h.ip.clone()
                    } else {
                        h.name.clone()
                    })
            }))
            .when(!self.targets_expanded && hidden > 0, |d| {
                d.child(
                    Button::new("batch-targets-more")
                        .ghost()
                        .xsmall()
                        .label(format!("+{hidden}"))
                        .tooltip(t!("batchTransfer.showRemaining", count = hidden))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.targets_expanded = true;
                            cx.notify();
                        })),
                )
            })
            .when(self.targets_expanded && hidden > 0, |d| {
                d.child(
                    Button::new("batch-targets-less")
                        .ghost()
                        .xsmall()
                        .label(t!("batchTransfer.collapse"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.targets_expanded = false;
                            cx.notify();
                        })),
                )
            })
            .when(self.hosts.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(t!("batchTransfer.noHosts")),
                )
            });

        let quick = h_flex()
            .gap_1()
            .flex_wrap()
            .children(QUICK_FILLS.iter().enumerate().map(|(ix, (label, value))| {
                let label = match *label {
                    "@hostName" => t!("batchTransfer.hostName"),
                    "@firstDir" => t!("batchTransfer.firstDir"),
                    other => other.to_string(),
                };
                let value = value.to_string();
                Button::new(("batch-quick", ix))
                    .ghost()
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let v = value.clone();
                        this.remote_path
                            .update(cx, |s, cx| s.set_value(v, window, cx));
                    }))
            }));

        let config = v_flex()
            .id("batch-config")
            .test_support()
            .w(px(400.))
            .flex_none()
            .h_full()
            .p_3()
            .gap_3()
            .border_r_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(t!("batchTransfer.settings")),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(source_name),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("batchTransfer.localPath")),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(div().flex_1().child(Input::new(&self.local_path).small()))
                            .child(
                                Button::new("batch-pick-file")
                                    .xsmall()
                                    .icon(Icon::new(IconName::File))
                                    .tooltip(t!("batchTransfer.pickFile"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("batch-pick-dir")
                                    .xsmall()
                                    .icon(Icon::new(IconName::FolderOpen))
                                    .tooltip(t!("batchTransfer.pickDir"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick(true, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("batchTransfer.remotePath")),
                    )
                    .child(Input::new(&self.remote_path).small())
                    .child(quick)
                    .child(
                        h_flex()
                            .gap_3()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(div().font_family("Menlo").child("{host}"))
                                    .child(t!("batchTransfer.varHost")),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(div().font_family("Menlo").child("{firstdir:path}"))
                                    .child(t!("batchTransfer.varFirstDir")),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(
                        v_flex()
                            .w(px(80.))
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child(t!("batchTransfer.concurrency")),
                            )
                            .child(crate::widgets::form::spin(
                                Input::new(&self.concurrency).small(),
                                "bt-concurrency",
                                &self.concurrency,
                                1,
                                Some(20),
                                crate::theme::ui(cx),
                            )),
                    )
                    .child(
                        v_flex()
                            .w(px(110.))
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{} ({})",
                                        t!("batchTransfer.timeout"),
                                        t!("batchTransfer.seconds")
                                    )),
                            )
                            .child(crate::widgets::form::spin(
                                Input::new(&self.timeout).small(),
                                "bt-timeout",
                                &self.timeout,
                                1,
                                Some(3600),
                                crate::theme::ui(cx),
                            )),
                    )
                    .child(div().flex_1())
                    .child(if self.active_task.is_some() {
                        Button::new("batch-upload")
                            .danger()
                            .loading(true)
                            .label(t!("batchTransfer.uploading"))
                            .into_any_element()
                    } else {
                        Button::new("batch-upload")
                            .primary()
                            .icon(Icon::new(IconName::Upload))
                            .label(t!("batchTransfer.start"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                            .into_any_element()
                    }),
            );

        let rows = self.results.iter().enumerate().map(|(ix, r)| {
            let color = if r.success {
                theme.success
            } else {
                theme.danger
            };
            v_flex()
                .id(("batch-result", ix))
                .test_support()
                .px_3()
                .py_1p5()
                .gap_1()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_color(color).child(if r.success {
                                    "✔"
                                } else {
                                    "✘"
                                }))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(r.host_name.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(div().text_color(color).child(if r.success {
                                    t!("batchTransfer.success")
                                } else {
                                    t!("batchTransfer.failed")
                                }))
                                .when(r.duration > 0, |d| d.child(format!("{}ms", r.duration)))
                                .when(r.file_size > 0, |d| d.child(format_size(r.file_size))),
                        ),
                )
                .when_some(r.error.clone(), |d, e| {
                    d.child(div().text_size(px(11.)).text_color(theme.danger).child(e))
                })
        });

        let results = v_flex()
            .id("batch-results")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                h_flex()
                    .p_3()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(t!("batchTransfer.results")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{done}/{total}")),
                            )
                            .when(!self.results.is_empty(), |d| {
                                d.child(
                                    Button::new("batch-clear")
                                        .xsmall()
                                        .label(t!("batchTransfer.clear"))
                                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .px_3()
                    .pb_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(format!("{}%", percent.round()))
                            .child(if pending > 0 {
                                t!("batchTransfer.remaining", count = pending)
                            } else {
                                t!("batchTransfer.nothingPending")
                            }),
                    )
                    .child(
                        div().h(px(6.)).rounded_full().bg(theme.border).child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(theme.primary)
                                .w(relative(percent / 100.0)),
                        ),
                    ),
            )
            .child(if self.results.is_empty() {
                v_flex()
                    .id("batch-empty")
                    .test_support()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Upload)
                            .size_6()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("batchTransfer.emptyTitle")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(if total > 0 {
                                t!("batchTransfer.waiting")
                            } else {
                                t!("batchTransfer.noTargets")
                            }),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .id("batch-results-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .into_any_element()
            });

        v_flex()
            .id("batch-transfer-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(strip))
            .child(h_flex().flex_1().min_h_0().child(config).child(results))
            .into_any_element()
    }
}
