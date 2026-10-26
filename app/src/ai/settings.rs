//! AI provider settings (the `ai` section of `SettingsPage.tsx`). `AiSettingsView` is what the settings window of
//! M10 embeds.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::assets::IconName;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::client::{ai_chat, ai_list_models, get_ai_config, save_ai_config, AiConfig, ChatMessage, ChatRequest};
use ops_i18n::t;

use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::toast;

pub struct ProviderDefaults {
    pub api_url: &'static str,
    pub model: &'static str,
    pub requires_key: bool,
    pub description_key: &'static str,
}

pub fn provider_defaults(provider: &str) -> ProviderDefaults {
    match provider {
        "openai" => ProviderDefaults { api_url: "https://api.openai.com/v1", model: "gpt-4o-mini", requires_key: true, description_key: "settings.provider.openaiDesc" },
        "ollama" => ProviderDefaults { api_url: "http://localhost:11434/v1", model: "qwen3:8b", requires_key: false, description_key: "settings.provider.ollamaDesc" },
        _ => ProviderDefaults { api_url: "", model: "custom", requires_key: true, description_key: "settings.provider.customDesc" },
    }
}

/// Preset models of a provider as `(value, label)`; the trailing `custom` entry lets the user type a model name.
pub fn model_presets(provider: &str) -> Vec<(String, String)> {
    let p = |v: &str, l: &str| (v.to_string(), l.to_string());
    let mut list = match provider {
        "ollama" => vec![p("qwen3:8b", "Qwen3 8B"), p("deepseek-r1:8b", "DeepSeek R1 8B"), p("llama3.1:8b", "Llama 3.1 8B"), p("mistral:7b", "Mistral 7B")],
        "openai" => vec![p("gpt-4o", "GPT-4o"), p("gpt-4o-mini", "GPT-4o Mini"), p("gpt-4.1", "GPT-4.1"), p("gpt-4.1-mini", "GPT-4.1 Mini"), p("o3", "o3"), p("o4-mini", "o4-mini")],
        _ => vec![
            p("deepseek-chat", "DeepSeek Chat"),
            p("deepseek-reasoner", "DeepSeek Reasoner"),
            p("qwen-turbo", "通义千问 Turbo"),
            p("qwen-plus", "通义千问 Plus"),
            p("claude-sonnet-4-20250514", "Claude Sonnet 4"),
            p("gpt-4o", "GPT-4o"),
        ],
    };
    list.push(p("custom", &t!("settings.customModel")));
    list
}

/// Trailing slashes and a pasted `/chat/completions` suffix are not part of the base URL.
pub fn normalize_api_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    trimmed.strip_suffix("/chat/completions").unwrap_or(trimmed).to_string()
}

fn choices(options: &[(String, String)]) -> Vec<Choice> {
    options.iter().map(|(v, l)| Choice::new(v.clone(), l.clone())).collect()
}

fn provider_choices() -> Vec<Choice> {
    vec![Choice::new("openai", "OpenAI"), Choice::new("ollama", t!("settings.ollamaLocal")), Choice::new("custom", t!("settings.customApi"))]
}

pub struct AiSettingsView {
    enabled: bool,
    provider: String,
    /// Selected model value; `custom` means "use the custom model name field".
    model: String,
    fetched_models: Vec<String>,
    loaded: bool,
    testing: bool,
    fetching: bool,
    api_url: Entity<InputState>,
    api_key: Entity<InputState>,
    custom_model: Entity<InputState>,
    provider_select: Entity<ChoiceSelect>,
    model_select: Entity<ChoiceSelect>,
    _subs: Vec<Subscription>,
}

impl AiSettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_url = cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1"));
        let api_key = cx.new(|cx| InputState::new(window, cx).placeholder("sk-...").masked(true));
        let custom_model = cx.new(|cx| InputState::new(window, cx).placeholder(t!("settings.customModelPlaceholder")));
        let provider_select = cx.new(|cx| new_select(provider_choices(), "openai", window, cx));
        let model_select = cx.new(|cx| new_select(choices(&model_presets("openai")), "gpt-4o-mini", window, cx));
        let subs = vec![
            cx.subscribe_in(&provider_select, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.choose_provider(&v, window, cx);
                }
            }),
            cx.subscribe_in(&model_select, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.model = v;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&api_url, window, |_, input, ev: &InputEvent, window, cx| {
                if let InputEvent::Blur = ev {
                    let normalized = normalize_api_url(input.read(cx).value().as_ref());
                    input.update(cx, |s, cx| s.set_value(normalized, window, cx));
                }
            }),
        ];
        let mut this = Self {
            enabled: false,
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            fetched_models: Vec::new(),
            loaded: false,
            testing: false,
            fetching: false,
            api_url,
            api_key,
            custom_model,
            provider_select,
            model_select,
            _subs: subs,
        };
        this.load(window, cx);
        this
    }

    // ---- accessors for tests ----

    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn api_url(&self) -> &Entity<InputState> {
        &self.api_url
    }
    pub fn api_key(&self) -> &Entity<InputState> {
        &self.api_key
    }
    pub fn custom_model(&self) -> &Entity<InputState> {
        &self.custom_model
    }
    pub fn fetched_models(&self) -> &[String] {
        &self.fetched_models
    }
    /// The stored configuration has been read into the form.
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    pub fn busy(&self) -> bool {
        self.testing || self.fetching
    }

    fn model_options(&self) -> Vec<(String, String)> {
        if self.fetched_models.is_empty() {
            model_presets(&self.provider)
        } else {
            let mut list: Vec<(String, String)> = self.fetched_models.iter().map(|m| (m.clone(), m.clone())).collect();
            list.push(("custom".into(), t!("settings.customModel")));
            list
        }
    }

    fn refresh_model_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = choices(&self.model_options());
        let selected = self.model.clone();
        self.model_select.update(cx, |s, cx| {
            s.set_items(options, window, cx);
            s.set_selected_value(&selected, window, cx);
        });
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { get_ai_config(&ctx.db).await }).await.and_then(|r| r);
            let Ok(config) = result else { return };
            let _ = this.update_in(cx, |this, window, cx| this.apply_config(config, window, cx));
        })
        .detach();
    }

    fn apply_config(&mut self, config: AiConfig, window: &mut Window, cx: &mut Context<Self>) {
        let preset = model_presets(&config.provider).iter().any(|(v, _)| *v == config.model);
        self.loaded = true;
        self.enabled = config.enabled;
        self.provider = config.provider.clone();
        self.fetched_models.clear();
        if !preset && !config.model.is_empty() {
            self.model = "custom".into();
            let model = config.model.clone();
            self.custom_model.update(cx, |s, cx| s.set_value(model, window, cx));
        } else {
            self.model = config.model.clone();
            self.custom_model.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let (url, key) = (config.api_url.clone(), config.api_key.clone());
        self.api_url.update(cx, |s, cx| s.set_value(url, window, cx));
        self.api_key.update(cx, |s, cx| s.set_value(key, window, cx));
        let provider = config.provider;
        self.provider_select.update(cx, |s, cx| s.set_selected_value(&provider, window, cx));
        self.refresh_model_select(window, cx);
        cx.notify();
    }

    fn choose_provider(&mut self, provider: &str, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = provider_defaults(provider);
        self.provider = provider.to_string();
        self.model = defaults.model.to_string();
        self.fetched_models.clear();
        if defaults.model != "custom" {
            self.custom_model.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.api_url.update(cx, |s, cx| s.set_value(defaults.api_url, window, cx));
        if provider == "ollama" {
            self.api_key.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.refresh_model_select(window, cx);
        cx.notify();
    }

    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = enabled;
        cx.notify();
    }

    /// The model name the form currently stands for.
    fn effective_model(&self, cx: &App) -> String {
        if self.model == "custom" {
            self.custom_model.read(cx).value().trim().to_string()
        } else {
            self.model.clone()
        }
    }

    pub fn fetch_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fetching {
            return;
        }
        self.fetching = true;
        let url = self.api_url.read(cx).value().to_string();
        let key = self.api_key.read(cx).value().to_string();
        let provider = self.provider.clone();
        let ctx = crate::services::ctx(cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let api_url = Some(url).filter(|u| !u.trim().is_empty());
            let api_key = Some(key).filter(|k| !k.is_empty() && !k.contains("****"));
            let result = ops_core::rt::run(async move { ai_list_models(&ctx.db, api_url, api_key, Some(provider)).await }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching = false;
                match result {
                    Ok(mut models) => {
                        models.sort_by(|a, b| a.id.cmp(&b.id));
                        let count = models.len();
                        this.fetched_models = models.into_iter().map(|m| m.id).collect();
                        if count > 0 {
                            toast::success(window, cx, t!("settings.fetchModelsSuccess", count = count));
                        } else {
                            toast::warning(window, cx, t!("settings.fetchModelsEmpty"));
                        }
                        this.refresh_model_select(window, cx);
                    }
                    Err(e) => toast::error(window, cx, t!("settings.fetchModelsFailed", error = e)),
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
        let model = self.effective_model(cx);
        if model.is_empty() {
            toast::error(window, cx, t!("settings.aiSelectModelFirst"));
            return;
        }
        self.testing = true;
        let ctx = crate::services::ctx(cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let request = ChatRequest {
                messages: vec![ChatMessage { role: "user".into(), content: t!("settings.aiTestPrompt"), images: None }],
                model: Some(model),
                temperature: None,
                max_tokens: None,
            };
            let result = ops_core::rt::run(async move { ai_chat(&ctx.db, request).await }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.testing = false;
                match result {
                    Ok(r) => toast::success(window, cx, t!("settings.aiTestSuccess", content = r.content)),
                    Err(e) => toast::error(window, cx, t!("settings.aiTestFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = provider_defaults(&self.provider);
        let api_url = normalize_api_url(self.api_url.read(cx).value().as_ref());
        let key = self.api_key.read(cx).value().to_string();
        if api_url.is_empty() {
            toast::error(window, cx, t!("settings.aiMissingApiUrl"));
            return;
        }
        if defaults.requires_key && key.trim().is_empty() {
            toast::error(window, cx, t!("settings.apiKeyRequired"));
            return;
        }
        let model = self.effective_model(cx);
        if self.model == "custom" && model.is_empty() {
            toast::error(window, cx, t!("settings.aiMissingCustomModel"));
            return;
        }
        let config = AiConfig {
            provider: self.provider.clone(),
            api_url,
            api_key: if self.provider == "ollama" { String::new() } else { key },
            model,
            enabled: self.enabled,
        };
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move {
                save_ai_config(&ctx.db, config).await?;
                get_ai_config(&ctx.db).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(saved) => {
                    toast::success(window, cx, t!("settings.aiSaved"));
                    this.apply_config(saved, window, cx);
                }
                Err(e) => toast::error(window, cx, e),
            });
        })
        .detach();
    }
}

impl Render for AiSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let defaults = provider_defaults(&self.provider);
        let custom = self.model == "custom";
        let label_of = |value: &str, options: &[(String, String)]| options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_else(|| value.to_string());
        let current_model = if custom {
            let name = self.custom_model.read(cx).value().trim().to_string();
            if name.is_empty() { t!("settings.customModelMissing") } else { name }
        } else {
            label_of(&self.model, &self.model_options())
        };
        let enabled = self.enabled;
        let ui = crate::theme::ui(cx);
        let this_weak = cx.entity().downgrade();
        use crate::settings::{group_with, row};
        // `.settings-ai-cred-row`: title + description on top, the control full width underneath
        let cred_row = |title: String, desc: Option<String>, control: AnyElement| {
            v_flex()
                .gap(px(4.))
                .py(px(8.))
                .child(
                    v_flex()
                        .gap(px(1.))
                        .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).line_height(relative(1.3)).text_color(ui.text_secondary).child(title))
                        .when_some(desc, |d, desc| d.child(div().text_size(px(11.)).line_height(relative(1.35)).text_color(ui.text_muted).child(desc))),
                )
                .child(control)
                .into_any_element()
        };
        let hint = div()
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.primary.opacity(0.12))
            .bg(ui.primary.opacity(0.06))
            .text_size(px(12.))
            .line_height(relative(1.6))
            .text_color(ui.text_secondary)
            .child(t!(defaults.description_key));
        let subsection = |children: Vec<AnyElement>| {
            v_flex().gap(px(10.)).px(px(14.)).py(px(12.)).rounded(px(14.)).border_1().border_color(ui.border).bg(ui.text_muted.opacity(0.03)).children(children).into_any_element()
        };
        let summary = h_flex()
            .id("ai-summary")
            .test_support()
            .flex_wrap()
            .gap_x(px(20.))
            .gap_y(px(6.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.text_muted.opacity(0.04))
            .text_size(px(12.))
            .line_height(relative(1.6))
            .child(h_flex().gap(px(6.)).items_baseline().child(div().text_color(ui.text_muted).child(t!("settings.currentModel"))).child(div().id("ai-current-model").test_support().font_weight(FontWeight::SEMIBOLD).text_color(ui.text).child(current_model)))
            .child(h_flex().gap(px(6.)).items_baseline().child(div().text_color(ui.text_muted).child(t!("settings.auth"))).child(div().text_color(ui.text).child(if defaults.requires_key { "Bearer API Key".to_string() } else { t!("settings.apiKeyNotRequired") })));

        v_flex()
            .id("ai-settings")
            .test_support()
            .w_full()
            .gap(px(20.))
            .child(group_with(
                t!("settings.aiGroupProvider"),
                t!("settings.aiGroupProviderDesc"),
                vec![
                    row(
                        t!("settings.aiEnabled"),
                        None,
                        Switch::new("ai-enabled").checked(enabled).on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let _ = this_weak.update(cx, |this, cx| this.set_enabled(checked, cx));
                        }),
                        cx,
                    ),
                    row(t!("settings.aiProvider"), None, div().w(px(160.)).child(Select::new(&self.provider_select)), cx),
                    hint.into_any_element(),
                    cred_row(t!("settings.apiUrl"), Some(t!("settings.aiGroupCredDesc")), Input::new(&self.api_url).into_any_element()),
                    cred_row(
                        "API Key".to_string(),
                        None,
                        if defaults.requires_key {
                            Input::new(&self.api_key).into_any_element()
                        } else {
                            div().h(px(32.)).px(px(11.)).flex().items_center().rounded(px(8.)).border_1().border_color(ui.border).text_size(px(13.)).text_color(ui.text_muted).child(t!("settings.apiKeyNotRequired")).into_any_element()
                        },
                    ),
                ],
                false,
                cx,
            ))
            .child(group_with(
                t!("settings.aiGroupModel"),
                t!("settings.aiGroupModelDesc"),
                vec![
                    subsection(vec![
                        row(
                            t!("settings.model"),
                            Some(t!("settings.aiModelFetchDesc").into()),
                            h_flex()
                                .gap(px(8.))
                                .items_center()
                                .child(div().w(px(200.)).child(Select::new(&self.model_select)))
                                .child(Button::new("ai-fetch-models").icon(Icon::new(IconName::RefreshCw)).label(t!("settings.fetchModels")).tooltip(t!("settings.fetchModelsTitle")).loading(self.fetching).on_click(cx.listener(|this, _, window, cx| this.fetch_models(window, cx)))),
                            cx,
                        ),
                        if custom { cred_row(t!("settings.customModelName"), None, Input::new(&self.custom_model).into_any_element()) } else { div().into_any_element() },
                        h_flex().justify_end().child(Button::new("ai-test").icon(Icon::new(IconName::Zap)).label(t!("settings.testConnection")).loading(self.testing).on_click(cx.listener(|this, _, window, cx| this.test_connection(window, cx)))).into_any_element(),
                    ]),
                    subsection(vec![
                        summary.into_any_element(),
                        h_flex().justify_end().child(Button::new("ai-save").primary().icon(Icon::new(IconName::Save)).label(t!("common.saveSettings")).disabled(self.testing).on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))).into_any_element(),
                    ]),
                ],
                false,
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{model_presets, normalize_api_url, provider_defaults};

    #[test]
    fn api_urls_lose_trailing_slashes_and_chat_suffix() {
        assert_eq!(normalize_api_url(" https://x.test/v1/ "), "https://x.test/v1");
        assert_eq!(normalize_api_url("https://x.test/v1/chat/completions/"), "https://x.test/v1");
        assert_eq!(normalize_api_url(""), "");
    }

    #[test]
    fn provider_defaults_match_the_tauri_build() {
        assert_eq!(provider_defaults("openai").model, "gpt-4o-mini");
        assert!(!provider_defaults("ollama").requires_key);
        assert_eq!(provider_defaults("anything").model, "custom");
        let presets = model_presets("openai");
        assert_eq!(presets.last().map(|(v, _)| v.as_str()), Some("custom"));
        assert!(model_presets("custom").iter().any(|(v, _)| v == "deepseek-chat"));
    }
}
