//! The global log window (port of `pages/Log/GlobalLogPage.tsx`).

use crate::scale::px;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_core::log::AppLogEntry;
use ops_i18n::t;
use regex::Regex;
use std::sync::LazyLock;

use super::store::{log_store, LogStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::window::{self, WindowKind};

/// How many rows are drawn at most.
pub const MAX_RENDERED_LOGS: usize = 300;

static MESSAGE_PATTERNS: LazyLock<Vec<(Regex, &'static str, &'static [&'static str])>> =
    LazyLock::new(|| {
        let p = |re: &str, key: &'static str, names: &'static [&'static str]| {
            (Regex::new(re).unwrap(), key, names)
        };
        vec![
            p(
                r"^主机 (.+) 正在连接\.\.\.$",
                "log.message.sshConnecting",
                &["hostId"],
            ),
            p(
                r"^主机 (.+) 已连接$",
                "log.message.sshConnected",
                &["hostId"],
            ),
            p(r"^主机 (.+) 连接空闲$", "log.message.sshIdle", &["hostId"]),
            p(
                r"^主机 (.+) 连接断开$",
                "log.message.sshDisconnected",
                &["hostId"],
            ),
            p(
                r"^主机 (.+) 正在重连\.\.\.$",
                "log.message.sshReconnecting",
                &["hostId"],
            ),
            p(
                r"^主机 (.+) 正在连接 ([^:]+):(\d+)\.\.\.$",
                "log.message.sshConnectingHost",
                &["hostId", "host", "port"],
            ),
            p(
                r"^主机 (.+) \((.+)\) 已连接$",
                "log.message.sshConnectedHost",
                &["hostId", "host"],
            ),
            p(
                r"^主机 (.+) 连接失败: (.+)$",
                "log.message.sshConnectFailed",
                &["hostId", "error"],
            ),
            p(
                r"^开始执行命令 \[(.*)\]，目标 (\d+) 台主机$",
                "log.message.executionStart",
                &["command", "count"],
            ),
            p(
                r"^命令执行完成 \[(.*)\]：全部成功（(\d+)台，耗时(\d+)ms）$",
                "log.message.executionAllSucceeded",
                &["command", "count", "duration"],
            ),
            p(
                r"^命令执行失败 \[(.*)\]：全部失败（(\d+)台，耗时(\d+)ms）$",
                "log.message.executionAllFailed",
                &["command", "count", "duration"],
            ),
            p(
                r"^命令执行完成 \[(.*)\]：成功(\d+)台，失败(\d+)台（耗时(\d+)ms）$",
                "log.message.executionPartial",
                &["command", "success", "fail", "duration"],
            ),
        ]
    });

/// Backend messages are stored in Chinese; show them in the current language (port of `formatLogMessage`).
pub fn format_log_message(message: &str) -> String {
    if message == "OpsBatch 后端服务已启动" || message == "OpsBatch backend started" {
        return t!("log.message.backendStarted");
    }
    if message == "全局日志窗口已连接" || message == "Global log connected" {
        return t!("log.connectedMessage");
    }
    for (re, key, names) in MESSAGE_PATTERNS.iter() {
        if let Some(caps) = re.captures(message) {
            let lang = ops_i18n::current_language();
            let template = ops_i18n::template(lang, key);
            let mut out = template.to_string();
            for (i, name) in names.iter().enumerate() {
                out = out.replace(&format!("{{{name}}}"), &caps[i + 1]);
            }
            return out;
        }
    }
    message.to_string()
}

/// Entries that pass the level/origin filters (`"all"` disables a filter).
pub fn filter_logs<'a>(logs: &'a [AppLogEntry], level: &str, origin: &str) -> Vec<&'a AppLogEntry> {
    logs.iter()
        .filter(|l| (level == "all" || l.level == level) && (origin == "all" || l.origin == origin))
        .collect()
}

pub struct GlobalLogWindow {
    store: Entity<LogStore>,
    level: String,
    origin: String,
    level_select: Entity<ChoiceSelect>,
    origin_select: Entity<ChoiceSelect>,
    scroll: ScrollHandle,
    follow: bool,
    _subs: Vec<Subscription>,
}

impl GlobalLogWindow {
    pub fn open(cx: &mut App) {
        let options = window::window_options(WindowKind::GlobalLog, &t!("nav.globalLog"), cx);
        window::open_or_focus("global-log", options, cx, |window, cx| {
            cx.new(|cx| Self::new(window, cx))
        });
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = log_store(cx);
        store.update(cx, |s, cx| s.ensure_started(cx));
        let levels = vec![
            Choice::new("all", t!("log.allLevels")),
            Choice::new("error", t!("log.error")),
            Choice::new("warn", t!("log.warn")),
            Choice::new("info", t!("log.info")),
            Choice::new("success", t!("log.success")),
        ];
        let origins = vec![
            Choice::new("all", t!("log.allOrigins")),
            Choice::new("backend", t!("log.backend")),
            Choice::new("frontend", t!("log.frontend")),
        ];
        let level_select = cx.new(|cx| new_select(levels, "all", window, cx));
        let origin_select = cx.new(|cx| new_select(origins, "all", window, cx));
        let subs = vec![
            cx.observe(&store, |this, _, cx| {
                if this.follow {
                    this.scroll.set_offset(point(px(0.), px(0.)));
                }
                cx.notify();
            }),
            cx.subscribe(&level_select, |this, _, ev, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.level = v;
                    cx.notify();
                }
            }),
            cx.subscribe(&origin_select, |this, _, ev, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.origin = v;
                    cx.notify();
                }
            }),
        ];
        Self {
            store,
            level: "all".into(),
            origin: "all".into(),
            level_select,
            origin_select,
            scroll: ScrollHandle::new(),
            follow: true,
            _subs: subs,
        }
    }

    pub fn set_filters(
        &mut self,
        level: &str,
        origin: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.level = level.to_string();
        self.origin = origin.to_string();
        let (l, o) = (level.to_string(), origin.to_string());
        self.level_select
            .update(cx, |s, cx| s.set_selected_value(&l, window, cx));
        self.origin_select
            .update(cx, |s, cx| s.set_selected_value(&o, window, cx));
        cx.notify();
    }

    pub fn following(&self) -> bool {
        self.follow
    }

    /// Visible lines as `(time, origin, source, text)` (tests).
    pub fn visible(&self, cx: &App) -> Vec<(String, String, String, String)> {
        let store = self.store.read(cx);
        filter_logs(&store.logs, &self.level, &self.origin)
            .into_iter()
            .take(MAX_RENDERED_LOGS)
            .map(|l| {
                (
                    l.timestamp.clone(),
                    origin_label(&l.origin),
                    source_label(&l.source),
                    format_log_message(&l.message),
                )
            })
            .collect()
    }
}

fn origin_label(origin: &str) -> String {
    match origin {
        "backend" => t!("log.backend"),
        "frontend" => t!("log.frontend"),
        other => other.to_string(),
    }
}

fn source_label(source: &str) -> String {
    match source {
        "system" => t!("log.source.system"),
        "ssh" => t!("log.source.ssh"),
        "execution" => t!("log.source.execution"),
        other => other.to_string(),
    }
}

impl Render for GlobalLogWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        // Following stops when the user scrolls away from the newest entries (the top).
        self.follow = f32::from(self.scroll.offset().y).abs() < 40.0;
        let (total, loading) = {
            let s = self.store.read(cx);
            (
                filter_logs(&s.logs, &self.level, &self.origin).len(),
                s.loading,
            )
        };
        let lines = self.visible(cx);
        // levelColors: error / warn / success / info(primary)
        let level_color = |level: &str| -> Hsla {
            match level {
                "error" => gpui_kit::rgb(0xef4444).into(),
                "warn" => gpui_kit::rgb(0xf59e0b).into(),
                "success" => gpui_kit::rgb(0x22c55e).into(),
                _ => ui.primary,
            }
        };
        let colors: Vec<Hsla> = {
            let s = self.store.read(cx);
            filter_logs(&s.logs, &self.level, &self.origin)
                .into_iter()
                .take(MAX_RENDERED_LOGS)
                .map(|l| level_color(&l.level))
                .collect()
        };
        // `.global-log-action-btn`: 24px, 11px text, surface-soft fill, 5px corners
        let action = |id: &'static str, label: String| {
            div()
                .id(id)
                .test_support()
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .border_1()
                .border_color(ui.border)
                .bg(ui.main)
                .text_size(px(11.))
                .text_color(ui.text_secondary)
                .cursor_pointer()
                .hover(move |d| d.bg(ui.text.opacity(0.1)))
                .child(label)
        };
        v_flex()
            .id("global-log")
            .test_support()
            .size_full()
            .bg(ui.card)
            .text_color(ui.text)
            .child(
                // `.global-log-toolbar`: 40px, 12px side padding, hairline underneath
                TitleBar::new()
                    .h(px(40.))
                    .pl(px(if cfg!(target_os = "macos") { 78. } else { 12. }))
                    .pr(px(12.))
                    .bg(ui.card)
                    .child(
                        h_flex()
                            .w_full()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div().w(px(79.)).child(
                                    Select::new(&self.level_select)
                                        .icon(gpui_kit::component::Icon::new(
                                            gpui_kit::assets::IconName::ChevronsUpDown,
                                        ))
                                        .xsmall()
                                        .text_size(px(12.)),
                                ),
                            )
                            .child(
                                div().w(px(79.)).child(
                                    Select::new(&self.origin_select)
                                        .icon(gpui_kit::component::Icon::new(
                                            gpui_kit::assets::IconName::ChevronsUpDown,
                                        ))
                                        .xsmall()
                                        .text_size(px(12.)),
                                ),
                            )
                            .child(
                                div()
                                    .id("log-count")
                                    .test_support()
                                    .text_size(px(11.))
                                    .text_color(ui.text_muted)
                                    .child(t!("log.count", count = total)),
                            )
                            .child(div().flex_1())
                            .when(!self.follow, |d| {
                                d.child(action("log-follow", t!("log.follow")).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.follow = true;
                                        this.scroll.set_offset(point(px(0.), px(0.)));
                                        cx.notify();
                                    }),
                                ))
                            })
                            .child(action("log-clear", t!("log.clear")).on_click(cx.listener(
                                |this, _, _, cx| this.store.update(cx, |s, cx| s.clear(cx)),
                            ))),
                    ),
            )
            .child(
                // `.workbench-log-body`: soft surface, 14/18 padding, mono font, 1.7 line height
                v_flex()
                    .id("log-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(18.))
                    .py(px(14.))
                    .bg(ui.main)
                    .font_family("Menlo")
                    .text_size(px(14.))
                    .line_height(px(23.))
                    .text_color(ui.text_secondary)
                    .when(loading, |d| {
                        d.child(
                            div()
                                .text_color(ui.text_muted)
                                .text_align(TextAlign::Center)
                                .pt(px(40.))
                                .child(t!("log.loading")),
                        )
                    })
                    .when(!loading && lines.is_empty(), |d| {
                        d.child(
                            div()
                                .id("log-empty")
                                .test_support()
                                .text_color(ui.text_muted)
                                .text_align(TextAlign::Center)
                                .pt(px(40.))
                                .child(t!("log.empty")),
                        )
                    })
                    .children(lines.into_iter().zip(colors).enumerate().map(
                        |(i, ((time, origin, source, text), color))| {
                            h_flex()
                                .id(ElementId::Name(format!("log-line-{i}").into()))
                                .test_support()
                                .gap(px(8.))
                                .items_baseline()
                                .whitespace_nowrap()
                                .child(div().flex_none().text_color(ui.text_muted).child(time))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(color)
                                        .child(format!("[{origin}]")),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(color)
                                        .child(format!("[{source}]")),
                                )
                                .child(div().child(text))
                        },
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_logs, format_log_message};
    use ops_core::log::AppLogEntry;

    fn entry(level: &str, origin: &str, message: &str) -> AppLogEntry {
        AppLogEntry {
            timestamp: "10:00:00.000".into(),
            level: level.into(),
            source: "ssh".into(),
            message: message.into(),
            origin: origin.into(),
        }
    }

    #[test]
    fn stored_chinese_messages_are_shown_in_the_current_language() {
        ops_i18n::set_current_language(ops_i18n::Language::EnUs);
        let f = |m: &str| format_log_message(m);
        assert_eq!(f("主机 h1 已连接"), "Host h1 connected");
        assert_eq!(
            f("主机 h1 连接失败: refused"),
            "Host h1 connection failed: refused"
        );
        assert!(
            f("开始执行命令 [ls -la]，目标 3 台主机").contains("[ls -la]")
                && f("开始执行命令 [ls -la]，目标 3 台主机").contains('3')
        );
        assert!(f("命令执行完成 [uptime]：成功2台，失败1台（耗时15ms）").contains("15"));
        assert_eq!(f("unknown message"), "unknown message");
        ops_i18n::set_current_language(ops_i18n::Language::ZhCn);
        assert_eq!(f("主机 h1 已连接"), "主机 h1 已连接");
        assert_eq!(
            f("主机 srv (10.0.0.1) 已连接"),
            "主机 srv (10.0.0.1) 已连接"
        );
        assert_eq!(f("全局日志窗口已连接"), "全局日志窗口已连接");
    }

    #[test]
    fn filters_combine() {
        let logs = vec![
            entry("error", "backend", "a"),
            entry("info", "backend", "b"),
            entry("error", "frontend", "c"),
        ];
        assert_eq!(filter_logs(&logs, "all", "all").len(), 3);
        assert_eq!(filter_logs(&logs, "error", "all").len(), 2);
        assert_eq!(filter_logs(&logs, "error", "frontend")[0].message, "c");
        assert!(filter_logs(&logs, "success", "all").is_empty());
    }
}
