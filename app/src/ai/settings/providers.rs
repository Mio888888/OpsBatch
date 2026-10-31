//! Providers tab: the provider list, the "add provider" menu and the per-provider form
//! (`ProviderCard`, `AddProviderDropdown`, `ProviderConfigForm`).

use std::collections::BTreeMap;

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::Select;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_ai::prefs::provider::{
    self, AdvancedParams, ProbeHealth, ProbeInputIssue, ProviderConfig, PROVIDER_PRESETS,
};
use ops_i18n::t;

use super::ui;
use crate::ai::prefs_store::PrefsStore;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

/// Colour of the letter badge standing in for Netcatty's provider icons.
fn badge_color(provider_id: &str) -> Hsla {
    let hue = provider_id
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        % 360;
    hsla(hue as f32 / 360., 0.55, 0.5, 1.)
}

fn icon_badge(p: &ProviderConfig, size: f32) -> Div {
    let color = badge_color(&p.provider_id);
    let letter = p
        .name
        .chars()
        .next()
        .or_else(|| p.provider_id.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size / 4.))
        .bg(color.opacity(0.18))
        .text_color(color)
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .child(letter)
}

pub struct ProvidersTab {
    store: Entity<PrefsStore>,
    editing: Option<String>,
    form: Option<Entity<ProviderForm>>,
    add_open: bool,
    _subs: Vec<Subscription>,
}

impl ProvidersTab {
    pub fn new(store: Entity<PrefsStore>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            editing: None,
            form: None,
            add_open: false,
            _subs: subs,
        }
    }

    pub fn editing(&self) -> Option<&str> {
        self.editing.as_deref()
    }
    pub fn form(&self) -> Option<&Entity<ProviderForm>> {
        self.form.as_ref()
    }
    pub fn providers(&self, cx: &App) -> Vec<ProviderConfig> {
        self.store.read(cx).prefs().providers.clone()
    }
    pub fn active_provider_id(&self, cx: &App) -> String {
        self.store.read(cx).prefs().active_provider_id.clone()
    }

    pub fn add_provider(&mut self, provider_id: &str, cx: &mut Context<Self>) -> String {
        let p = provider::new_provider(provider_id);
        let id = p.id.clone();
        self.store
            .update(cx, |s, cx| s.update(cx, |prefs| prefs.providers.push(p)));
        self.add_open = false;
        cx.notify();
        id
    }

    pub fn toggle_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| p.set_provider_enabled(&id, enabled))
        });
    }

    pub fn remove_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        if self.editing.as_deref() == Some(id.as_str()) {
            self.editing = None;
            self.form = None;
        }
        self.store
            .update(cx, |s, cx| s.update(cx, |p| p.remove_provider(&id)));
        cx.notify();
    }

    fn confirm_remove(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (weak, id) = (weak.clone(), id.clone());
            alert
                .confirm()
                .title(t!("ncai.confirmRemoveProvider", name = name.clone()))
                .show_cancel(true)
                .ok_text(t!("ncai.actionRemove"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| this.remove_provider(&id, cx));
                    true
                })
        });
    }

    /// Opens (or closes, when it is already open) the form of a provider.
    pub fn toggle_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.as_deref() == Some(id) {
            self.close_form(cx);
            return;
        }
        let Some(provider) = self
            .store
            .read(cx)
            .prefs()
            .providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
        else {
            return;
        };
        let store = self.store.clone();
        let form = cx.new(|cx| ProviderForm::new(store, provider, window, cx));
        let sub = cx.subscribe(&form, |this, _, _: &FormClosed, cx| this.close_form(cx));
        self._subs.push(sub);
        self.editing = Some(id.to_string());
        self.form = Some(form);
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.form = None;
        cx.notify();
    }
}

impl Render for ProvidersTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let prefs = self.store.read(cx).prefs().clone();
        let weak = cx.entity().downgrade();

        let add_button = Button::new("ai-provider-add")
            .small()
            .icon(Icon::new(IconName::Plus))
            .label(t!("ncai.providers.add"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.add_open = !this.add_open;
                cx.notify();
            }));

        let menu = self.add_open.then(|| {
            let w = weak.clone();
            v_flex()
                .id("ai-provider-menu")
                .test_support()
                .w_full()
                .rounded(px(10.))
                .border_1()
                .border_color(ui_colors.border)
                .bg(ui_colors.card)
                .py(px(4.))
                .children(PROVIDER_PRESETS.iter().enumerate().map(|(ix, preset)| {
                    let (w, pid) = (w.clone(), preset.id);
                    let proto = ProviderConfig {
                        provider_id: preset.id.into(),
                        name: preset.name.into(),
                        ..Default::default()
                    };
                    h_flex()
                        .id(("ai-provider-menu-item", ix))
                        .test_support()
                        .gap(px(10.))
                        .items_center()
                        .px(px(12.))
                        .py(px(7.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .hover(move |d| d.bg(ui_colors.nav_hover))
                        .child(icon_badge(&proto, 20.))
                        .child(preset.name)
                        .on_click(move |_, _, cx| {
                            let _ = w.update(cx, |this, cx| {
                                this.add_provider(pid, cx);
                            });
                        })
                }))
        });

        let list: AnyElement = if prefs.providers.is_empty() {
            v_flex()
                .id("ai-provider-empty")
                .test_support()
                .items_center()
                .gap(px(8.))
                .p(px(24.))
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(ui_colors.border)
                .child(
                    Icon::new(IconName::Bot)
                        .size_6()
                        .text_color(ui_colors.text_muted),
                )
                .child(ui::muted_text(t!("ncai.providers.empty"), cx))
                .into_any_element()
        } else {
            v_flex()
                .gap(px(8.))
                .children(prefs.providers.iter().enumerate().map(|(ix, p)| {
                    let active = p.id == prefs.active_provider_id;
                    let editing = self.editing.as_deref() == Some(p.id.as_str());
                    let has_key = p.api_key.as_deref().is_some_and(|k| !k.is_empty());
                    let (id_edit, id_remove, id_toggle, name_remove) =
                        (p.id.clone(), p.id.clone(), p.id.clone(), p.name.clone());
                    let (w1, w2) = (weak.clone(), weak.clone());
                    ui::list_card(active, cx)
                        .id(("ai-provider-card", ix))
                        .test_support()
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .items_center()
                                .child(icon_badge(p, 32.))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            h_flex()
                                                .gap(px(8.))
                                                .items_center()
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .truncate()
                                                        .child(p.name.clone()),
                                                )
                                                .when(active, |d| {
                                                    d.child(ui::pill(
                                                        t!("ncai.providers.active"),
                                                        ui_colors.primary,
                                                    ))
                                                }),
                                        )
                                        .child(
                                            h_flex()
                                                .gap(px(8.))
                                                .child(ui::colored_text(
                                                    if has_key {
                                                        t!("ncai.providers.apiKeyConfigured")
                                                    } else {
                                                        t!("ncai.providers.noApiKey")
                                                    },
                                                    if has_key {
                                                        ui::green()
                                                    } else {
                                                        ui_colors.text_muted
                                                    },
                                                ))
                                                .when_some(
                                                    p.default_model
                                                        .clone()
                                                        .filter(|m| !m.is_empty()),
                                                    |d, m| {
                                                        d.child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(ui_colors.text_muted)
                                                                .child("|"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(ui_colors.text_muted)
                                                                .truncate()
                                                                .child(m),
                                                        )
                                                    },
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .items_center()
                                        .child(
                                            Button::new(("ai-provider-edit", ix))
                                                .ghost()
                                                .small()
                                                .icon(Icon::new(IconName::Pencil))
                                                .tooltip(t!("ncai.providers.configure"))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.toggle_edit(&id_edit, window, cx)
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new(("ai-provider-remove", ix))
                                                .ghost()
                                                .small()
                                                .icon(Icon::new(IconName::Trash))
                                                .tooltip(t!("ncai.providers.remove"))
                                                .on_click(move |_, window, cx| {
                                                    let (id, name) =
                                                        (id_remove.clone(), name_remove.clone());
                                                    let _ = w1.update(cx, |this, cx| {
                                                        this.confirm_remove(id, name, window, cx)
                                                    });
                                                }),
                                        )
                                        .child(ui::switch(
                                            ("ai-provider-switch", ix),
                                            p.enabled,
                                            move |on, _, cx| {
                                                let id = id_toggle.clone();
                                                let _ = w2.update(cx, |this, cx| {
                                                    this.toggle_enabled(&id, on, cx)
                                                });
                                            },
                                        )),
                                ),
                        )
                        .when(editing, |d| {
                            d.when_some(self.form.clone(), |d, f| d.child(f))
                        })
                        .into_any_element()
                }))
                .into_any_element()
        };

        let _ = &theme;
        v_flex()
            .id("ai-providers-tab")
            .test_support()
            .gap(px(24.))
            .child(ui::section(
                "ai-providers",
                t!("ncai.providers"),
                Some(add_button.into_any_element()),
                vec![
                    menu.map(|m| m.into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                    list,
                ],
                cx,
            ))
    }
}

// ---------------------------------------------------------------------------
// Provider form
// ---------------------------------------------------------------------------

pub struct FormClosed;

pub struct ProviderForm {
    store: Entity<PrefsStore>,
    original: ProviderConfig,
    name: Entity<InputState>,
    style: Entity<ChoiceSelect>,
    openai_api: Entity<ChoiceSelect>,
    api_key: Entity<InputState>,
    base_url: Entity<InputState>,
    model: Entity<InputState>,
    model_pick: Entity<ChoiceSelect>,
    context_window: Entity<InputState>,
    skip_tls: bool,
    headers: Vec<(Entity<InputState>, Entity<InputState>)>,
    advanced_open: bool,
    max_tokens: Entity<InputState>,
    temperature: Entity<InputState>,
    top_p: Entity<InputState>,
    frequency_penalty: Entity<InputState>,
    presence_penalty: Entity<InputState>,
    reasoning: Entity<ChoiceSelect>,
    fetched: Vec<String>,
    context_windows: BTreeMap<String, u64>,
    status: Option<(ProbeHealth, String)>,
    testing: bool,
    fetching: bool,
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FormClosed> for ProviderForm {}

fn style_choices(resolved: &str) -> Vec<Choice> {
    let mut v = vec![Choice::new(
        "",
        format!(
            "{} ({})",
            t!("ncai.providers.style.inherited"),
            t!(&format!("ncai.providers.style.{resolved}"))
        ),
    )];
    v.extend(
        provider::STYLES
            .iter()
            .map(|s| Choice::new(*s, t!(&format!("ncai.providers.style.{s}")))),
    );
    v
}

fn api_choices() -> Vec<Choice> {
    vec![
        Choice::new("chat", t!("ncai.providers.openaiApi.chat")),
        Choice::new("responses", t!("ncai.providers.openaiApi.responses")),
    ]
}

fn reasoning_choices() -> Vec<Choice> {
    vec![
        Choice::new("", t!("ncai.providers.advancedParams.default")),
        Choice::new("low", "Low"),
        Choice::new("medium", "Medium"),
        Choice::new("high", "High"),
    ]
}

fn fmt_num(v: Option<f64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

/// `HeaderRow` validation: token names, no duplicates, none the app sets itself.
pub fn headers_valid(rows: &[(String, String)]) -> bool {
    let mut seen = std::collections::HashSet::new();
    rows.iter()
        .filter(|(n, v)| !(n.trim().is_empty() && v.trim().is_empty()))
        .all(|(name, _)| {
            let n = name.trim();
            let token = !n.is_empty()
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c));
            let reserved = matches!(
                n.to_ascii_lowercase().as_str(),
                "content-length" | "transfer-encoding"
            );
            token && !reserved && seen.insert(n.to_ascii_lowercase())
        })
}

impl ProviderForm {
    pub fn new(
        store: Entity<PrefsStore>,
        p: ProviderConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let resolved = provider::resolve_style(&p);
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.providers.name.placeholder"))
        });
        let style = cx.new(|cx| {
            new_select(
                style_choices(resolved),
                p.style.as_deref().unwrap_or(""),
                window,
                cx,
            )
        });
        let openai_api =
            cx.new(|cx| new_select(api_choices(), provider::resolve_openai_api(&p), window, cx));
        let api_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ncai.providers.apiKey.placeholder"))
                .masked(true)
        });
        let preset = provider::preset(&p.provider_id);
        let base_url = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                preset
                    .map(|x| x.base_url)
                    .filter(|u| !u.is_empty())
                    .unwrap_or("https://api.example.com/v1"),
            )
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.providers.defaultModel.placeholder"))
        });
        let model_pick = cx.new(|cx| new_select(Vec::new(), "", window, cx));
        let context_window = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("ncai.providers.contextWindow.placeholder"))
        });
        let max_tokens = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ncai.providers.advancedParams.maxTokens.placeholder"))
        });
        let temperature = cx.new(|cx| InputState::new(window, cx).placeholder("0 – 2"));
        let top_p = cx.new(|cx| InputState::new(window, cx).placeholder("0 – 1"));
        let frequency_penalty = cx.new(|cx| InputState::new(window, cx).placeholder("-2 – 2"));
        let presence_penalty = cx.new(|cx| InputState::new(window, cx).placeholder("-2 – 2"));
        let advanced = p.advanced_params.clone().unwrap_or_default();
        let reasoning = cx.new(|cx| {
            new_select(
                reasoning_choices(),
                advanced.reasoning_effort.as_deref().unwrap_or(""),
                window,
                cx,
            )
        });

        name.update(cx, |s, cx| s.set_value(p.name.clone(), window, cx));
        api_key.update(cx, |s, cx| {
            s.set_value(p.api_key.clone().unwrap_or_default(), window, cx)
        });
        base_url.update(cx, |s, cx| {
            s.set_value(p.base_url.clone().unwrap_or_default(), window, cx)
        });
        model.update(cx, |s, cx| {
            s.set_value(p.default_model.clone().unwrap_or_default(), window, cx)
        });
        context_window.update(cx, |s, cx| {
            s.set_value(
                p.context_window.map(|c| c.to_string()).unwrap_or_default(),
                window,
                cx,
            )
        });
        max_tokens.update(cx, |s, cx| {
            s.set_value(
                advanced
                    .max_tokens
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        temperature.update(cx, |s, cx| {
            s.set_value(fmt_num(advanced.temperature), window, cx)
        });
        top_p.update(cx, |s, cx| s.set_value(fmt_num(advanced.top_p), window, cx));
        frequency_penalty.update(cx, |s, cx| {
            s.set_value(fmt_num(advanced.frequency_penalty), window, cx)
        });
        presence_penalty.update(cx, |s, cx| {
            s.set_value(fmt_num(advanced.presence_penalty), window, cx)
        });

        let headers = p
            .custom_headers
            .iter()
            .map(|(n, v)| {
                let name_input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("ncai.providers.headers.name"))
                });
                let value_input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(t!("ncai.providers.headers.value"))
                        .masked(true)
                });
                name_input.update(cx, |s, cx| s.set_value(n.clone(), window, cx));
                value_input.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
                (name_input, value_input)
            })
            .collect();

        let subs = vec![
            cx.subscribe_in(&style, window, |this, _, ev, window, cx| {
                if confirmed_value(ev).is_some() {
                    this.refresh_style_labels(window, cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&model_pick, window, |this, _, ev, window, cx| {
                if let Some(m) = confirmed_value(ev).filter(|m| !m.is_empty()) {
                    this.model.update(cx, |s, cx| s.set_value(m, window, cx));
                }
            }),
        ];
        let mut this = Self {
            store,
            original: p.clone(),
            name,
            style,
            openai_api,
            api_key,
            base_url,
            model,
            model_pick,
            context_window,
            skip_tls: p.skip_tls_verify,
            headers,
            advanced_open: p.advanced_params.as_ref().is_some_and(|a| !a.is_empty()),
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            reasoning,
            fetched: Vec::new(),
            context_windows: p.model_context_windows.clone(),
            status: None,
            testing: false,
            fetching: false,
            error: None,
            _subs: subs,
        };
        this.refresh_model_choices(window, cx);
        this
    }

    // ---- accessors for tests ----

    pub fn name(&self) -> &Entity<InputState> {
        &self.name
    }
    pub fn api_key(&self) -> &Entity<InputState> {
        &self.api_key
    }
    pub fn base_url(&self) -> &Entity<InputState> {
        &self.base_url
    }
    pub fn model(&self) -> &Entity<InputState> {
        &self.model
    }
    pub fn context_window(&self) -> &Entity<InputState> {
        &self.context_window
    }
    pub fn temperature(&self) -> &Entity<InputState> {
        &self.temperature
    }
    pub fn fetched_models(&self) -> &[String] {
        &self.fetched
    }
    pub fn status(&self) -> Option<&(ProbeHealth, String)> {
        self.status.as_ref()
    }
    pub fn busy(&self) -> bool {
        self.testing || self.fetching
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn set_skip_tls(&mut self, on: bool, cx: &mut Context<Self>) {
        self.skip_tls = on;
        cx.notify();
    }

    fn style_value(&self, cx: &App) -> Option<String> {
        self.style
            .read(cx)
            .selected_value()
            .cloned()
            .filter(|s: &String| !s.is_empty())
    }

    fn refresh_style_labels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        let resolved = provider::resolve_style(&draft);
        let selected = draft.style.unwrap_or_default();
        self.style.update(cx, |s, cx| {
            s.set_items(style_choices(resolved), window, cx);
            s.set_selected_value(&selected, window, cx);
        });
    }

    fn refresh_model_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        let mut ids: Vec<String> = provider::preset_models(&draft);
        for m in &self.fetched {
            if !ids.contains(m) {
                ids.push(m.clone());
            }
        }
        let choices: Vec<Choice> = ids.into_iter().map(|m| Choice::new(m.clone(), m)).collect();
        self.model_pick
            .update(cx, |s, cx| s.set_items(choices, window, cx));
    }

    /// The provider as the form currently describes it (nothing is validated or stored).
    pub fn draft(&self, cx: &App) -> ProviderConfig {
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_string();
        let opt = |s: String| Some(s).filter(|s| !s.is_empty());
        let mut p = self.original.clone();
        p.name = text(&self.name);
        p.style = self.style_value(cx);
        p.api_key = opt(self.api_key.read(cx).value().to_string());
        p.base_url = opt(text(&self.base_url));
        p.default_model = opt(text(&self.model));
        p.skip_tls_verify = self.skip_tls;
        p.context_window = provider::sanitize_context_window(&text(&self.context_window))
            .ok()
            .flatten();
        p.model_context_windows = self.context_windows.clone();
        let resolved_style = provider::resolve_style(&p);
        p.openai_api = (resolved_style == "openai")
            .then(|| {
                self.openai_api
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .unwrap_or_else(|| "chat".into())
            })
            .filter(|a| a == "responses");
        p.custom_headers = self
            .headers
            .iter()
            .map(|(n, v)| (text(n), v.read(cx).value().to_string()))
            .filter(|(n, v)| !(n.is_empty() && v.trim().is_empty()))
            .collect();
        let num = |i: &Entity<InputState>| text(i).parse::<f64>().ok();
        let advanced = AdvancedParams {
            max_tokens: text(&self.max_tokens).parse::<u32>().ok(),
            temperature: num(&self.temperature),
            top_p: num(&self.top_p),
            frequency_penalty: num(&self.frequency_penalty),
            presence_penalty: num(&self.presence_penalty),
            reasoning_effort: self
                .reasoning
                .read(cx)
                .selected_value()
                .cloned()
                .filter(|r: &String| !r.is_empty()),
        }
        .clamped();
        p.advanced_params = Some(advanced).filter(|a| !a.is_empty());
        p
    }

    pub fn add_header(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("ncai.providers.headers.name")));
        let v = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("ncai.providers.headers.value"))
                .masked(true)
        });
        self.headers.push((n, v));
        cx.notify();
    }

    pub fn remove_header(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.headers.len() {
            self.headers.remove(ix);
        }
        cx.notify();
    }

    pub fn fetch_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fetching {
            return;
        }
        let draft = self.draft(cx);
        if let Err(issue) = provider::validate_probe_inputs(&draft) {
            self.status = Some((ProbeHealth::Error, issue_text(issue)));
            cx.notify();
            return;
        }
        self.fetching = true;
        self.status = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { provider::discover_models(&draft).await })
                .await
                .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching = false;
                match result {
                    Ok(found) => {
                        this.fetched = found.ids;
                        this.context_windows.extend(found.context_windows);
                        this.status = None;
                        this.refresh_model_choices(window, cx);
                    }
                    Err(e) => this.status = Some((ProbeHealth::Error, e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.testing {
            return;
        }
        let draft = self.draft(cx);
        if let Err(issue) = provider::validate_probe_inputs(&draft) {
            self.status = Some((ProbeHealth::Error, issue_text(issue)));
            cx.notify();
            return;
        }
        self.testing = true;
        self.status = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { provider::probe(&draft).await }).await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.testing = false;
                this.status = Some(match result {
                    Ok(r) => match r.health {
                        ProbeHealth::Ok => (
                            ProbeHealth::Ok,
                            t!("ncai.providers.test.ok", latency = r.latency_ms),
                        ),
                        ProbeHealth::Warn if r.model_count == Some(0) || r.error.is_some() => (
                            ProbeHealth::Warn,
                            t!("ncai.providers.test.warn", latency = r.latency_ms),
                        ),
                        ProbeHealth::Warn => (
                            ProbeHealth::Warn,
                            t!("ncai.providers.test.warnSlow", latency = r.latency_ms),
                        ),
                        ProbeHealth::Error => (
                            ProbeHealth::Error,
                            t!(
                                "ncai.providers.test.error",
                                detail = r.error.unwrap_or_default()
                            ),
                        ),
                    },
                    Err(_) => (ProbeHealth::Error, t!("ncai.providers.test.unavailable")),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Validates and stores the form; closes it on success.
    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_string();
        self.error = None;
        if provider::sanitize_context_window(&text(&self.context_window)).is_err() {
            self.error = Some(t!("ncai.providers.contextWindow.error"));
            cx.notify();
            return false;
        }
        let rows: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(n, v)| (text(n), v.read(cx).value().to_string()))
            .collect();
        if !headers_valid(&rows) {
            self.error = Some(t!("ncai.providers.headers.invalid"));
            cx.notify();
            return false;
        }
        let mut draft = self.draft(cx);
        if draft.name.is_empty() {
            draft.name = provider::preset(&draft.provider_id)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| "Custom".into());
        }
        if draft.provider_id == "ollama" {
            draft.base_url = draft
                .base_url
                .map(|u| provider::normalize_ollama_base_url(&u));
        }
        let id = draft.id.clone();
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                let is_active = p.active_provider_id == id;
                let model = draft.default_model.clone();
                if let Some(slot) = p.provider_mut(&id) {
                    // the stored on/off state is not part of the form
                    let enabled = slot.enabled;
                    *slot = draft;
                    slot.enabled = enabled;
                }
                if is_active {
                    p.active_model_id = model.unwrap_or_default();
                }
            })
        });
        cx.emit(FormClosed);
        true
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(FormClosed);
    }
}

fn issue_text(issue: ProbeInputIssue) -> String {
    match issue {
        ProbeInputIssue::MissingBaseUrl => t!("ncai.providers.test.missingBaseUrl"),
        ProbeInputIssue::MissingApiKey => t!("ncai.providers.test.missingApiKey"),
    }
}

impl Render for ProviderForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_colors = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let draft = self.draft(cx);
        let style = provider::resolve_style(&draft);
        let weak = cx.entity().downgrade();

        let base_help = match (draft.provider_id.as_str(), style) {
            ("ollama", _) => Some(t!("ncai.providers.baseUrl.ollamaHelp")),
            (_, "anthropic") => Some(t!("ncai.providers.baseUrl.anthropicHelp")),
            _ => None,
        };
        let header_rows: Vec<AnyElement> = self
            .headers
            .iter()
            .enumerate()
            .map(|(ix, (n, v))| {
                let w = weak.clone();
                h_flex()
                    .id(("ai-header-row", ix))
                    .test_support()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        div()
                            .w(px(180.))
                            .child(gpui_kit::component::input::Input::new(n)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(gpui_kit::component::input::Input::new(v).mask_toggle()),
                    )
                    .child(
                        Button::new(("ai-header-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::X))
                            .tooltip(t!("ncai.providers.headers.remove"))
                            .on_click(move |_, _, cx| {
                                let _ = w.update(cx, |this, cx| this.remove_header(ix, cx));
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        let advanced_open = self.advanced_open;
        let advanced = v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .id("ai-advanced-toggle")
                    .test_support()
                    .gap(px(6.))
                    .items_center()
                    .cursor_pointer()
                    .child(
                        Icon::new(if advanced_open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .text_color(ui_colors.text_muted),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ui_colors.text_secondary)
                            .child(t!("ncai.providers.advancedParams")),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced_open = !this.advanced_open;
                        cx.notify();
                    })),
            )
            .when(advanced_open, |d| {
                d.child(ui::muted_text(t!("ncai.providers.advancedParams.hint"), cx))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .flex_wrap()
                            .child(ui::stacked(
                                "max_tokens",
                                None,
                                ui::input(&self.max_tokens, 140.),
                                cx,
                            ))
                            .child(ui::stacked(
                                "temperature",
                                None,
                                ui::input(&self.temperature, 120.),
                                cx,
                            ))
                            .child(ui::stacked("top_p", None, ui::input(&self.top_p, 120.), cx))
                            .child(ui::stacked(
                                "frequency_penalty",
                                None,
                                ui::input(&self.frequency_penalty, 140.),
                                cx,
                            ))
                            .child(ui::stacked(
                                "presence_penalty",
                                None,
                                ui::input(&self.presence_penalty, 140.),
                                cx,
                            ))
                            .child(ui::stacked(
                                "reasoning_effort",
                                None,
                                div().w(px(160.)).child(Select::new(&self.reasoning)),
                                cx,
                            )),
                    )
            });

        let status = self.status.clone().map(|(health, text)| {
            let color = match health {
                ProbeHealth::Ok => ui::green(),
                ProbeHealth::Warn => ui::amber(),
                ProbeHealth::Error => theme.danger,
            };
            div()
                .id("ai-provider-status")
                .test_support()
                .child(ui::colored_text(text, color))
        });

        v_flex()
            .id("ai-provider-form")
            .test_support()
            .gap(px(12.))
            .pt(px(12.))
            .border_t_1()
            .border_color(ui_colors.border)
            .child(ui::stacked(
                t!("ncai.providers.name"),
                None,
                gpui_kit::component::input::Input::new(&self.name),
                cx,
            ))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_1().min_w_0().child(ui::stacked(
                        t!("ncai.providers.style"),
                        Some(t!("ncai.providers.style.help")),
                        Select::new(&self.style),
                        cx,
                    )))
                    .when(style == "openai", |d| {
                        d.child(div().flex_1().min_w_0().child(ui::stacked(
                            t!("ncai.providers.openaiApi"),
                            Some(t!("ncai.providers.openaiApi.help")),
                            Select::new(&self.openai_api),
                            cx,
                        )))
                    }),
            )
            .child(ui::stacked(
                t!("ncai.providers.apiKey"),
                None,
                gpui_kit::component::input::Input::new(&self.api_key).mask_toggle(),
                cx,
            ))
            .child(ui::stacked(
                t!("ncai.providers.baseUrl"),
                base_help,
                gpui_kit::component::input::Input::new(&self.base_url),
                cx,
            ))
            .child(ui::stacked(
                t!("ncai.providers.defaultModel"),
                None,
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(gpui_kit::component::input::Input::new(&self.model)),
                    )
                    .child(
                        div().w(px(200.)).child(
                            Select::new(&self.model_pick)
                                .placeholder(t!("ncai.providers.clickToLoadModels")),
                        ),
                    )
                    .child(
                        Button::new("ai-provider-fetch")
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .tooltip(t!("ncai.providers.refreshModels"))
                            .loading(self.fetching)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fetch_models(window, cx)),
                            ),
                    ),
                cx,
            ))
            .child(ui::stacked(
                t!("ncai.providers.contextWindow"),
                Some(t!("ncai.providers.contextWindow.help")),
                ui::input(&self.context_window, 200.),
                cx,
            ))
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_center()
                    .child(ui::switch("ai-provider-tls", self.skip_tls, {
                        let w = weak.clone();
                        move |on, _, cx| {
                            let _ = w.update(cx, |this, cx| this.set_skip_tls(on, cx));
                        }
                    }))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(ui_colors.text_secondary)
                            .child(t!("ncai.providers.skipTLSVerify")),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ui_colors.text_secondary)
                            .child(t!("ncai.providers.headers")),
                    )
                    .child(ui::muted_text(t!("ncai.providers.headers.help"), cx))
                    .children(header_rows)
                    .child(
                        h_flex().child(
                            Button::new("ai-header-add")
                                .small()
                                .icon(Icon::new(IconName::Plus))
                                .label(t!("ncai.providers.headers.add"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_header(window, cx)),
                                ),
                        ),
                    ),
            )
            .child(advanced)
            .when_some(self.error.clone(), |d, e| {
                d.child(ui::colored_text(e, theme.danger))
            })
            .when_some(status, |d, s| d.child(s))
            .child(
                h_flex()
                    .gap(px(8.))
                    .justify_end()
                    .child(
                        Button::new("ai-provider-test")
                            .small()
                            .icon(Icon::new(IconName::Zap))
                            .label(if self.testing {
                                t!("ncai.providers.test.testing")
                            } else {
                                t!("ncai.providers.test")
                            })
                            .disabled(self.testing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.test_connection(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("ai-provider-cancel")
                            .small()
                            .label(t!("common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        Button::new("ai-provider-save")
                            .small()
                            .primary()
                            .label(t!("common.save"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save(cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::headers_valid;

    fn rows(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn header_rows_are_validated() {
        assert!(headers_valid(&rows(&[
            ("X-Org", "1"),
            ("Authorization-Extra", "2")
        ])));
        assert!(headers_valid(&rows(&[("", ""), ("X-A", "v")])));
        assert!(!headers_valid(&rows(&[("X-A", "1"), ("x-a", "2")])));
        assert!(!headers_valid(&rows(&[("Content-Length", "1")])));
        assert!(!headers_valid(&rows(&[("Transfer-Encoding", "chunked")])));
        assert!(!headers_valid(&rows(&[("bad name", "1")])));
        assert!(!headers_valid(&rows(&[("", "value-without-name")])));
    }
}
