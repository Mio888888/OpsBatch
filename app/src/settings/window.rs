//! The settings window: a section rail on the left, the selected section on the right (port of `SettingsPage`).

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex, Icon, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use super::appearance::AppearanceSection;
use super::danger::DangerSection;
use super::data::DataSection;
use super::general::GeneralSection;
use crate::ai::settings::AiSettingsView;
use crate::library::action_page::QuickActionsPage;
use crate::library::command_page::CommandLibPage;
use crate::library::script_page::ScriptLibPage;
use crate::window::{self, WindowKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    QuickActions,
    CommandLib,
    DockerLib,
    ScriptLib,
    Ai,
    Danger,
    Data,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::General,
        Section::Appearance,
        Section::QuickActions,
        Section::CommandLib,
        Section::DockerLib,
        Section::ScriptLib,
        Section::Ai,
        Section::Danger,
        Section::Data,
    ];

    fn key(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::QuickActions => "quickActions",
            Section::CommandLib => "commandLib",
            Section::DockerLib => "dockerLib",
            Section::ScriptLib => "scriptLib",
            Section::Ai => "ai",
            Section::Danger => "danger",
            Section::Data => "data",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::General => IconName::SlidersHorizontal,
            Section::Appearance => IconName::Palette,
            Section::QuickActions => IconName::Zap,
            Section::CommandLib => IconName::Code,
            Section::DockerLib => IconName::Boxes,
            Section::ScriptLib => IconName::FileText,
            Section::Ai => IconName::Bot,
            Section::Danger => IconName::TriangleAlert,
            Section::Data => IconName::Database,
        }
    }
}

struct OpenSettings(WeakEntity<SettingsWindow>);
impl Global for OpenSettings {}

pub struct SettingsWindow {
    section: Section,
    general: Entity<GeneralSection>,
    appearance: Entity<AppearanceSection>,
    danger: Entity<DangerSection>,
    data: Entity<DataSection>,
    quick_actions: Entity<QuickActionsPage>,
    commands: Entity<CommandLibPage>,
    docker: Entity<CommandLibPage>,
    scripts: Entity<ScriptLibPage>,
    ai: Entity<AiSettingsView>,
}

impl SettingsWindow {
    pub fn open(section: Section, cx: &mut App) {
        let options = window::window_options(WindowKind::Settings, &t!("nav.settings"), cx);
        window::open_or_focus("settings", options, cx, move |window, cx| cx.new(|cx| Self::new(section, window, cx)));
        // an already open window jumps to the requested section
        if let Some(open) = cx.try_global::<OpenSettings>().and_then(|g| g.0.upgrade()) {
            open.update(cx, |this, cx| this.select(section, cx));
        }
    }

    pub fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let weak = cx.entity().downgrade();
        cx.set_global(OpenSettings(weak));
        Self {
            section,
            general: cx.new(|cx| GeneralSection::new(window, cx)),
            appearance: cx.new(|cx| AppearanceSection::new(window, cx)),
            danger: cx.new(|cx| DangerSection::new(window, cx)),
            data: cx.new(|cx| DataSection::new(window, cx)),
            quick_actions: cx.new(|cx| QuickActionsPage::new(window, cx)),
            commands: cx.new(|cx| CommandLibPage::new(false, window, cx)),
            docker: cx.new(|cx| CommandLibPage::new(true, window, cx)),
            scripts: cx.new(|cx| ScriptLibPage::new(window, cx)),
            ai: cx.new(|cx| AiSettingsView::new(window, cx)),
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn select(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    pub fn general(&self) -> &Entity<GeneralSection> {
        &self.general
    }
    pub fn appearance(&self) -> &Entity<AppearanceSection> {
        &self.appearance
    }
    pub fn danger(&self) -> &Entity<DangerSection> {
        &self.danger
    }
    pub fn data(&self) -> &Entity<DataSection> {
        &self.data
    }
    pub fn ai(&self) -> &Entity<AiSettingsView> {
        &self.ai
    }
    pub fn commands(&self) -> &Entity<CommandLibPage> {
        &self.commands
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        // library pages manage their own padding/scrolling; the form sections get a scrolling column
        let (body, full_bleed): (AnyElement, bool) = match self.section {
            Section::General => (self.general.clone().into_any_element(), false),
            Section::Appearance => (self.appearance.clone().into_any_element(), false),
            Section::Danger => (self.danger.clone().into_any_element(), false),
            Section::Data => (self.data.clone().into_any_element(), false),
            Section::Ai => (self.ai.clone().into_any_element(), false),
            Section::QuickActions => (self.quick_actions.clone().into_any_element(), true),
            Section::CommandLib => (self.commands.clone().into_any_element(), true),
            Section::DockerLib => (self.docker.clone().into_any_element(), true),
            Section::ScriptLib => (self.scripts.clone().into_any_element(), true),
        };
        let current = self.section;
        // `.settings-sidebar-card`: 208px, sidebar colour, 50px top padding for the window buttons
        let nav = v_flex().id("settings-nav").test_support().w(px(208.)).flex_shrink_0().h_full().pt(px(50.)).pb(px(14.)).px(px(10.)).gap(px(1.)).border_r_1().border_color(ui.border).bg(ui.sidebar).children(Section::ALL.iter().enumerate().map(|(ix, s)| {
            let s = *s;
            let active = s == current;
            h_flex()
                .id(("settings-section", ix))
                .test_support()
                .relative()
                .w_full()
                .h(px(32.))
                .gap(px(10.))
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .py(px(6.))
                .pr(px(10.))
                .pl(px(12.))
                .rounded(px(8.))
                .items_center()
                .cursor_pointer()
                .text_color(if active { ui.primary } else { ui.text_secondary })
                .when(active, |d| d.bg(ui.input_bg))
                .when(!active, |d| d.hover(move |d| d.bg(ui.nav_hover).text_color(ui.text)))
                // the 2x16 accent bar at the left edge of the selected item
                .when(active, |d| d.child(div().absolute().left(px(3.)).top_1_2().mt(px(-8.)).w(px(2.)).h(px(16.)).rounded(px(2.)).bg(ui.primary)))
                .child(Icon::new(s.icon()).size_4().opacity(if active { 1.0 } else { 0.72 }))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.3)).child(t!(&format!("settings.sections.{}", s.key()))))
                .on_click(cx.listener(move |this, _, _, cx| this.select(s, cx)))
        }));
        // `.settings-main-card` / `.settings-main-body`: main colour, 18/24/24 padding
        let main = div()
            .id("settings-body")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(ui.main)
            .border_1()
            .border_color(ui.border)
            .map(|d| {
                if full_bleed {
                    // embedded library pages: same 18/24/24 padding, the list scrolls inside its card
                    d.pt(px(18.)).px(px(24.)).pb(px(24.)).child(div().size_full().child(body))
                } else {
                    d.overflow_y_scroll().pt(px(18.)).px(px(24.)).pb(px(24.)).child(div().max_w(px(640.)).child(body))
                }
            });
        // The window has no title bar of its own: the traffic lights float over the sidebar and the
        // transparent strip at the top still drags the window.
        div()
            .id("settings-window")
            .test_support()
            .relative()
            .size_full()
            .bg(ui.page)
            .text_color(ui.text)
            .child(h_flex().size_full().child(nav).child(main))
            .child(TitleBar::new().absolute().top_0().left_0().right_0().h(px(34.)).bg(gpui_kit::transparent_black()).border_b_0())
    }
}
