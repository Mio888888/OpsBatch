//! Right-hand host monitor (port of `HostMonitorPanel`): system info, CPU/memory/swap bars, top processes,
//! network and ping charts, filesystems. Polling starts only when the user asks for it, then every 3 seconds.

use std::time::Duration;

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::*;
use ops_i18n::t;
use ops_terminal::monitor::{
    is_host_monitor_idle, should_poll_host_monitor, HostMonitorRefreshState,
};

use super::monitor_model::{format_bytes_per_second, format_memory, MonitorModel};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

pub struct HostMonitorPanel {
    host_id: String,
    host_ip: String,
    model: MonitorModel,
    error: Option<String>,
    realtime: bool,
    _poll: Option<Task<()>>,
}

impl HostMonitorPanel {
    pub fn new(host_id: String, host_ip: String) -> Self {
        Self {
            host_id,
            host_ip,
            model: MonitorModel::new(),
            error: None,
            realtime: false,
            _poll: None,
        }
    }

    pub fn host_id(&self) -> &str {
        &self.host_id
    }
    pub fn model(&self) -> &MonitorModel {
        &self.model
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn is_realtime(&self) -> bool {
        self.realtime
    }

    /// "Start realtime refresh": fetch immediately, then every 3 seconds; requests never overlap.
    pub fn start_realtime(&mut self, cx: &mut Context<Self>) {
        if self.realtime {
            return;
        }
        self.realtime = true;
        let state = HostMonitorRefreshState {
            realtime_enabled: true,
            has_snapshot: false,
        };
        if !should_poll_host_monitor(state) {
            return;
        }
        let ctx = crate::services::ctx(cx);
        let host_id = self.host_id.clone();
        self._poll = Some(cx.spawn(async move |this, cx| loop {
            let (c, id) = (ctx.clone(), host_id.clone());
            let result = ops_core::rt::run(async move {
                ops_ssh::hosts::get_host_monitor_snapshot(c.core.clone(), c.registry.clone(), id)
                    .await
            })
            .await
            .and_then(|r| r);
            let alive = this
                .update(cx, |this, cx| {
                    match result {
                        Ok(snapshot) => {
                            this.model.push(snapshot);
                            this.error = None;
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
                .is_ok();
            if !alive {
                break;
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }));
        cx.notify();
    }

    fn state(&self) -> HostMonitorRefreshState {
        HostMonitorRefreshState {
            realtime_enabled: self.realtime,
            has_snapshot: !self.model.is_empty(),
        }
    }
}

fn line(text: impl Into<SharedString>, color: Hsla) -> Div {
    div().text_color(color).child(text.into())
}

fn section(cx: &App) -> Div {
    div()
        .px_1p5()
        .py_1()
        .border_t_1()
        .border_color(cx.theme().border)
        .flex()
        .flex_col()
        .gap_0p5()
}

fn skeleton_line(width: Pixels, cx: &App) -> Div {
    div()
        .h(px(8.))
        .w(width)
        .rounded(px(3.))
        .bg(cx.theme().border)
}

fn resource_bar(
    label: SharedString,
    value: Option<f64>,
    detail: String,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .gap_1()
        .items_center()
        .my(px(3.))
        .child(
            div()
                .w(px(36.))
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .h(px(18.))
                .overflow_hidden()
                .bg(theme.border)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .bottom_0()
                        .w(relative((value.unwrap_or(0.) / 100.) as f32))
                        .bg(Hsla {
                            a: 0.35,
                            ..theme.success
                        }),
                )
                .child(
                    div()
                        .relative()
                        .px_1()
                        .text_size(px(10.))
                        .text_right()
                        .child(detail),
                ),
        )
}

fn bars(heights: Vec<f32>, color: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .h(px(68.))
        .items_end()
        .gap(px(2.))
        .overflow_hidden()
        .px_1p5()
        .py_1()
        .rounded(px(4.))
        .bg(Hsla {
            a: 0.3,
            ..theme.border
        })
        .children(heights.into_iter().map(move |h| {
            div()
                .flex_1()
                .min_w(px(3.))
                .h(relative(h / 100.))
                .min_h(px(2.))
                .bg(color)
                .rounded_t(px(1.))
        }))
}

fn chart_note(text: SharedString, cx: &App) -> Div {
    div()
        .h(px(68.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl Render for HostMonitorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let idle = is_host_monitor_idle(self.state());
        let loading = !idle && self.model.is_empty() && self.error.is_none();
        let snapshot = self.model.latest().cloned();
        let not_started = t!("terminal.monitorNotStarted");

        let host_row = h_flex()
            .justify_between()
            .items_center()
            .gap_2()
            .mb_1()
            .px_1p5()
            .py_1()
            .text_color(theme.primary)
            .child(div().min_w_0().truncate().child(format!(
                "IP {}",
                if self.host_ip.is_empty() {
                    "--"
                } else {
                    &self.host_ip
                }
            )))
            .child({
                let ip = self.host_ip.clone();
                Button::new("monitor-copy-ip")
                    .ghost()
                    .xsmall()
                    .label(t!("terminal.copy"))
                    .disabled(self.host_ip.is_empty())
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(ip.clone()))
                    })
            });

        let system = {
            let mut s = section(cx).child(
                div()
                    .text_color(theme.primary)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("─ {} ─", t!("terminal.systemInfo"))),
            );
            if idle {
                s = s.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Button::new("monitor-start")
                                .small()
                                .icon(Icon::new(IconName::Play))
                                .label(t!("terminal.startRealtimeMonitor"))
                                .on_click(cx.listener(|this, _, _, cx| this.start_realtime(cx))),
                        )
                        .child(line(t!("terminal.monitorIdleHint"), theme.muted_foreground)),
                );
            } else if loading {
                s = s.child(
                    v_flex()
                        .gap_1()
                        .child(skeleton_line(px(160.), cx))
                        .child(skeleton_line(px(110.), cx))
                        .child(skeleton_line(px(220.), cx))
                        .child(skeleton_line(px(90.), cx)),
                );
            } else {
                let g = |o: &Option<String>| o.clone().unwrap_or_else(|| "--".into());
                s = s
                    .child(line(
                        t!(
                            "terminal.uptime",
                            value = snapshot
                                .as_ref()
                                .map(|x| g(&x.uptime))
                                .unwrap_or_else(|| "--".into())
                        ),
                        theme.foreground,
                    ))
                    .child(line(
                        t!(
                            "terminal.loadAverage",
                            value = snapshot
                                .as_ref()
                                .map(|x| g(&x.load_average))
                                .unwrap_or_else(|| "--".into())
                        ),
                        theme.foreground,
                    ))
                    .child(line(
                        snapshot
                            .as_ref()
                            .and_then(|x| x.os.clone())
                            .unwrap_or_else(|| t!("common.noData")),
                        theme.muted_foreground,
                    ))
                    .child(line(
                        t!(
                            "terminal.kernel",
                            value = snapshot
                                .as_ref()
                                .map(|x| g(&x.kernel))
                                .unwrap_or_else(|| "--".into())
                        ),
                        theme.muted_foreground,
                    ));
            }
            if let Some(e) = &self.error {
                s = s.child(line(e.clone(), theme.danger));
            }
            s
        };

        let resources = {
            let mut s = section(cx);
            if idle {
                s = s.child(line(not_started.clone(), theme.muted_foreground));
            } else if loading {
                s = s.children((0..3).map(|_| skeleton_line(px(240.), cx).my_1()));
            } else {
                let cpu = self.model.cpu_percent();
                let mem = snapshot
                    .as_ref()
                    .map(|x| format_memory(x.memory_used_mb, x.memory_total_mb))
                    .unwrap_or_else(|| "--".into());
                let swap = snapshot
                    .as_ref()
                    .map(|x| format_memory(x.swap_used_mb, x.swap_total_mb))
                    .unwrap_or_else(|| "--".into());
                s = s
                    .child(resource_bar(
                        "CPU".into(),
                        cpu,
                        cpu.map(|c| format!("{:.0}%", c))
                            .unwrap_or_else(|| "--".into()),
                        cx,
                    ))
                    .child(resource_bar(
                        t!("terminal.memory").into(),
                        self.model.memory_percent(),
                        mem,
                        cx,
                    ))
                    .child(resource_bar(
                        t!("terminal.swap").into(),
                        self.model.swap_percent(),
                        swap,
                        cx,
                    ));
            }
            s
        };

        let processes = {
            let mut s = section(cx).child(
                div().grid().grid_cols(1).child(
                    h_flex()
                        .gap_1()
                        .text_color(theme.primary)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(div().w(px(56.)).child(t!("terminal.memory")))
                        .child(div().w(px(40.)).child("CPU"))
                        .child(div().flex_1().child(t!("terminal.processCommand"))),
                ),
            );
            if idle {
                s = s.child(line(not_started.clone(), theme.muted_foreground));
            } else if loading {
                s = s.children((0..3).map(|_| skeleton_line(px(250.), cx).my_0p5()));
            } else if let Some(x) = &snapshot {
                if x.processes.is_empty() {
                    s = s.child(line(t!("common.noData"), theme.muted_foreground));
                }
                s = s.children(x.processes.iter().enumerate().map(|(i, p)| {
                    h_flex()
                        .id(("monitor-process", i))
                        .gap_1()
                        .child(div().w(px(56.)).truncate().child(p.memory.clone()))
                        .child(div().w(px(40.)).truncate().child(p.cpu.clone()))
                        .child(div().flex_1().min_w_0().truncate().child(p.command.clone()))
                }));
            }
            s
        };

        let (rx, tx) = self.model.current_rate();
        let interfaces = self.model.interfaces();
        let active_iface = self.model.active_interface();
        let weak = cx.entity().downgrade();
        let net_head = h_flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .bg(Hsla {
                a: 0.07,
                ..theme.primary
            })
            .text_color(theme.primary)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(10.))
            .child(format!("↑{}", format_bytes_per_second(tx)))
            .child(format!("↓{}", format_bytes_per_second(rx)))
            .child(if interfaces.is_empty() {
                div().child("--").into_any_element()
            } else {
                let items = interfaces.clone();
                let active = active_iface.clone();
                Button::new("monitor-iface")
                    .xsmall()
                    .outline()
                    .label(active_iface.clone())
                    .tooltip(t!("terminal.switchNetworkCard"))
                    .disabled(interfaces.len() == 1)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu;
                        for name in items.iter().cloned() {
                            let weak = weak.clone();
                            let n = name.clone();
                            menu = menu.item(
                                PopupMenuItem::new(name.clone())
                                    .checked(name == active)
                                    .on_click(move |_, _, cx| {
                                        let n = n.clone();
                                        let _ = weak.update(cx, |this, cx| {
                                            this.model.select_interface(&n);
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu
                    })
                    .into_any_element()
            });
        let network = {
            let mut s = section(cx).child(net_head);
            s = s.child(if idle {
                chart_note(not_started.clone().into(), cx).into_any_element()
            } else if loading {
                bars(
                    (0..18).map(|i| (18 + (i % 7) * 9) as f32).collect(),
                    Hsla {
                        a: 0.4,
                        ..theme.border
                    },
                    cx,
                )
                .into_any_element()
            } else if self.model.len() > 1 {
                bars(
                    self.model.network_bars(),
                    Hsla {
                        a: 0.56,
                        ..theme.primary
                    },
                    cx,
                )
                .into_any_element()
            } else {
                chart_note(t!("terminal.collecting").into(), cx).into_any_element()
            });
            s
        };

        let ping = {
            let label = snapshot
                .as_ref()
                .and_then(|x| x.ping_ms)
                .map(|p| format!("PING {p:.1}ms"))
                .unwrap_or_else(|| "PING --".into());
            let mut s = section(cx).child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.))
                    .bg(Hsla {
                        a: 0.07,
                        ..theme.primary
                    })
                    .text_color(theme.primary)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(10.))
                    .child(label),
            );
            s = s.child(if idle {
                chart_note(not_started.clone().into(), cx).into_any_element()
            } else if loading {
                bars(
                    (0..18).map(|i| (14 + (i % 5) * 10) as f32).collect(),
                    Hsla {
                        a: 0.4,
                        ..theme.border
                    },
                    cx,
                )
                .into_any_element()
            } else if !self.model.is_empty() {
                bars(
                    self.model.ping_bars(),
                    Hsla {
                        a: 0.64,
                        ..theme.primary
                    },
                    cx,
                )
                .into_any_element()
            } else {
                chart_note(t!("terminal.collecting").into(), cx).into_any_element()
            });
            s
        };

        let filesystems = {
            let mut s = section(cx).child(
                h_flex()
                    .gap_2()
                    .pb_0p5()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_color(theme.primary)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().flex_1().child(t!("terminal.path")))
                    .child(div().w(px(90.)).child(t!("terminal.availableSize"))),
            );
            if idle {
                s = s.child(line(not_started, theme.muted_foreground));
            } else if loading {
                s = s.children((0..3).map(|_| skeleton_line(px(250.), cx).my_0p5()));
            } else if let Some(x) = &snapshot {
                if x.filesystems.is_empty() {
                    s = s.child(line(t!("common.noData"), theme.muted_foreground));
                }
                s = s.children(x.filesystems.iter().enumerate().map(|(i, f)| {
                    h_flex()
                        .id(("monitor-fs", i))
                        .gap_2()
                        .child(div().flex_1().min_w_0().truncate().child(f.path.clone()))
                        .child(
                            div()
                                .w(px(90.))
                                .truncate()
                                .child(format!("{}/{}", f.available, f.total)),
                        )
                }));
            }
            s
        };

        v_flex()
            .id("host-monitor")
            .size_full()
            .overflow_hidden()
            .bg(theme.sidebar)
            .border_l_1()
            .border_color(theme.border)
            .text_size(px(11.))
            .child(
                v_flex()
                    .id("host-monitor-content")
                    .w(px(272.))
                    .min_w(px(272.))
                    .h_full()
                    .overflow_y_scroll()
                    .pl_2()
                    .pr_1p5()
                    .py_2()
                    .child(host_row)
                    .child(system)
                    .child(resources)
                    .child(processes)
                    .child(network)
                    .child(ping)
                    .child(filesystems),
            )
    }
}
