//! Asciinema recording replay (the original shows the accumulated output with ANSI colours in a <pre>).

use crate::scale::px;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;
use ops_core::asciinema::{AsciinemaRecording, RecordingMeta};
use ops_i18n::t;

use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

pub const SPEEDS: [f32; 5] = [0.5, 1.0, 2.0, 5.0, 10.0];

/// Delay before event `next` when playing at `speed` (at least 10 ms, like the original).
pub fn event_delay_ms(prev_time: f64, next_time: f64, speed: f32) -> u64 {
    (((next_time - prev_time) * 1000.0 / speed as f64).max(10.0)) as u64
}

pub struct AsciinemaPlayer {
    history_id: String,
    recordings: Vec<RecordingMeta>,
    host_select: Option<Entity<ChoiceSelect>>,
    recording: Option<AsciinemaRecording>,
    loading: bool,
    playing: bool,
    position: usize,
    output: String,
    speed: f32,
    speed_select: Entity<ChoiceSelect>,
    play_task: Option<Task<()>>,
    load_seq: u64,
    _subs: Vec<Subscription>,
}

impl AsciinemaPlayer {
    pub fn open(history_id: String, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(history_id, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| {
            d.title(t!("commands.recordingReplay"))
                .w(px(900.))
                .child(shown.clone())
        });
        view
    }

    pub fn new(history_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let speed_select = cx.new(|cx| {
            new_select(
                SPEEDS
                    .iter()
                    .map(|s| Choice::new(s.to_string(), format!("{s}x")))
                    .collect(),
                "1",
                window,
                cx,
            )
        });
        let subs = vec![cx.subscribe_in(&speed_select, window, |this, _, ev, _, _| {
            if let Some(v) = confirmed_value(ev).and_then(|v| v.parse::<f32>().ok()) {
                this.speed = v;
            }
        })];
        let mut this = Self {
            history_id,
            recordings: Vec::new(),
            host_select: None,
            recording: None,
            loading: false,
            playing: false,
            position: 0,
            output: String::new(),
            speed: 1.0,
            speed_select,
            play_task: None,
            load_seq: 0,
            _subs: subs,
        };
        this.load_list(window, cx);
        this
    }

    pub fn recordings(&self) -> &[RecordingMeta] {
        &self.recordings
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub fn position(&self) -> usize {
        self.position
    }
    pub fn event_count(&self) -> usize {
        self.recording.as_ref().map_or(0, |r| r.events.len())
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    fn load_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, id) = (crate::services::ctx(cx), self.history_id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let list =
                ops_core::rt::run(
                    async move { ops_core::asciinema::list_recordings(&ctx.core, id) },
                )
                .await
                .and_then(|r| r)
                .unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                this.recordings = list;
                if let Some(first) = this.recordings.first().cloned() {
                    let choices = this
                        .recordings
                        .iter()
                        .map(|r| {
                            Choice::new(
                                r.host_id.clone(),
                                format!(
                                    "{} ({:.1}s)",
                                    if r.host_name.is_empty() {
                                        &r.host_id
                                    } else {
                                        &r.host_name
                                    },
                                    r.duration
                                ),
                            )
                        })
                        .collect();
                    let sel = cx.new(|cx| new_select(choices, &first.host_id, window, cx));
                    this._subs
                        .push(cx.subscribe_in(&sel, window, |this, _, ev, window, cx| {
                            if let Some(h) = confirmed_value(ev) {
                                this.load_recording(&h, window, cx);
                            }
                        }));
                    this.host_select = Some(sel);
                    this.load_recording(&first.host_id, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn load_recording(&mut self, host_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.pause();
        self.loading = true;
        self.load_seq += 1;
        let seq = self.load_seq;
        cx.notify();
        let (ctx, id, host) = (
            crate::services::ctx(cx),
            self.history_id.clone(),
            host_id.to_string(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let rec = ops_core::rt::run(async move {
                ops_core::asciinema::read_asciinema_recording(&ctx.core, id, host)
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, _, cx| {
                if seq != this.load_seq {
                    return; // a newer selection superseded this load
                }
                this.loading = false;
                this.position = 0;
                this.output.clear();
                this.recording = rec.ok().flatten();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn play(&mut self, cx: &mut Context<Self>) {
        if self.recording.as_ref().is_none_or(|r| r.events.is_empty()) || self.playing {
            return;
        }
        self.playing = true;
        cx.notify();
        self.play_task = Some(cx.spawn(async move |this, cx| loop {
            let delay = this
                .update(cx, |this, cx| {
                    let rec = this.recording.as_ref()?;
                    if !this.playing || this.position >= rec.events.len() {
                        this.playing = false;
                        cx.notify();
                        return None;
                    }
                    let i = this.position;
                    this.output.push_str(&rec.events[i].content);
                    this.position += 1;
                    cx.notify();
                    let next = rec.events.get(this.position)?;
                    Some(event_delay_ms(rec.events[i].time, next.time, this.speed))
                })
                .ok()
                .flatten();
            match delay {
                Some(ms) => {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(ms))
                        .await
                }
                None => {
                    let _ = this.update(cx, |this, cx| {
                        this.playing = false;
                        cx.notify();
                    });
                    break;
                }
            }
        }));
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.play_task = None;
    }

    pub fn skip_to_end(&mut self, cx: &mut Context<Self>) {
        self.pause();
        if let Some(r) = &self.recording {
            self.output = r.events.iter().map(|e| e.content.as_str()).collect();
            self.position = r.events.len();
        }
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.pause();
        self.output.clear();
        self.position = 0;
        cx.notify();
    }
}

impl Render for AsciinemaPlayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let total = self.event_count();
        let progress = if total == 0 {
            0.0
        } else {
            self.position as f32 / total as f32
        };
        let ansi = crate::ansi::parse(&self.output);
        let runs: Vec<(std::ops::Range<usize>, HighlightStyle)> = ansi
            .spans
            .iter()
            .map(|(r, c)| {
                (
                    r.clone(),
                    HighlightStyle {
                        color: Some(rgb(*c).into()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let body: AnyElement =
            if self.loading {
                div()
                    .p_6()
                    .flex()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child(t!("common.loading"))
                    .into_any_element()
            } else if self.recording.is_some() {
                v_flex()
                    .gap_3()
                    .child(
                        div().h(px(4.)).rounded_full().bg(theme.border).child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(theme.primary)
                                .w(relative(progress)),
                        ),
                    )
                    .child(
                        div()
                            .id("asciinema-output")
                            .test_support()
                            .h(px(380.))
                            .overflow_y_scroll()
                            .p_4()
                            .rounded(px(8.))
                            .bg(rgb(0x1e1e1e))
                            .text_color(rgb(0xd4d4d4))
                            .font_family("Menlo")
                            .text_size(px(13.))
                            .child(if self.output.is_empty() {
                                StyledText::new(if self.playing {
                                    String::new()
                                } else {
                                    "▶".to_string()
                                })
                            } else {
                                StyledText::new(ansi.text.clone()).with_highlights(runs)
                            }),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(if self.playing {
                                        Button::new("asc-pause").danger().label("⏸").on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.pause();
                                                cx.notify();
                                            }),
                                        )
                                    } else {
                                        Button::new("asc-play")
                                            .primary()
                                            .label("▶")
                                            .disabled_when(self.position >= total)
                                            .on_click(cx.listener(|this, _, _, cx| this.play(cx)))
                                    })
                                    .child(Button::new("asc-end").label("⏭").on_click(
                                        cx.listener(|this, _, _, cx| this.skip_to_end(cx)),
                                    ))
                                    .child(
                                        Button::new("asc-reset")
                                            .label("↺")
                                            .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(90.))
                                            .child(Select::new(&self.speed_select).small()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(theme.muted_foreground)
                                            .child(format!("{}/{}", self.position, total)),
                                    ),
                            ),
                    )
                    .into_any_element()
            } else {
                div()
                    .p_6()
                    .flex()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child(t!("common.noData"))
                    .into_any_element()
            };
        v_flex()
            .id("asciinema-player")
            .test_support()
            .gap_3()
            .when_some(self.host_select.clone(), |d, sel| {
                d.child(div().w(px(300.)).child(Select::new(&sel)))
            })
            .child(body)
            .child(
                h_flex().justify_end().child(
                    Button::new("asc-close")
                        .label(t!("common.close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    }
}

trait DisabledWhen {
    fn disabled_when(self, cond: bool) -> Self;
}

impl DisabledWhen for Button {
    fn disabled_when(self, cond: bool) -> Self {
        use gpui_kit::component::Disableable as _;
        self.disabled(cond)
    }
}

use gpui_kit::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::event_delay_ms;

    #[test]
    fn delays_scale_with_speed_and_never_drop_below_ten_ms() {
        assert_eq!(event_delay_ms(1.0, 2.0, 1.0), 1000);
        assert_eq!(event_delay_ms(1.0, 2.0, 2.0), 500);
        assert_eq!(event_delay_ms(1.0, 1.001, 1.0), 10);
        assert_eq!(event_delay_ms(1.0, 3.0, 10.0), 200);
    }
}
