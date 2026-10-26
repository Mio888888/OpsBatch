//! Web search tab (`WebSearchSettings`).

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::prefs::search::{search_preset, WebSearchConfig, MAX_RESULTS_LIMIT, SEARCH_PRESETS};
use ops_i18n::t;

use super::ui;
use crate::ai::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

pub struct SearchTab {
    store: Entity<PrefsStore>,
    provider: Entity<ChoiceSelect>,
    api_key: Entity<InputState>,
    api_host: Entity<InputState>,
    max_results: Entity<InputState>,
    _subs: Vec<Subscription>,
}

fn provider_choices() -> Vec<Choice> {
    SEARCH_PRESETS.iter().map(|p| Choice::new(p.id, p.name)).collect()
}

impl SearchTab {
    pub fn new(store: Entity<PrefsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = cx.new(|cx| new_select(provider_choices(), "tavily", window, cx));
        let api_key = cx.new(|cx| InputState::new(window, cx).placeholder(t!("ncai.webSearch.apiKey.placeholder")).masked(true));
        let api_host = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let max_results = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![
            cx.subscribe_in(&store, window, |this, _, ev: &PrefsEvent, window, cx| {
                if matches!(ev, PrefsEvent::Loaded) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&provider, window, |this, _, ev, window, cx| {
                if let Some(id) = confirmed_value(ev) {
                    this.choose_provider(&id, window, cx);
                }
            }),
            ui::on_commit(&api_key, window, cx, |this, text, _, cx| {
                let key = Some(text.trim().to_string()).filter(|k| !k.is_empty());
                this.update(cx, |c| c.api_key = key);
            }),
            ui::on_commit(&api_host, window, cx, |this, text, _, cx| {
                let host = Some(text.trim().to_string()).filter(|h| !h.is_empty());
                this.update(cx, |c| c.api_host = host);
            }),
            ui::on_commit(&max_results, window, cx, |this, text, window, cx| {
                match ui::parse_int(&text).filter(|n| (1..=MAX_RESULTS_LIMIT as i64).contains(n)) {
                    Some(n) => this.update(cx, |c| c.max_results = Some(n as u32)),
                    None => this.sync(window, cx),
                }
            }),
        ];
        let mut this = Self { store, provider, api_key, api_host, max_results, _subs: subs };
        this.sync(window, cx);
        this
    }

    pub fn config(&self, cx: &App) -> WebSearchConfig {
        self.store.read(cx).prefs().web_search.clone().unwrap_or_default()
    }

    pub fn api_key(&self) -> &Entity<InputState> {
        &self.api_key
    }
    pub fn api_host(&self) -> &Entity<InputState> {
        &self.api_host
    }
    pub fn max_results(&self) -> &Entity<InputState> {
        &self.max_results
    }

    /// Pushes the stored values into the fields.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let c = self.config(cx);
        let preset = search_preset(&c.provider_id);
        self.provider.update(cx, |s, cx| s.set_selected_value(&c.provider_id, window, cx));
        ui::set_input(&self.api_key, c.api_key.as_deref().unwrap_or(""), window, cx);
        ui::set_input(&self.api_host, c.api_host.as_deref().unwrap_or(preset.default_api_host), window, cx);
        ui::set_input(&self.max_results, &c.max_results.unwrap_or(5).to_string(), window, cx);
        cx.notify();
    }

    fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut WebSearchConfig)) {
        self.store.update(cx, |s, cx| {
            s.update(cx, |p| {
                let mut config = p.web_search.take().unwrap_or_default();
                f(&mut config);
                p.web_search = Some(config);
            })
        });
    }

    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.update(cx, |c| c.enabled = enabled);
    }

    /// Switching provider resets the key and host to that provider's defaults.
    pub fn choose_provider(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.update(cx, |c| c.switch_provider(&id));
        self.sync(window, cx);
    }
}

impl Render for SearchTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config(cx);
        let preset = config.preset();
        let weak = cx.entity().downgrade();
        let mut rows = vec![
            ui::setting_row(
                Some(t!("ncai.webSearch.enable")),
                Some(t!("ncai.webSearch.enable.description")),
                ui::switch("ai-search-enabled", config.enabled, {
                    let weak = weak.clone();
                    move |on, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.set_enabled(on, cx));
                    }
                }),
                cx,
            ),
            ui::setting_row(Some(t!("ncai.webSearch.provider")), Some(t!("ncai.webSearch.provider.description")), div().w(px(192.)).child(Select::new(&self.provider)), cx),
        ];
        if preset.requires_api_key {
            rows.push(ui::setting_row(Some(t!("ncai.webSearch.apiKey")), Some(t!("ncai.webSearch.apiKey.description")), div().w(px(256.)).child(Input::new(&self.api_key).mask_toggle()), cx));
        }
        rows.push(ui::setting_row(
            Some(t!("ncai.webSearch.apiHost")),
            Some(if config.provider_id == "searxng" { t!("ncai.webSearch.apiHost.searxngDescription") } else { t!("ncai.webSearch.apiHost.description") }),
            ui::input(&self.api_host, 256.),
            cx,
        ));
        rows.push(ui::setting_row(Some(t!("ncai.webSearch.maxResults")), Some(t!("ncai.webSearch.maxResults.description")), ui::input(&self.max_results, 80.), cx));
        v_flex().id("ai-search-tab").test_support().gap(px(24.)).child(ui::section("ai-web-search", t!("ncai.webSearch.title"), None, vec![ui::card(rows, true, cx).into_any_element()], cx))
    }
}
