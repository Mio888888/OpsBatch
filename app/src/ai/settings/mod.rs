//! AI settings (port of Netcatty's `SettingsAITab`): providers, agents, tool access, web search and safety.
//! `AiSettingsView` is what the settings window embeds; the five tabs share one [`PrefsStore`].

mod agents;
mod mcp;
mod providers;
mod quick_messages;
mod safety;
mod search;
mod tools;
pub mod ui;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use super::prefs_store::{PrefsEvent, PrefsStore};
use crate::widgets::toast;

pub use agents::AgentsTab;
pub use providers::ProvidersTab;
pub use safety::SafetyTab;
pub use search::SearchTab;
pub use tools::ToolsTab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiTab {
    Providers,
    Agents,
    Tools,
    Search,
    Safety,
}

impl AiTab {
    pub const ALL: [AiTab; 5] = [AiTab::Providers, AiTab::Agents, AiTab::Tools, AiTab::Search, AiTab::Safety];

    fn label(self) -> String {
        match self {
            AiTab::Providers => t!("ncai.providers"),
            AiTab::Agents => t!("ncai.agents"),
            AiTab::Tools => t!("ncai.toolAccess.title"),
            AiTab::Search => t!("ncai.webSearch.title"),
            AiTab::Safety => t!("ncai.safety.title"),
        }
    }
}

pub struct AiSettingsView {
    store: Entity<PrefsStore>,
    tab: AiTab,
    providers: Entity<ProvidersTab>,
    agents: Entity<AgentsTab>,
    tools: Entity<ToolsTab>,
    search: Entity<SearchTab>,
    safety: Entity<SafetyTab>,
    _subs: Vec<Subscription>,
}

impl AiSettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = super::prefs_store::shared(cx);
        let subs = vec![cx.subscribe_in(&store, window, |_, _, ev: &PrefsEvent, window, cx| {
            if let PrefsEvent::SaveFailed(e) = ev {
                toast::error(window, cx, t!("ncai.providers.headers.saveError").to_string() + " " + e);
            }
        })];
        Self {
            providers: cx.new(|cx| ProvidersTab::new(store.clone(), window, cx)),
            agents: cx.new(|cx| AgentsTab::new(store.clone(), window, cx)),
            tools: cx.new(|cx| ToolsTab::new(store.clone(), window, cx)),
            search: cx.new(|cx| SearchTab::new(store.clone(), window, cx)),
            safety: cx.new(|cx| SafetyTab::new(store.clone(), window, cx)),
            store,
            tab: AiTab::Providers,
            _subs: subs,
        }
    }

    pub fn store(&self) -> &Entity<PrefsStore> {
        &self.store
    }
    pub fn tab(&self) -> AiTab {
        self.tab
    }
    pub fn select_tab(&mut self, tab: AiTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }
    /// The stored preferences have been read.
    pub fn loaded(&self, cx: &App) -> bool {
        self.store.read(cx).loaded()
    }
    pub fn providers(&self) -> &Entity<ProvidersTab> {
        &self.providers
    }
    pub fn agents(&self) -> &Entity<AgentsTab> {
        &self.agents
    }
    pub fn tools(&self) -> &Entity<ToolsTab> {
        &self.tools
    }
    pub fn search(&self) -> &Entity<SearchTab> {
        &self.search
    }
    pub fn safety(&self) -> &Entity<SafetyTab> {
        &self.safety
    }
}

impl Render for AiSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<(AiTab, String)> = AiTab::ALL.iter().map(|t| (*t, t.label())).collect();
        let weak = cx.entity().downgrade();
        let bar = ui::tab_bar("ai-tabs", &tabs, self.tab, move |tab, _, cx| {
            let _ = weak.update(cx, |this, cx| this.select_tab(tab, cx));
        }, cx);
        let body: AnyElement = match self.tab {
            AiTab::Providers => self.providers.clone().into_any_element(),
            AiTab::Agents => self.agents.clone().into_any_element(),
            AiTab::Tools => self.tools.clone().into_any_element(),
            AiTab::Search => self.search.clone().into_any_element(),
            AiTab::Safety => self.safety.clone().into_any_element(),
        };
        v_flex().id("ai-settings").test_support().w_full().gap(px(20.)).child(bar).child(body)
    }
}
