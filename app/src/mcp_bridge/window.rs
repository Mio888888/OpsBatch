//! The small always-on-top window at the top right of the screen that opens while an outside agent works through the
//! MCP bridge: the server being operated on top (name, address, a live preview of its terminal), the operation records
//! below, and the commands waiting for approval.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;

use super::activity::{ActivityEvent, Decision, McpActivity, Record, Status, Target};
use super::{activity as activity_of, bridge, BridgeStatus};
use crate::terminal::workspace::workspace;

pub const LABEL: &str = "mcp-activity";
const WIDTH: f32 = 380.;
/// Lines of the terminal shown in the 16:10 view (10px text, 1.35 line height).
const PREVIEW_LINES: usize = 15;
const HEIGHT: f32 = 660.;
/// The floating approval cards sit this far above the bottom edge, clear of the footer line.
const APPROVALS_BOTTOM: f32 = 50.;

fn options(cx: &App) -> WindowOptions {
    // top right of the primary display, under the menu bar
    let bounds = cx
        .primary_display()
        .map(|d| {
            let b = d.bounds();
            Bounds {
                origin: point(
                    b.origin.x + b.size.width - px(WIDTH + 16.),
                    b.origin.y + px(44.),
                ),
                size: size(px(WIDTH), px(HEIGHT)),
            }
        })
        .unwrap_or_else(|| Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(300.), px(320.))),
        titlebar: Some(TitlebarOptions {
            title: Some(t!("mcpwin.title").into()),
            ..gpui_kit::component::TitleBar::title_bar_options()
        }),
        kind: gpui_kit::WindowKind::PopUp,
        focus: false,
        is_minimizable: false,
        ..gpui_kit::component::TitleBar::window_options()
    }
}

/// Opens the window (without taking the focus unless `activate`), or raises the open one.
pub fn show(activate: bool, cx: &mut App) {
    let activity = activity_of(cx);
    crate::window::open_background(LABEL, options(cx), activate, cx, move |window, cx| {
        cx.new(|cx| McpActivityWindow::new(activity, window, cx))
    });
}

pub struct McpActivityWindow {
    activity: Entity<McpActivity>,
    _subs: Vec<Subscription>,
    _refresh: Task<()>,
}

impl McpActivityWindow {
    pub fn new(
        activity: Entity<McpActivity>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![
            cx.observe(&activity, |_, _, cx| cx.notify()),
            cx.observe(&bridge(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&activity, |_, _, ev: &ActivityEvent, cx| {
                let ActivityEvent::ApprovalNeeded(_) = ev;
                cx.notify();
            }),
        ];
        // the terminal preview of the header follows the session while calls are running
        let refresh = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(800))
                .await;
            if this
                .update(cx, |this, cx| {
                    let busy = this
                        .activity
                        .read(cx)
                        .records()
                        .iter()
                        .any(|r| matches!(r.status, Status::Running | Status::Pending));
                    if busy {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        });
        Self {
            activity,
            _subs: subs,
            _refresh: refresh,
        }
    }

    pub fn decide(&mut self, id: &str, decision: Decision, cx: &mut Context<Self>) {
        self.activity.update(cx, |a, cx| a.decide(id, decision, cx));
    }

    /// The last lines shown in the target's terminal (the "thumbnail" of the server).
    pub fn preview(&self, target: &Target, cx: &App) -> String {
        let ws = workspace(cx).read(cx);
        for tab in ws.tabs() {
            for pane in tab
                .pane
                .iter()
                .chain(tab.split.iter().filter_map(|s| s.pane.as_ref()))
            {
                if pane.read(cx).session_id == target.session_id {
                    return super::last_lines(&pane.read(cx).buffer_text(), PREVIEW_LINES);
                }
            }
        }
        String::new()
    }
}

fn status_style(
    status: Status,
    ui: &crate::theme::UiColors,
    danger: Hsla,
) -> (&'static str, IconName, Hsla) {
    match status {
        Status::Running => ("mcpwin.status.running", IconName::Loader, ui.primary),
        Status::Pending => (
            "mcpwin.status.pending",
            IconName::Clock,
            crate::ai::settings::ui::amber(),
        ),
        Status::Done => (
            "mcpwin.status.done",
            IconName::Check,
            crate::ai::settings::ui::green(),
        ),
        Status::Failed => ("mcpwin.status.failed", IconName::X, danger),
        Status::Denied => ("mcpwin.status.denied", IconName::Ban, danger),
    }
}

impl Render for McpActivityWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::ActiveTheme as _;
        let ui = crate::theme::ui(cx);
        let danger = cx.theme().danger;
        let activity = self.activity.read(cx);
        let records: Vec<Record> = activity.records().iter().rev().cloned().collect();
        let pending: Vec<Record> = activity.pending().into_iter().cloned().collect();
        let target = activity.target().cloned();
        let weak = cx.entity().downgrade();

        // header: just the live view of the module being operated on (a terminal today), at a common 16:10 screen shape
        let header: AnyElement = match &target {
            Some(tg) => {
                let preview = self.preview(tg, cx);
                div()
                    .id("mcp-target")
                    .test_support()
                    .w_full()
                    .aspect_ratio(16. / 10.)
                    .flex_none()
                    .overflow_hidden()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(ui.border)
                    .bg(ui.terminal_bg)
                    .px(px(10.))
                    .py(px(8.))
                    .text_color(ui.term_fg)
                    .font_family("Menlo")
                    .text_size(px(10.))
                    .line_height(relative(1.35))
                    .child(
                        div()
                            .id("mcp-preview")
                            .test_support()
                            .size_full()
                            .overflow_hidden()
                            .child(if preview.is_empty() {
                                "…".to_string()
                            } else {
                                preview
                            }),
                    )
                    .into_any_element()
            }
            None => div()
                .id("mcp-target-empty")
                .test_support()
                .w_full()
                .aspect_ratio(16. / 10.)
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .border_1()
                .border_color(ui.border)
                .bg(ui.terminal_bg)
                .text_size(px(12.))
                .text_color(ui.text_muted)
                .child(t!("mcpwin.waiting"))
                .into_any_element(),
        };

        // approvals: floating cards over the lower part of the records, so answering one (the card disappears) never
        // pushes the records around
        let card_base = ui
            .main
            .blend(crate::ai::settings::ui::amber().opacity(0.08));
        let approvals: Vec<AnyElement> = pending
            .iter()
            .enumerate()
            .map(|(ix, r)| {
                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                let (id1, id2, id3) = (r.id.clone(), r.id.clone(), r.id.clone());
                let fade_key = {
                    use std::hash::{Hash as _, Hasher as _};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    r.id.hash(&mut h);
                    h.finish() as usize
                };
                let card = v_flex()
                    .id(SharedString::from(format!("mcp-approval-{ix}")))
                    .test_support()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(crate::ai::settings::ui::amber().opacity(0.7))
                    .bg(card_base)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .size_4()
                                    .text_color(crate::ai::settings::ui::amber()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("mcpwin.approvalTitle")),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(ui.text_muted)
                            .child(t!(&format!("mcpwin.tool.{}", r.tool))),
                    )
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .bg(ui.terminal_bg)
                            .text_color(ui.term_fg)
                            .font_family("Menlo")
                            .text_size(px(11.))
                            .line_height(relative(1.4))
                            .child(r.summary.clone()),
                    )
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(
                                Button::new(SharedString::from(format!("mcp-approve-{ix}")))
                                    .small()
                                    .primary()
                                    .label(t!("mcpwin.approve"))
                                    .on_click(move |_, _, cx| {
                                        let _ = w1.update(cx, |this, cx| {
                                            this.decide(&id1, Decision::Approve, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("mcp-always-{ix}")))
                                    .small()
                                    .label(t!("mcpwin.alwaysAllow"))
                                    .on_click(move |_, _, cx| {
                                        let _ = w2.update(cx, |this, cx| {
                                            this.decide(&id2, Decision::AlwaysAllow, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("mcp-reject-{ix}")))
                                    .small()
                                    .danger()
                                    .label(t!("mcpwin.reject"))
                                    .on_click(move |_, _, cx| {
                                        let _ = w3.update(cx, |this, cx| {
                                            this.decide(&id3, Decision::Reject, cx)
                                        });
                                    }),
                            ),
                    );
                // a new card fades in (once per approval, not on every repaint)
                crate::workbench::motion::fade_in(card, ("mcp-approval-fade", fade_key), cx)
            })
            .collect();

        // records, newest first
        let rows: Vec<AnyElement> = records
            .iter()
            .enumerate()
            .map(|(ix, r)| {
                let (label, icon, color) = status_style(r.status, &ui, danger);
                let first = r
                    .detail
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .to_string();
                v_flex()
                    .id(SharedString::from(format!("mcp-record-{ix}")))
                    .test_support()
                    .gap(px(2.))
                    .py(px(7.))
                    .border_b_1()
                    .border_color(ui.border.opacity(0.6))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(Icon::new(icon).size_3p5().text_color(color))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(t!(&format!("mcpwin.tool.{}", r.tool))),
                            )
                            .child(div().text_size(px(10.)).text_color(color).child(t!(label)))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(ui.text_muted)
                                    .child(r.time.clone()),
                            ),
                    )
                    .when(!r.summary.is_empty(), |d| {
                        d.child(
                            div()
                                .font_family("Menlo")
                                .text_size(px(11.))
                                .text_color(ui.text_secondary)
                                .truncate()
                                .child(r.summary.clone()),
                        )
                    })
                    // the result line is always there (empty while the call runs) so a finished record keeps its height
                    .child(
                        div()
                            .h(px(18.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(ui.text_muted)
                            .truncate()
                            .child(
                                if r.status == Status::Running || r.status == Status::Pending {
                                    String::new()
                                } else {
                                    first
                                },
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        let status_line = match bridge(cx).read(cx).status() {
            BridgeStatus::Running(url) => format!("{} · {url}", t!("mcpwin.bridge")),
            BridgeStatus::Starting => t!("ncai.externalMcp.status.starting"),
            BridgeStatus::Error(e) => format!("{} · {e}", t!("ncai.externalMcp.status.error")),
            _ => t!("ncai.externalMcp.status.disabled"),
        };

        v_flex()
            .id("mcp-window")
            .test_support()
            .relative()
            .size_full()
            .bg(ui.main)
            .text_color(ui.text)
            .gap(px(10.))
            .px(px(12.))
            .pb(px(12.))
            // clear of the window buttons
            .pt(px(38.))
            .child(header)
            .child(
                v_flex()
                    .id("mcp-records")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .pb(px(4.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text_muted)
                            .child(t!("mcpwin.records")),
                    )
                    .when(rows.is_empty(), |d| {
                        d.child(
                            div()
                                .py(px(12.))
                                .text_size(px(12.))
                                .text_color(ui.text_muted)
                                .child(t!("mcpwin.noRecords")),
                        )
                    })
                    .children(rows),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .id("mcp-bridge-status")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.))
                            .font_family("Menlo")
                            .text_color(ui.text_muted)
                            .child(status_line),
                    )
                    .child(
                        Button::new("mcp-clear")
                            .ghost()
                            .xsmall()
                            .label(t!("mcpwin.clear"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.activity.update(cx, |a, cx| a.clear_finished(cx))
                            })),
                    ),
            )
            .when(!approvals.is_empty(), |d| {
                d.child(
                    div()
                        .id("mcp-approvals")
                        .test_support()
                        .absolute()
                        .left(px(12.))
                        .right(px(12.))
                        .bottom(px(APPROVALS_BOTTOM))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(approvals),
                )
            })
    }
}
