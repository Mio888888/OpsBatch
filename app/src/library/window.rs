//! Window hosting the four library pages (until the settings window of M10 embeds them).

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Icon, Selectable as _, Sizable as _, TitleBar,
};
use gpui_kit::*;
use ops_i18n::t;

use super::action_page::QuickActionsPage;
use super::command_page::CommandLibPage;
use super::script_page::ScriptLibPage;
use crate::window::{self, WindowKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Commands,
    Scripts,
    Actions,
    Docker,
}

pub struct LibraryWindow {
    tab: LibraryTab,
    commands: Entity<CommandLibPage>,
    scripts: Entity<ScriptLibPage>,
    actions: Entity<QuickActionsPage>,
    docker: Entity<CommandLibPage>,
}

impl LibraryWindow {
    pub fn open(tab: LibraryTab, cx: &mut App) {
        let options = window::window_options(WindowKind::Library, &t!("library.windowTitle"), cx);
        window::open_or_focus("library", options, cx, move |window, cx| {
            cx.new(|cx| Self::new(tab, window, cx))
        });
    }

    pub fn new(tab: LibraryTab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            tab,
            commands: cx.new(|cx| CommandLibPage::new(false, window, cx)),
            scripts: cx.new(|cx| ScriptLibPage::new(window, cx)),
            actions: cx.new(|cx| QuickActionsPage::new(window, cx)),
            docker: cx.new(|cx| CommandLibPage::new(true, window, cx)),
        }
    }

    pub fn tab(&self) -> LibraryTab {
        self.tab
    }
    pub fn set_tab(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }
    pub fn commands(&self) -> &Entity<CommandLibPage> {
        &self.commands
    }
    pub fn scripts(&self) -> &Entity<ScriptLibPage> {
        &self.scripts
    }
    pub fn actions(&self) -> &Entity<QuickActionsPage> {
        &self.actions
    }
}

impl Render for LibraryWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tab_button = |id: &'static str,
                          label: String,
                          icon: IconName,
                          tab: LibraryTab,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .small()
                .icon(Icon::new(icon))
                .label(label)
                .selected(self.tab == tab)
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
        };
        let page: AnyElement = match self.tab {
            LibraryTab::Commands => self.commands.clone().into_any_element(),
            LibraryTab::Scripts => self.scripts.clone().into_any_element(),
            LibraryTab::Actions => self.actions.clone().into_any_element(),
            LibraryTab::Docker => self.docker.clone().into_any_element(),
        };
        v_flex()
            .id("library-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .gap_1()
                        .child(tab_button(
                            "lib-tab-commands",
                            t!("library.tabCommands"),
                            IconName::SquareTerminal,
                            LibraryTab::Commands,
                            cx,
                        ))
                        .child(tab_button(
                            "lib-tab-scripts",
                            t!("library.tabScripts"),
                            IconName::FileText,
                            LibraryTab::Scripts,
                            cx,
                        ))
                        .child(tab_button(
                            "lib-tab-actions",
                            t!("library.tabActions"),
                            IconName::Zap,
                            LibraryTab::Actions,
                            cx,
                        ))
                        .child(tab_button(
                            "lib-tab-docker",
                            t!("library.tabDocker"),
                            IconName::Cloud,
                            LibraryTab::Docker,
                            cx,
                        )),
                ),
            )
            .child(div().flex_1().min_h_0().child(page))
    }
}
