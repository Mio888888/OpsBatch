//! Title-bar CPU / memory chips for the app's own process (port of TitlebarPerformanceMonitor.tsx).

use std::time::Duration;

use crate::scale::px;
use gpui_kit::base::StyledExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_core::system::{
    calculate_process_cpu_percent, format_process_memory, process_memory_bar_percent,
    LocalPerformanceSnapshot,
};

const REFRESH: Duration = Duration::from_millis(2500);

pub struct PerfMonitor {
    previous: Option<LocalPerformanceSnapshot>,
    memory_mb: Option<f64>,
    cpu_percent: Option<f64>,
    error: bool,
    _task: Task<()>,
}

impl PerfMonitor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |this, cx| loop {
            let snapshot = ops_core::rt::run(ops_core::system::get_local_performance_snapshot())
                .await
                .and_then(|r| r);
            let alive = this
                .update(cx, |this, cx| {
                    match snapshot {
                        Ok(current) => {
                            this.cpu_percent =
                                calculate_process_cpu_percent(this.previous.as_ref(), &current);
                            this.memory_mb = current.memory_rss_mb.map(|m| m as f64);
                            this.previous = Some(current);
                            this.error = false;
                        }
                        Err(_) => this.error = true,
                    }
                    cx.notify();
                })
                .is_ok();
            if !alive {
                break;
            }
            cx.background_executor().timer(REFRESH).await;
        });
        Self {
            previous: None,
            memory_mb: None,
            cpu_percent: None,
            error: false,
            _task: task,
        }
    }

    pub fn cpu_label(&self) -> String {
        self.cpu_percent
            .map(|v| format!("{}%", v.round() as i64))
            .unwrap_or_else(|| "--".into())
    }

    pub fn memory_label(&self) -> String {
        format_process_memory(self.memory_mb)
    }

    pub fn title(&self) -> String {
        if self.error {
            ops_i18n::t!("performance.unavailable")
        } else {
            ops_i18n::t!(
                "performance.title",
                cpu = self.cpu_label(),
                memory = self.memory_label()
            )
        }
    }
}

fn chip(
    label: &'static str,
    percent: f64,
    value: String,
    min_value_w: f32,
    cx: &App,
) -> impl IntoElement {
    let ui = crate::theme::ui(cx);
    div()
        .h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_size(px(9.))
                .font_weight(FontWeight::BOLD)
                .line_height(relative(1.))
                .text_color(ui.text_muted)
                .child(label),
        )
        .child(
            div()
                .w(px(24.))
                .h(px(4.))
                .overflow_hidden()
                .rounded_full()
                .bg(ui.text_muted.opacity(0.18))
                .child(
                    div()
                        .h_full()
                        .w(relative((percent / 100.0).clamp(0.0, 1.0) as f32))
                        .rounded_full()
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(ui.primary, 0.),
                            linear_color_stop(ui.primary, 1.),
                        )),
                ),
        )
        .child(
            div()
                .min_w(px(min_value_w))
                .text_size(px(9.))
                .font_weight(FontWeight::BOLD)
                .line_height(relative(1.))
                .text_color(ui.text_secondary)
                .child(value),
        )
}

impl Render for PerfMonitor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let cpu = self.cpu_percent.unwrap_or(0.0);
        let mem = process_memory_bar_percent(self.memory_mb);
        // `.titlebar-performance-monitor`: 24px pill, 1px border, 68% card surface
        div()
            .id("titlebar-performance")
            .h_flex()
            .h(px(24.))
            .gap_1()
            .px_1p5()
            .py(px(2.))
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(ui.border)
            .bg(ui.card.opacity(0.68))
            .when(self.error, |d| d.opacity(0.62))
            .child(chip("CPU", cpu, self.cpu_label(), 26., cx))
            .child(chip("MEM", mem, self.memory_label(), 48., cx))
    }
}
