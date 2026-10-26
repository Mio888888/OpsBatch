//! Main window: title bar (performance chips, mode tabs, tools), optional asset sidebar and the page area.

pub mod asset_panel;
pub mod motion;
pub mod perf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use crate::asset_tree::HostKind;
use crate::stores::assets::AssetsStore;
use crate::terminal::page::{PageEvent, TerminalPage};
use crate::terminal::workspace::workspace;
use asset_panel::{AssetPanel, AssetPanelEvent};
use perf::PerfMonitor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Terminal,
    Workflow,
    Github,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Terminal, Mode::Workflow, Mode::Github];

    pub fn label_key(self) -> &'static str {
        match self {
            Mode::Terminal => "nav.terminal",
            Mode::Workflow => "nav.workflow",
            Mode::Github => "nav.github",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Mode::Terminal => IconName::Code,
            Mode::Workflow => IconName::Archive, // ApartmentOutlined = lucide Archive
            Mode::Github => IconName::Code, // GithubOutlined = lucide Code
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Mode::Terminal => "mode-terminal",
            Mode::Workflow => "mode-workflow",
            Mode::Github => "mode-github",
        }
    }
}

pub struct Workbench {
    mode: Mode,
    assets: Entity<AssetsStore>,
    asset_panel: Entity<AssetPanel>,
    asset_panel_visible: bool,
    perf: Entity<PerfMonitor>,
    terminal: Option<Entity<TerminalPage>>,
    repo_sync: Option<Entity<crate::repo_sync::page::RepoSyncPage>>,
    workflow: Option<Entity<crate::workflow::page::WorkflowPage>>,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl Workbench {
    pub fn new(assets: Entity<AssetsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let asset_panel = cx.new(|cx| AssetPanel::new(assets.clone(), window, cx));
        let perf = cx.new(PerfMonitor::new);
        let subs = vec![cx.subscribe_in(&asset_panel, window, |this, _, ev: &AssetPanelEvent, window, cx| {
            match ev {
                AssetPanelEvent::Close => this.set_asset_panel_visible(false, cx),
                AssetPanelEvent::OpenHost(id) => this.open_host(id, window, cx),
                AssetPanelEvent::BatchUpload(ids) => {
                    if ids.is_empty() {
                        crate::widgets::toast::warning(window, cx, t!("assets.selectLinuxHostsFirst"));
                    } else {
                        crate::transfer::window::BatchTransferWindow::open(ids.clone(), cx);
                    }
                }
                AssetPanelEvent::BroadcastTerminal(ids) => {
                    if ids.is_empty() {
                        crate::widgets::toast::warning(window, cx, t!("assets.selectLinuxHostsFirst"));
                    } else {
                        crate::broadcast::window::BatchTerminalWindow::open(ids.clone(), cx);
                    }
                }
            }
        })];
        let (mode, visible) = {
            let ui = cx.global::<crate::ui_state::UiState>();
            (ui.mode, ui.asset_panel_visible)
        };
        if visible {
            assets.update(cx, |s, cx| s.reload(cx));
        }
        // RDP / VNC windows ask for the asset sidebar through the event bus (the original `open-asset-manager` event).
        let open_assets = crate::services::ctx(cx).events.subscribe(crate::window::OPEN_ASSET_MANAGER_TOPIC);
        let task = cx.spawn(async move |this, cx| {
            while open_assets.recv_async().await.is_ok() {
                if this.update(cx, |this, cx| this.set_asset_panel_visible(true, cx)).is_err() {
                    break;
                }
            }
        });
        let update_sub = cx.observe(&crate::update::update_store(cx), |_, _, cx| cx.notify());
        let mut subs = subs;
        subs.push(update_sub);
        let mut this = Self { mode, assets, asset_panel, asset_panel_visible: visible, perf, terminal: None, repo_sync: None, workflow: None, _subs: subs, _tasks: vec![task] };
        if mode == Mode::Terminal {
            this.ensure_terminal_page(window, cx);
        }
        this
    }

    fn ensure_terminal_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TerminalPage> {
        if let Some(page) = &self.terminal {
            return page.clone();
        }
        let ws = workspace(cx);
        let page = cx.new(|cx| TerminalPage::new(ws, window, cx));
        let sub = cx.subscribe(&page, |this, _, ev: &PageEvent, cx| match ev {
            PageEvent::ShowAssets => this.set_asset_panel_visible(true, cx),
            PageEvent::OpenEditor { host_id, dir, path } => crate::editor::window::EditorWindow::open(host_id, *dir, path, cx),
        });
        self._subs.push(sub);
        self.terminal = Some(page.clone());
        page
    }

    pub fn workflow_page(&self) -> Option<&Entity<crate::workflow::page::WorkflowPage>> {
        self.workflow.as_ref()
    }

    pub fn terminal_page(&self) -> Option<&Entity<TerminalPage>> {
        self.terminal.as_ref()
    }

    /// Open a host from the sidebar: Linux hosts get a terminal tab, Windows hosts an RDP window, VNC hosts a VNC window.
    pub fn open_host(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.assets.read(cx).host(id).cloned() else { return };
        self.assets.update(cx, |s, cx| s.record_recent(&host.id, cx));
        match HostKind::of(&host) {
            HostKind::Windows => {
                crate::rdp::window::RdpWindow::open(crate::rdp::window::HostRequest { host_id: host.id.clone(), name: host.name.clone(), ip: host.ip.clone() }, cx);
                return;
            }
            HostKind::Vnc => {
                crate::vnc::window::VncWindow::open(
                    crate::vnc::window::VncHost { host_id: host.id.clone(), name: host.name.clone(), ip: host.ip.clone(), port: ops_vnc::config::vnc_port_from_settings(&host.rdp_settings) },
                    cx,
                );
                return;
            }
            HostKind::Linux => {}
        }
        self.set_mode(Mode::Terminal, cx);
        self.ensure_terminal_page(window, cx);
        workspace(cx).update(cx, |w, cx| {
            w.open_host(&host.id, &host.name, &host.ip, cx);
        });
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            cx.global_mut::<crate::ui_state::UiState>().mode = mode;
            cx.notify();
        }
    }

    pub fn asset_panel_visible(&self) -> bool {
        self.asset_panel_visible
    }

    /// Showing the sidebar reloads the data, like the Tauri build did.
    pub fn set_asset_panel_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible && !self.asset_panel_visible {
            self.assets.update(cx, |s, cx| s.reload(cx));
        }
        self.asset_panel_visible = visible;
        cx.global_mut::<crate::ui_state::UiState>().asset_panel_visible = visible;
        cx.notify();
    }

    pub fn assets(&self) -> &Entity<AssetsStore> {
        &self.assets
    }

    fn page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.mode == Mode::Terminal {
            let page = self.ensure_terminal_page(window, cx);
            return motion::fade_in(div().flex_1().min_w_0().h_full().child(page), ("page-fade", 0usize), cx);
        }
        if self.mode == Mode::Workflow {
            let page = match &self.workflow {
                Some(p) => p.clone(),
                None => {
                    let p = cx.new(|cx| crate::workflow::page::WorkflowPage::new(window, cx));
                    self.workflow = Some(p.clone());
                    p
                }
            };
            return motion::fade_in(div().flex_1().min_w_0().h_full().child(page), ("page-fade", 1usize), cx);
        }
        if self.mode == Mode::Github {
            let page = self.repo_sync.get_or_insert_with(|| cx.new(crate::repo_sync::page::RepoSyncPage::new)).clone();
            return motion::fade_in(div().flex_1().min_w_0().h_full().child(page), ("page-fade", 2usize), cx);
        }
        let theme = cx.theme().clone();
        let (title, note): (String, &str) = match self.mode {
            Mode::Terminal => (String::new(), ""),
            Mode::Workflow => (t!("workflow.workflowListTitle"), "workflow: milestone M9"),
            Mode::Github => (t!("repoSync.title"), "repo sync: milestone M6"),
        };
        v_flex()
            .id("page")
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_center()
            .gap_2()
            .child(div().text_size(px(16.)).child(title))
            .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(note))
            .when(self.mode == Mode::Terminal && !self.asset_panel_visible, |d| {
                d.child(
                    Button::new("open-assets")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Plus))
                        .label(t!("assets.title"))
                        .on_click(cx.listener(|this, _, _, cx| this.set_asset_panel_visible(true, cx))),
                )
            })
            .into_any_element()
    }
}

/// One entry of the pill-shaped mode switch (`.workbench-mode-tab`).
fn mode_tab(m: Mode, active: bool, ui: &crate::theme::UiColors, cx: &mut Context<Workbench>) -> impl IntoElement {
    let ui = *ui;
    h_flex()
        .id(m.id())
        .test_support()
        .h(px(28.))
        .px(px(14.))
        .gap(px(5.))
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if active { gpui_kit::white() } else { ui.text_secondary })
        .when(active, |d| d.bg(linear_gradient(135., linear_color_stop(ui.primary, 0.), linear_color_stop(ui.primary_strong, 1.))).shadow(vec![BoxShadow { color: ui.primary.opacity(0.28), offset: point(px(0.), px(8.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }]))
        .when(!active, |d| d.hover(|d| d.bg(ui.nav_hover)))
        .child(Icon::new(m.icon()).size_4())
        .child(t!(m.label_key()))
        .on_click(cx.listener(move |this, _, _, cx| this.set_mode(m, cx)))
}

/// A 28px icon button of the title bar (`.tool-icon-button`).
fn tool_button(id: &'static str, icon: IconName, tip: String, ui: &crate::theme::UiColors, badge: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let ui = *ui;
    div()
        .id(id)
        .test_support()
        .relative()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .cursor_pointer()
        .text_color(ui.text)
        .bg(ui.input_bg)
        .border_1()
        .border_color(ui.border)
        .hover(move |d| d.bg(ui.primary).border_color(ui.primary).text_color(gpui_kit::white()).shadow(vec![BoxShadow { color: ui.primary.opacity(0.28), offset: point(px(0.), px(8.)), blur_radius: px(18.), spread_radius: px(0.), inset: false }]))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .child(Icon::new(icon).size_4())
        .when(badge, |d| d.child(div().id("update-dot").test_support().absolute().top(px(4.)).right(px(4.)).size(px(7.)).rounded_full().bg(gpui_kit::rgb(0xff4d4f)).border_1().border_color(ui.card)))
        .on_click(on_click)
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let mode = self.mode;

        // center: pill with the three work modes
        let tabs = h_flex()
            .id("mode-tabs")
            .p(px(2.))
            .items_center()
            .rounded_full()
            .bg(ui.input_bg)
            .border_1()
            .border_color(ui.border)
            .children(Mode::ALL.map(|m| mode_tab(m, m == mode, &ui, cx).into_any_element()));

        let (has_update, latest) = {
            let u = crate::update::update_store(cx).read(cx);
            (u.has_update(), u.info.as_ref().and_then(|i| i.latest_version.clone()).unwrap_or_default())
        };
        let update_tip = if has_update { t!("appUpdate.availableTooltip", version = latest) } else { t!("appUpdate.check") };
        // right: log, settings, update (red dot when a release is waiting)
        let tools = h_flex()
            .gap(px(10.))
            .items_center()
            .child(tool_button("tool-log", IconName::FileText, t!("nav.globalLog"), &ui, false, |_, _, cx| crate::log::window::GlobalLogWindow::open(cx)))
            .child(tool_button("tool-settings", IconName::Settings, t!("nav.settings"), &ui, false, |_, _, cx| crate::settings::window::SettingsWindow::open(crate::settings::window::Section::General, cx)))
            .child(tool_button("tool-update", IconName::CircleFadingArrowUp, update_tip, &ui, has_update, |_, window, cx| drop(crate::update::UpdateModal::open(window, cx))));

        // macOS keeps its native traffic lights (placed like the original's own 12px dots): room for them
        // (the three columns stay equal so the mode pill is centred on the window)
        let left_pad = if cfg!(target_os = "macos") { px(78.) } else { px(0.) };

        v_flex()
            .size_full()
            .bg(ui.page)
            .text_color(ui.text)
            .child(
                TitleBar::new().h(px(38.)).pl(px(16.)).pr(px(16.)).bg(ui.page).border_b_0().child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().min_w_0().flex().justify_start().child(div().ml(left_pad).child(self.perf.clone())))
                        .child(tabs)
                        .child(div().flex_1().min_w_0().flex().justify_end().child(tools)),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.asset_panel_visible, |d| d.child(motion::fade_in(div().h_full().child(self.asset_panel.clone()), "asset-panel-fade", cx)))
                    // `.workbench-shell`: the main panel is a rounded sheet on the window background
                    .child(div().flex_1().min_w_0().h_full().bg(ui.main).rounded_tl(px(14.)).overflow_hidden().child(self.page(window, cx))),
            )
    }
}
