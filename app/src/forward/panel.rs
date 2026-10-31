//! Forward rules of one host: list, add form, stop, delete. Port of `components/PortForwardPanel.tsx`.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_ssh::forward::{ForwardConfig, ForwardEntry, ForwardStatus, ForwardType};

use super::{form_is_complete, format_bytes};
use crate::sftp::store::blocking;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::toast;

pub struct PortForwardPanel {
    host_id: String,
    forwards: Vec<ForwardEntry>,
    show_form: bool,
    form_type: ForwardType,
    type_select: Entity<ChoiceSelect>,
    local_input: Entity<InputState>,
    remote_input: Entity<InputState>,
    label_input: Entity<InputState>,
    adding: bool,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl PortForwardPanel {
    pub fn new(host_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ctx = crate::services::ctx(cx);
        let choices = vec![
            Choice::new("local", t!("portForward.localForward")),
            Choice::new("remote", t!("portForward.remoteForward")),
            Choice::new("dynamic", t!("portForward.dynamicForward")),
        ];
        let type_select = cx.new(|cx| new_select(choices, "local", window, cx));
        let local_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("portForward.localAddr"))
                .default_value("127.0.0.1:")
        });
        let remote_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("portForward.remoteAddr")));
        let label_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("portForward.notePlaceholder")));

        let subs = vec![
            cx.subscribe_in(&type_select, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.form_type = match v.as_str() {
                        "remote" => ForwardType::Remote,
                        "dynamic" => ForwardType::Dynamic,
                        _ => ForwardType::Local,
                    };
                    let placeholder = if matches!(this.form_type, ForwardType::Dynamic) {
                        t!("portForward.localAddrDynamic")
                    } else {
                        t!("portForward.localAddr")
                    };
                    this.local_input
                        .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
                    cx.notify();
                }
            }),
            cx.observe(&local_input, |_, _, cx| cx.notify()),
            cx.observe(&remote_input, |_, _, cx| cx.notify()),
        ];

        let mut tasks = Vec::new();
        // Initial list.
        {
            let (c, id) = (ctx.clone(), host_id.clone());
            tasks.push(cx.spawn(async move |this, cx| {
                let list = blocking(move || ops_ssh::forward::forward_list(&c.forwards, id))
                    .await
                    .unwrap_or_default();
                let _ = this.update(cx, |this, cx| {
                    for e in list {
                        this.upsert(e);
                    }
                    cx.notify();
                });
            }));
        }
        // Live status updates.
        {
            let events = ctx.events.subscribe(format!("forward-status:{host_id}"));
            tasks.push(cx.spawn(async move |this, cx| {
                while let Ok(ev) = events.recv_async().await {
                    let Some(entry) = ev.parse::<ForwardEntry>() else {
                        continue;
                    };
                    if this
                        .update(cx, |this, cx| {
                            this.upsert(entry);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        Self {
            host_id,
            forwards: Vec::new(),
            show_form: false,
            form_type: ForwardType::Local,
            type_select,
            local_input,
            remote_input,
            label_input,
            adding: false,
            _subs: subs,
            _tasks: tasks,
        }
    }

    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    /// (local, remote, note) text fields of the add form.
    pub fn form_inputs(&self) -> (Entity<InputState>, Entity<InputState>, Entity<InputState>) {
        (
            self.local_input.clone(),
            self.remote_input.clone(),
            self.label_input.clone(),
        )
    }

    pub fn forwards(&self) -> &[ForwardEntry] {
        &self.forwards
    }

    pub fn set_form_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.show_form = visible;
        cx.notify();
    }

    fn upsert(&mut self, entry: ForwardEntry) {
        match self.forwards.iter_mut().find(|f| f.id == entry.id) {
            Some(slot) => *slot = entry,
            None => self.forwards.push(entry),
        }
    }

    /// Create the rule from the form fields.
    pub fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dynamic = matches!(self.form_type, ForwardType::Dynamic);
        let local = self.local_input.read(cx).value().to_string();
        let remote = self.remote_input.read(cx).value().to_string();
        let label = self.label_input.read(cx).value().to_string();
        if self.adding || !form_is_complete(dynamic, &local, &remote) {
            return;
        }
        self.adding = true;
        cx.notify();
        let config = ForwardConfig {
            forward_type: self.form_type.clone(),
            local_addr: local,
            remote_addr: if dynamic { String::new() } else { remote },
            label: (!label.is_empty()).then_some(label),
        };
        let (ctx, host) = (crate::services::ctx(cx), self.host_id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let c = ctx.clone();
            let result = blocking(move || {
                ops_ssh::forward::forward_add(
                    c.clone(),
                    &c.db,
                    &c.registry,
                    &c.forwards,
                    host,
                    config,
                )
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.adding = false;
                match result {
                    Ok(entry) => {
                        this.upsert(entry);
                        this.show_form = false;
                        this.local_input
                            .update(cx, |s, cx| s.set_value("127.0.0.1:", window, cx));
                        this.remote_input
                            .update(cx, |s, cx| s.set_value("", window, cx));
                        this.label_input
                            .update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stop(&mut self, forward_id: &str, cx: &mut Context<Self>) {
        let (ctx, host, id) = (
            crate::services::ctx(cx),
            self.host_id.clone(),
            forward_id.to_string(),
        );
        cx.spawn(async move |_, _| {
            if let Err(e) =
                blocking(move || ops_ssh::forward::forward_stop(&ctx.forwards, host, id)).await
            {
                log::warn!("forward_stop failed: {e}");
            }
        })
        .detach();
    }

    pub fn remove(&mut self, forward_id: &str, cx: &mut Context<Self>) {
        let (ctx, host, id) = (
            crate::services::ctx(cx),
            self.host_id.clone(),
            forward_id.to_string(),
        );
        let id2 = id.clone();
        cx.spawn(async move |this, cx| {
            match blocking(move || ops_ssh::forward::forward_remove(&ctx.forwards, host, id)).await
            {
                Ok(()) => {
                    let _ = this.update(cx, |this, cx| {
                        this.forwards.retain(|f| f.id != id2);
                        cx.notify();
                    });
                }
                Err(e) => log::warn!("forward_remove failed: {e}"),
            }
        })
        .detach();
    }
}

impl Render for PortForwardPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let dynamic = matches!(self.form_type, ForwardType::Dynamic);
        let complete = form_is_complete(
            dynamic,
            &self.local_input.read(cx).value(),
            &self.remote_input.read(cx).value(),
        );

        let form = self.show_form.then(|| {
            v_flex()
                .id("pf-form")
                .test_support()
                .gap_1p5()
                .p_2()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .w(px(180.))
                        .child(Select::new(&self.type_select).small()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(Input::new(&self.local_input).small()))
                        .when(!dynamic, |d| {
                            d.child("→")
                                .child(div().flex_1().child(Input::new(&self.remote_input).small()))
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.label_input).small()))
                        .child(
                            Button::new("pf-submit")
                                .primary()
                                .small()
                                .label(t!("portForward.confirm"))
                                .loading(self.adding)
                                .disabled(!complete)
                                .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                        ),
                )
        });

        let rows: Vec<AnyElement> = self
            .forwards
            .iter()
            .enumerate()
            .map(|(ix, f)| {
                let (letter, chip) = match f.config.forward_type {
                    ForwardType::Local => ("L", theme.primary),
                    ForwardType::Remote => ("R", theme.warning),
                    ForwardType::Dynamic => ("D", theme.success),
                };
                let (status_label, status_color) = match f.status {
                    ForwardStatus::Active => (t!("portForward.statusActive"), theme.success),
                    ForwardStatus::Suspended => (t!("portForward.statusSuspended"), theme.warning),
                    ForwardStatus::Error => (t!("portForward.statusError"), theme.danger),
                };
                let addr = if matches!(f.config.forward_type, ForwardType::Dynamic) {
                    f.config.local_addr.clone()
                } else {
                    format!("{} → {}", f.config.local_addr, f.config.remote_addr)
                };
                let (stop_id, rm_id) = (f.id.clone(), f.id.clone());
                let active = matches!(f.status, ForwardStatus::Active);
                let error = f.error.clone();
                h_flex()
                    .id(("pf-item", ix))
                    .test_support()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(px(12.))
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded(px(3.))
                            .bg(Hsla { a: 0.2, ..chip })
                            .text_color(chip)
                            .text_center()
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .child(letter),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(addr))
                    .when_some(f.config.label.clone(), |d, l| {
                        d.child(div().text_color(theme.muted_foreground).child(l))
                    })
                    .child(
                        div()
                            .px_1p5()
                            .rounded_full()
                            .bg(Hsla {
                                a: 0.12,
                                ..status_color
                            })
                            .text_color(status_color)
                            .text_size(px(10.))
                            .child(status_label),
                    )
                    .when(f.bytes_sent > 0 || f.bytes_received > 0, |d| {
                        d.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .text_size(px(10.))
                                .child(format!(
                                    "↑{} ↓{}",
                                    format_bytes(f.bytes_sent),
                                    format_bytes(f.bytes_received)
                                )),
                        )
                    })
                    .when_some(error, |d, e| {
                        d.child(
                            div()
                                .id(("pf-error", ix))
                                .text_color(theme.danger)
                                .child("!")
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(e.clone())
                                        .build(window, cx)
                                }),
                        )
                    })
                    .when(active, |d| {
                        d.child(
                            Button::new(("pf-stop", ix))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::Square))
                                .tooltip(t!("portForward.stop"))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.stop(&stop_id, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new(("pf-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::X))
                            .tooltip(t!("common.delete"))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove(&rm_id, cx))),
                    )
                    .into_any_element()
            })
            .collect();

        v_flex()
            .id("pf-panel")
            .test_support()
            .size_full()
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("portForward.title")),
                    )
                    .child(
                        Button::new("pf-toggle-form")
                            .ghost()
                            .xsmall()
                            .label(if self.show_form {
                                t!("common.cancel")
                            } else {
                                t!("portForward.addNew")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_form = !this.show_form;
                                cx.notify();
                            })),
                    ),
            )
            .children(form)
            .child(
                v_flex()
                    .id("pf-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(rows.is_empty(), |d| {
                        d.child(
                            div()
                                .p_4()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(t!("portForward.noRules")),
                        )
                    })
                    .children(rows),
            )
    }
}
