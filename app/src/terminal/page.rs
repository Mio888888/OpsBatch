//! The terminal page: tab bar, session area (single / split / state cards), search overlay, host monitor and
//! bottom panel. Port of `pages/Terminal/TerminalPage.tsx` + `BottomPanel.tsx`.

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable, ResizableState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_terminal::auth::is_ssh_authentication_failure;
use ops_terminal::tabs::TabCloseMode;

use super::auth_dialog::AuthRetryDialog;
use super::monitor_panel::HostMonitorPanel;
use super::pane::{pane_element_id, SearchResult, TerminalPane};
use super::workspace::{ConnState, TabKind, TerminalWorkspace, WorkspaceEvent};
use crate::ai::inline::{AiInlinePanel, InlineEvent};
use crate::ai::panel::{AiChatPanel, AiTarget};
use crate::ai::store::AiScope;
use crate::forward::panel::PortForwardPanel;
use crate::library::inline::{InlineKind, InlinePanel};
use crate::sftp::panel::{SftpPanel, SftpPanelEvent};
use crate::theme::css;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Sftp,
    Forward,
    Ai,
    Commands,
    Scripts,
    Docker,
}

impl BottomTab {
    fn label(self) -> String {
        match self {
            BottomTab::Sftp => "SFTP".into(),
            BottomTab::Forward => t!("terminal.tab.portForward"),
            BottomTab::Ai => "AI".into(),
            BottomTab::Commands => t!("terminal.tab.commands"),
            BottomTab::Scripts => t!("terminal.tab.scripts"),
            BottomTab::Docker => t!("terminal.tab.docker"),
        }
    }

    fn id(self) -> &'static str {
        match self {
            BottomTab::Sftp => "bottom-tab-sftp",
            BottomTab::Forward => "bottom-tab-forward",
            BottomTab::Ai => "bottom-tab-ai",
            BottomTab::Commands => "bottom-tab-commands",
            BottomTab::Scripts => "bottom-tab-scripts",
            BottomTab::Docker => "bottom-tab-docker",
        }
    }

    /// Local shells only get the tabs that do not need an SSH host.
    pub fn available(self, remote: bool) -> bool {
        remote
            || matches!(
                self,
                BottomTab::Ai | BottomTab::Commands | BottomTab::Scripts
            )
    }
}

#[derive(Debug, Clone)]
pub enum PageEvent {
    /// "+" button / "new connection" / "open assets sidebar": show the host list.
    ShowAssets,
    /// Open a remote file or directory of `host_id` in the editor window.
    OpenEditor {
        host_id: String,
        dir: bool,
        path: String,
    },
}

pub struct TerminalPage {
    workspace: Entity<TerminalWorkspace>,
    search_visible: bool,
    search_input: Entity<InputState>,
    search: SearchResult,
    search_query: String,
    bottom_tab: BottomTab,
    monitor: Option<Entity<HostMonitorPanel>>,
    sftp: Option<Entity<SftpPanel>>,
    sftp_sub: Option<Subscription>,
    forward: Option<Entity<PortForwardPanel>>,
    inline: std::collections::HashMap<&'static str, Entity<InlinePanel>>,
    h_state: Entity<ResizableState>,
    v_state: Entity<ResizableState>,
    ai_panel: Option<Entity<AiChatPanel>>,
    ai_inline: Option<Entity<AiInlinePanel>>,
    ai_inline_sub: Option<Subscription>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for TerminalPage {}

impl TerminalPage {
    pub fn new(
        workspace: Entity<TerminalWorkspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("terminal.searchPlaceholder")));
        let subs = vec![
            cx.subscribe_in(
                &workspace,
                window,
                |this, _, ev: &WorkspaceEvent, window, cx| match ev {
                    WorkspaceEvent::Changed => {
                        this.sync_monitor(cx);
                        this.ensure_panels(window, cx);
                        this.rerun_search(cx);
                        cx.notify();
                    }
                    WorkspaceEvent::Activated => this.focus_active(window, cx),
                },
            ),
            cx.subscribe_in(
                &search_input,
                window,
                |this, input, ev: &InputEvent, _, cx| match ev {
                    InputEvent::Change => {
                        this.search_query = input.read(cx).value().to_string();
                        this.run_search(false, None, cx);
                    }
                    InputEvent::PressEnter { shift, .. } => this.jump_search(*shift, cx),
                    _ => {}
                },
            ),
        ];
        workspace.update(cx, |w, cx| w.ensure_initial_tab(cx));
        let mut page = Self {
            workspace,
            search_visible: false,
            search_input,
            search: SearchResult::default(),
            search_query: String::new(),
            bottom_tab: BottomTab::Sftp,
            monitor: None,
            sftp: None,
            sftp_sub: None,
            forward: None,
            inline: std::collections::HashMap::new(),
            h_state: cx.new(|_| ResizableState::default()),
            v_state: cx.new(|_| ResizableState::default()),
            ai_panel: None,
            ai_inline: None,
            ai_inline_sub: None,
            _subs: subs,
        };
        page.sync_monitor(cx);
        page.ensure_panels(window, cx);
        page
    }

    // ---- accessors for tests / the workbench ----

    pub fn workspace(&self) -> &Entity<TerminalWorkspace> {
        &self.workspace
    }
    pub fn monitor(&self) -> Option<&Entity<HostMonitorPanel>> {
        self.monitor.as_ref()
    }
    pub fn bottom_tab(&self) -> BottomTab {
        self.bottom_tab
    }
    pub fn set_bottom_tab(&mut self, tab: BottomTab, window: &mut Window, cx: &mut Context<Self>) {
        self.bottom_tab = tab;
        self.ensure_panels(window, cx);
        cx.notify();
    }

    pub fn ai_panel(&self) -> Option<&Entity<AiChatPanel>> {
        self.ai_panel.as_ref()
    }
    pub fn ai_inline(&self) -> Option<&Entity<AiInlinePanel>> {
        self.ai_inline.as_ref()
    }

    /// The active terminal tab as an AI chat target.
    fn ai_target(&self, cx: &App) -> Option<AiTarget> {
        let w = self.workspace.read(cx);
        let tab = w.active_tab().filter(|t| t.state == ConnState::Connected)?;
        let pane = tab.pane.clone()?;
        let session_id = pane.read(cx).session_id.clone();
        let (scope, host) = match (tab.kind, &tab.host_id) {
            (TabKind::Remote, Some(host_id)) => (
                AiScope::new("ssh_host", host_id),
                Some((tab.host_name.clone(), tab.host_ip.clone())),
            ),
            _ => (AiScope::new("terminal_session", &session_id), None),
        };
        Some(AiTarget {
            session_id,
            scope,
            host,
            label: tab.host_name.clone(),
            pane: pane.downgrade(),
        })
    }

    fn sync_ai_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.ai_target(cx);
        if let Some(panel) = self.ai_panel.clone() {
            panel.update(cx, |p, cx| p.set_target(target.clone(), window, cx));
        }
        match (&self.ai_inline, target) {
            (Some(inline), Some(t)) => inline.update(cx, |i, _| i.set_target(t)),
            (Some(_), None) => self.close_inline(window, cx),
            _ => {}
        }
    }

    /// "AI analyze error" on the terminal selection: open the AI tab and start a turn about it.
    pub fn analyze_selection(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_bottom_tab(BottomTab::Ai, window, cx);
        if let Some(panel) = self.ai_panel.clone() {
            panel.update(cx, |p, cx| p.analyze_selection(&text, cx));
        }
    }

    pub fn open_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.ai_target(cx) else {
            return;
        };
        if let Some(existing) = &self.ai_inline {
            existing.update(cx, |i, cx| i.focus_input(window, cx));
            return;
        }
        let panel = cx.new(|cx| AiInlinePanel::new(target, window, cx));
        self.ai_inline_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, ev: &InlineEvent, window, cx| match ev {
                InlineEvent::Close => this.close_inline(window, cx),
                InlineEvent::Inject(command) => this.insert_into_terminal(command, window, cx),
            },
        ));
        self.ai_inline = Some(panel);
        cx.notify();
    }

    pub fn close_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_inline.take().is_some() {
            self.ai_inline_sub = None;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub fn toggle_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_inline.is_some() {
            self.close_inline(window, cx);
        } else {
            self.open_inline(window, cx);
        }
    }

    /// The command / script / docker panel currently created for this key (`commands`, `scripts`, `docker`).
    pub fn inline_panel(&self, key: &str) -> Option<&Entity<InlinePanel>> {
        self.inline.get(key)
    }

    /// Put a command on the active terminal's prompt (no Enter) and give the terminal focus.
    pub fn insert_into_terminal(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_pane(cx) else {
            return;
        };
        pane.update(cx, |p, cx| p.insert_command(command, cx));
        let handle = pane.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    pub fn forward_panel(&self) -> Option<&Entity<PortForwardPanel>> {
        self.forward.as_ref()
    }

    pub fn sftp_panel(&self) -> Option<&Entity<SftpPanel>> {
        self.sftp.as_ref()
    }

    /// Create the bottom-panel content for the active host when its tab is selected (and drop it when the host goes away).
    fn ensure_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let host = {
            let w = self.workspace.read(cx);
            w.active_tab()
                .filter(|t| t.state == ConnState::Connected && t.kind == TabKind::Remote)
                .and_then(|t| t.host_id.clone())
        };
        // a local shell has no SFTP/forward tabs: the panel falls back to AI, whose content must exist then
        let remote = self
            .workspace
            .read(cx)
            .active_tab()
            .is_some_and(|t| t.kind == TabKind::Remote);
        let effective = if self.bottom_tab.available(remote) {
            self.bottom_tab
        } else {
            BottomTab::Ai
        };
        match (&host, self.bottom_tab) {
            (Some(h), BottomTab::Sftp) => {
                if self
                    .sftp
                    .as_ref()
                    .map(|p| p.read(cx).host_id() != h)
                    .unwrap_or(true)
                {
                    let panel = cx.new(|cx| SftpPanel::new(h.clone(), window, cx));
                    self.sftp_sub =
                        Some(
                            cx.subscribe(&panel, |_, _, ev: &SftpPanelEvent, cx| match ev {
                                SftpPanelEvent::OpenEditor { host_id, dir, path } => {
                                    cx.emit(PageEvent::OpenEditor {
                                        host_id: host_id.clone(),
                                        dir: *dir,
                                        path: path.clone(),
                                    })
                                }
                            }),
                        );
                    self.sftp = Some(panel);
                }
            }
            (None, _) => {
                self.sftp = None;
                self.sftp_sub = None;
                self.forward = None;
            }
            _ => {}
        }
        if effective == BottomTab::Ai && self.ai_panel.is_none() {
            self.ai_panel = Some(cx.new(|cx| AiChatPanel::new(window, cx)));
        }
        self.sync_ai_target(window, cx);
        let kind = match self.bottom_tab {
            BottomTab::Commands => Some(("commands", InlineKind::Commands)),
            BottomTab::Scripts => Some(("scripts", InlineKind::Scripts)),
            BottomTab::Docker => Some(("docker", InlineKind::Docker)),
            _ => None,
        };
        if let Some((key, kind)) = kind {
            if !self.inline.contains_key(key) {
                let page = cx.entity().downgrade();
                let insert: crate::library::inline::InsertFn =
                    std::rc::Rc::new(move |text, window, cx| {
                        let _ = page.update(cx, |p, cx| p.insert_into_terminal(&text, window, cx));
                    });
                let panel = cx.new(|cx| InlinePanel::new(kind, insert, window, cx));
                self.inline.insert(key, panel);
            }
        }
        if let (Some(h), BottomTab::Forward) = (&host, self.bottom_tab) {
            if self
                .forward
                .as_ref()
                .map(|p| p.read(cx).host_id() != h)
                .unwrap_or(true)
            {
                self.forward = Some(cx.new(|cx| PortForwardPanel::new(h.clone(), window, cx)));
            }
        }
    }
    pub fn search_visible(&self) -> bool {
        self.search_visible
    }
    pub fn search_state(&self) -> SearchResult {
        self.search
    }

    fn active_pane(&self, cx: &App) -> Option<Entity<TerminalPane>> {
        self.workspace
            .read(cx)
            .active_tab()
            .and_then(|t| t.pane.clone())
    }

    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.active_pane(cx) {
            let handle = p.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        }
    }

    fn sync_monitor(&mut self, cx: &mut Context<Self>) {
        let want = {
            let w = self.workspace.read(cx);
            w.active_tab()
                .filter(|t| t.state == ConnState::Connected && t.kind == TabKind::Remote)
                .and_then(|t| t.host_id.clone().map(|id| (id, t.host_ip.clone())))
        };
        match (&self.monitor, want) {
            (Some(m), Some((id, _))) if m.read(cx).host_id() == id => {}
            (_, Some((id, ip))) => self.monitor = Some(cx.new(|_| HostMonitorPanel::new(id, ip))),
            (_, None) => self.monitor = None,
        }
    }

    // ---- search ----

    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ok = self
            .workspace
            .read(cx)
            .active_tab()
            .is_some_and(|t| t.state == ConnState::Connected && t.split.is_none());
        if !ok {
            return;
        }
        self.search_visible = true;
        let input = self.search_input.clone();
        input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.active_pane(cx) {
            p.update(cx, |p, _| p.clear_search());
        }
        self.search_visible = false;
        self.search_query.clear();
        self.search = SearchResult::default();
        let input = self.search_input.clone();
        input.update(cx, |s, cx| s.set_value("", window, cx));
        self.focus_active(window, cx);
        cx.notify();
    }

    fn run_search(&mut self, backwards: bool, from: Option<usize>, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane(cx) else {
            return;
        };
        let q = self.search_query.clone();
        self.search = pane.update(cx, |p, _| p.search(&q, backwards, from));
        cx.notify();
    }

    fn rerun_search(&mut self, cx: &mut Context<Self>) {
        if self.search_visible && !self.search_query.trim().is_empty() {
            self.run_search(false, None, cx);
        }
    }

    pub fn jump_search(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let from = self.search.match_index;
        self.run_search(backwards, from, cx);
    }

    // ---- actions ----

    fn show_assets(&mut self, cx: &mut Context<Self>) {
        cx.emit(PageEvent::ShowAssets);
    }

    fn open_split(&mut self, key: String, cx: &mut Context<Self>) {
        self.workspace.update(cx, |w, cx| w.open_split(&key, cx));
    }

    // ---- rendering helpers ----

    fn terminal_bg(cx: &App) -> Hsla {
        css(crate::theme::current(cx).resolved.theme.terminal.background)
    }

    fn pane_view(
        &self,
        tab_key: String,
        pane: Entity<TerminalPane>,
        split_ok: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity().downgrade();
        let sid = pane.read(cx).session_id.clone();
        let menu_pane = pane.clone();
        div()
            .id(ElementId::Name(format!("terminal-pane-box-{sid}").into()))
            .size_full()
            .min_w_0()
            .p(px(4.))
            .bg(Self::terminal_bg(cx))
            .child(pane)
            .context_menu(move |menu, _, cx| {
                let selection = menu_pane.read(cx).selection_text();
                let mut menu = menu;
                if let Some(text) = selection.clone() {
                    let copy = text.clone();
                    menu = menu.item(PopupMenuItem::new(t!("terminal.copy")).on_click(
                        move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        },
                    ));
                    // Settings → AI → Tool access → "show the AI action on a selection"
                    if crate::ai::runtime::limits(cx).show_selection_action {
                        menu = menu.separator();
                        let page = page.clone();
                        menu = menu.item(
                            PopupMenuItem::new(format!("🔍 {}", t!("terminal.aiAnalyzeError")))
                                .on_click(move |_, window, cx| {
                                    let text = text.clone();
                                    let _ = page.update(cx, |this, cx| {
                                        this.analyze_selection(text, window, cx)
                                    });
                                }),
                        );
                    }
                } else {
                    let p = menu_pane.clone();
                    menu = menu.item(PopupMenuItem::new(t!("terminal.paste")).on_click(
                        move |_, _, cx| {
                            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                                p.update(cx, |p, cx| p.paste_text(&text, cx));
                            }
                        },
                    ));
                }
                if split_ok {
                    let page = page.clone();
                    let key = tab_key.clone();
                    menu = menu.separator().item(
                        PopupMenuItem::new(t!("terminal.openSplit")).on_click(move |_, _, cx| {
                            let key = key.clone();
                            let _ = page.update(cx, |this, cx| this.open_split(key, cx));
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn search_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let base = div().absolute().top(px(10.)).right(px(14.));
        if !self.search_visible {
            return base
                .child({
                    // `.terminal-pane-search-button`
                    let ui = crate::theme::ui(cx);
                    div()
                        .id("terminal-search-open")
                        .test_support()
                        .size(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(ui.term_border)
                        .bg(gpui_kit::rgba(0x0a101abd))
                        .text_color(ui.text_secondary)
                        .opacity(0.78)
                        .cursor_pointer()
                        .hover(move |d| {
                            d.opacity(1.)
                                .text_color(ui.term_fg)
                                .bg(gpui_kit::rgba(0x141e2ceb))
                        })
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(t!("terminal.openSearch"))
                                .build(window, cx)
                        })
                        .child(Icon::new(IconName::Search).size_4())
                        .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx)))
                })
                .into_any_element();
        }
        let has_query = !self.search_query.trim().is_empty();
        let count = if !has_query {
            String::new()
        } else if self.search.match_count > 0 {
            format!(
                "{}/{}",
                self.search.match_index.map(|i| i + 1).unwrap_or(0),
                self.search.match_count
            )
        } else {
            t!("terminal.searchNoMatch")
        };
        let can_jump = has_query && self.search.match_count > 0;
        base.child(
            h_flex()
                .id("terminal-search-box")
                .test_support()
                .w(px(360.))
                .max_w_full()
                .h(px(34.))
                .px_1()
                .gap_1()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    this.close_search(window, cx);
                    cx.stop_propagation();
                }))
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "escape" {
                        this.close_search(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.search_input).small().appearance(false)),
                )
                .child(
                    div()
                        .min_w(px(42.))
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground)
                        .child(count),
                )
                .child(
                    Button::new("terminal-search-prev")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronUp))
                        .tooltip(t!("terminal.searchPrevious"))
                        .disabled(!can_jump)
                        .on_click(cx.listener(|this, _, _, cx| this.jump_search(true, cx))),
                )
                .child(
                    Button::new("terminal-search-next")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronDown))
                        .tooltip(t!("terminal.searchNext"))
                        .disabled(!can_jump)
                        .on_click(cx.listener(|this, _, _, cx| this.jump_search(false, cx))),
                )
                .child(
                    Button::new("terminal-search-close")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::X))
                        .tooltip(t!("common.close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
                ),
        )
        .into_any_element()
    }

    fn state_card(&self, header: String, light: Hsla, body: impl IntoElement, cx: &App) -> Div {
        let theme = cx.theme();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                v_flex()
                    .w(px(520.))
                    .max_w_full()
                    .p(px(18.))
                    .gap_3()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child(div().size(px(8.)).rounded_full().bg(light))
                            .child(header),
                    )
                    .child(body),
            )
    }

    fn connecting_view(&self, target: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
        self.state_card(
            t!("terminal.connectingTitle"),
            theme.primary,
            v_flex()
                .id("terminal-connecting")
                .test_support()
                .gap_3()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(Spinner::new().small())
                        .child(
                            v_flex()
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::BOLD)
                                        .child(t!("terminal.connectTo", target = target.clone())),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(theme.muted_foreground)
                                        .child(t!("terminal.connectingSubtitle")),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(Hsla { a: 0.24, ..black() })
                        .text_size(px(12.))
                        .child(div().text_color(theme.success).child("$"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(format!("opsbatch terminal connect --target {target}")),
                        ),
                ),
            cx,
        )
        .into_any_element()
    }

    fn error_view(
        &self,
        ix: usize,
        key: String,
        remote: bool,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let can_edit_auth = remote && is_ssh_authentication_failure(message.as_deref());
        let shown = message
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| t!("terminal.errorFallback"));
        let retry_key = key.clone();
        let auth_key = key.clone();
        self.state_card(
            if remote {
                t!("terminal.remoteConnectFailed")
            } else {
                t!("terminal.localStartFailed")
            },
            theme.danger,
            v_flex()
                .id(("terminal-error", ix))
                .test_support()
                .gap_2()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .child(t!("terminal.sessionStartFailed")),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child(shown),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .mt_2()
                        .child(
                            Button::new("terminal-retry")
                                .primary()
                                .label(t!("terminal.retry"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let k = retry_key.clone();
                                    this.workspace.update(cx, |w, cx| w.retry(&k, cx));
                                })),
                        )
                        .when(can_edit_auth, |d| {
                            d.child(
                                Button::new("terminal-edit-auth")
                                    .label(t!("terminal.editAuthAndRetry"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        AuthRetryDialog::open(
                                            this.workspace.clone(),
                                            auth_key.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                        })
                        .when(remote, |d| {
                            d.child(
                                Button::new("terminal-open-assets")
                                    .label(t!("terminal.openAssetsSidebar"))
                                    .on_click(cx.listener(|this, _, _, cx| this.show_assets(cx))),
                            )
                        }),
                ),
            cx,
        )
        .into_any_element()
    }

    fn session_area(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let w = self.workspace.read(cx);
        let Some(tab) = w.active_tab() else {
            return div().into_any_element();
        };
        let ix = w.tabs().iter().position(|t| t.key == tab.key).unwrap_or(0);
        let key = tab.key.clone();
        match tab.state {
            ConnState::Connecting => self.connecting_view(
                if tab.host_ip.is_empty() {
                    tab.host_name.clone()
                } else {
                    tab.host_ip.clone()
                },
                cx,
            ),
            ConnState::Error => {
                let (remote, msg) = (tab.kind == TabKind::Remote, tab.error.clone());
                self.error_view(ix, key, remote, msg, cx)
            }
            ConnState::Connected => {
                let Some(main) = tab.pane.clone() else {
                    return div().into_any_element();
                };
                match &tab.split {
                    None => {
                        let main_view = self.pane_view(key.clone(), main, true, cx);
                        div()
                            .relative()
                            .size_full()
                            .child(main_view)
                            .child(self.search_overlay(cx))
                            .into_any_element()
                    }
                    Some(split) => {
                        let (split_state, split_pane, split_err) =
                            (split.state, split.pane.clone(), split.error.clone());
                        let left = self.pane_view(key.clone(), main, false, cx);
                        let close_key = key.clone();
                        let right: AnyElement = match (split_state, split_pane) {
                            (ConnState::Connected, Some(p)) => {
                                self.pane_view(key.clone(), p, false, cx)
                            }
                            (state, _) => {
                                let err = state == ConnState::Error;
                                v_flex()
                                    .id("terminal-split-state")
                                    .test_support()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(div().size(px(8.)).rounded_full().bg(if err {
                                        theme.danger
                                    } else {
                                        theme.primary
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(theme.muted_foreground)
                                            .child(if err {
                                                split_err.filter(|e| !e.is_empty()).unwrap_or_else(
                                                    || t!("terminal.splitConnectFailed"),
                                                )
                                            } else {
                                                t!("terminal.openingSplit")
                                            }),
                                    )
                                    .when(err, |d| {
                                        let k = close_key.clone();
                                        d.child(
                                            Button::new("terminal-split-state-close")
                                                .small()
                                                .label(t!("terminal.closeSplit"))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    let k = k.clone();
                                                    this.workspace
                                                        .update(cx, |w, cx| w.close_split(&k, cx));
                                                })),
                                        )
                                    })
                                    .into_any_element()
                            }
                        };
                        let k = key.clone();
                        h_flex()
                            .id("terminal-split")
                            .test_support()
                            .size_full()
                            .child(div().flex_1().min_w_0().h_full().child(left))
                            .child(div().w(px(1.)).h_full().bg(theme.border))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(right)
                                    .when(split_state == ConnState::Connected, |d| {
                                        d.child(
                                            div().absolute().top(px(4.)).right(px(8.)).child(
                                                Button::new("terminal-split-close")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Icon::new(IconName::X))
                                                    .tooltip(t!("terminal.closeSplit"))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            let k = k.clone();
                                                            this.workspace.update(cx, |w, cx| {
                                                                w.close_split(&k, cx)
                                                            });
                                                        },
                                                    )),
                                            ),
                                        )
                                    }),
                            )
                            .into_any_element()
                    }
                }
            }
        }
    }

    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let (tabs, active): (Vec<_>, Option<String>) = {
            let w = self.workspace.read(cx);
            (
                w.tabs()
                    .iter()
                    .map(|t| {
                        (
                            t.key.clone(),
                            t.host_name.clone(),
                            t.host_ip.clone(),
                            t.state,
                        )
                    })
                    .collect(),
                w.active_key().map(str::to_string),
            )
        };
        let page = cx.entity().downgrade();
        let ws = self.workspace.clone();
        let list_ws = ws.clone();
        let list_page = page.clone();
        // `--terminal-muted` / `--terminal-weak` on non-Windows platforms
        let (muted, weak) = (ui.text_secondary, ui.text_muted);
        let green: Hsla = gpui_kit::rgb(0x52c41a).into();
        let blue: Hsla = gpui_kit::rgb(0x2f80ed).into();
        let red: Hsla = gpui_kit::rgb(0xff4d4f).into();

        let items = tabs
            .into_iter()
            .enumerate()
            .map(|(ix, (key, name, ip, state))| {
                let is_active = active.as_deref() == Some(key.as_str());
                let dot = match state {
                    ConnState::Connected => green,
                    ConnState::Connecting => blue,
                    ConnState::Error => red,
                };
                let status_text = match state {
                    ConnState::Connected => None,
                    ConnState::Connecting => Some((t!("terminal.state.connecting"), blue)),
                    ConnState::Error => Some((t!("terminal.state.error"), red)),
                };
                let select_key = key.clone();
                let close_key = key.clone();
                let menu_key = key.clone();
                let menu_ws = ws.clone();
                h_flex()
                    .id(("terminal-tab", ix))
                    .test_support()
                    .group("terminal-tab")
                    .relative()
                    .h(px(37.))
                    .max_w(px(240.))
                    .px(px(12.))
                    .gap(px(6.))
                    .items_center()
                    .flex_none()
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if is_active {
                        ui.term_accent
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .text_color(if is_active { ui.term_fg } else { muted })
                    .hover(move |d| d.text_color(ui.term_fg).bg(gpui_kit::rgba(0x80caff0a)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let k = select_key.clone();
                        this.workspace.update(cx, |w, cx| w.set_active(&k, cx));
                    }))
                    .child(
                        div()
                            .size(px(6.))
                            .flex_none()
                            .rounded_full()
                            .bg(dot)
                            .shadow(vec![BoxShadow {
                                color: dot.opacity(
                                    if matches!(state, ConnState::Connected | ConnState::Error) {
                                        0.18
                                    } else {
                                        0.2
                                    },
                                ),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(2.),
                                inset: false,
                            }]),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap(px(1.))
                            .items_start()
                            .line_height(relative(1.15))
                            .child(
                                div()
                                    .max_w(px(140.))
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name.clone()),
                            )
                            .when(!ip.is_empty(), |d| {
                                d.child(
                                    div()
                                        .max_w(px(140.))
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(if is_active { muted } else { weak })
                                        .child(ip),
                                )
                            }),
                    )
                    .when_some(status_text, |d, (text, color)| {
                        d.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(Hsla { a: 0.1, ..color })
                                .text_color(color)
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(text),
                        )
                    })
                    .child(
                        div()
                            .id(("terminal-tab-close", ix))
                            .test_support()
                            .size(px(16.))
                            .ml(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .text_color(if is_active { ui.term_fg } else { weak })
                            .when(!is_active, |d| {
                                d.opacity(0.).group_hover("terminal-tab", |s| s.opacity(1.))
                            })
                            .hover(move |d| d.text_color(ui.term_fg).bg(gpui_kit::rgba(0xffffff1f)))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(t!(
                                    "terminal.closeTabLabel",
                                    name = name.clone()
                                ))
                                .build(window, cx)
                            })
                            .child(Icon::new(IconName::X).size_3())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let k = close_key.clone();
                                this.workspace.update(cx, |w, cx| w.close_tab(&k, cx));
                                cx.stop_propagation();
                            })),
                    )
                    .context_menu(move |menu, _, _| {
                        tab_menu(menu, menu_ws.clone(), menu_key.clone())
                    })
                    .into_any_element()
            });

        // `.ui-tabs-list`: 36px strip on the 92% terminal background with a hairline underneath
        h_flex()
            .id("terminal-tab-bar")
            .test_support()
            .h(px(38.))
            .flex_none()
            .px(px(8.))
            .bg(ui.term_tab_bg)
            .border_b_1()
            .border_color(ui.term_border)
            .child(
                h_flex()
                    .id("terminal-tab-list")
                    .test_support()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .children(items)
                    .context_menu(move |menu, _, _| {
                        let page = list_page.clone();
                        let ws = list_ws.clone();
                        menu.item(
                            PopupMenuItem::new(t!("terminal.context.newConnection")).on_click(
                                move |_, _, cx| {
                                    let _ = page.update(cx, |this, cx| this.show_assets(cx));
                                },
                            ),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new(t!("terminal.context.closeAll")).on_click(
                                move |_, _, cx| ws.update(cx, |w, cx| w.close_all_sessions(cx)),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .id("terminal-tab-add")
                    .test_support()
                    .w(px(28.))
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(move |d| d.text_color(ui.term_fg).bg(ui.term_border))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(t!("terminal.connectHost"))
                            .build(window, cx)
                    })
                    .child(Icon::new(IconName::Plus).size_4())
                    .on_click(cx.listener(|this, _, _, cx| this.show_assets(cx))),
            )
    }

    fn bottom_panel(&self, remote: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let theme = cx.theme().clone();
        let effective = if self.bottom_tab.available(remote) {
            self.bottom_tab
        } else {
            BottomTab::Ai
        };
        let all = [
            BottomTab::Sftp,
            BottomTab::Forward,
            BottomTab::Ai,
            BottomTab::Commands,
            BottomTab::Scripts,
            BottomTab::Docker,
        ];
        let muted = ui.text_secondary;
        // `.bottom-panel-shell`: terminal panel colour with a hairline on top; `.bottom-panel-tab-bar`: 28px strip
        v_flex()
            .id("bottom-panel")
            .test_support()
            .size_full()
            .bg(ui.terminal_panel_bg)
            .border_t_1()
            .border_color(ui.term_border)
            // `.bottom-panel-resize-handle`: 8px strip with a 42x2 grip
            .child(
                div()
                    .h(px(8.))
                    .flex_none()
                    .w_full()
                    .relative()
                    .bg(ui.terminal_panel_bg.blend(ui.term_border.opacity(0.16)))
                    .child(
                        div()
                            .absolute()
                            .top(px(3.))
                            .left_1_2()
                            .ml(px(-21.))
                            .w(px(42.))
                            .h(px(2.))
                            .rounded_full()
                            .bg(ui.text_secondary.opacity(0.54)),
                    ),
            )
            .child(
                h_flex()
                    .min_h(px(28.))
                    .flex_none()
                    .items_center()
                    .bg(ui.terminal_panel_bg)
                    .border_b_1()
                    .border_color(ui.term_border)
                    .children(all.into_iter().filter(|t| t.available(remote)).map(|tab| {
                        let active = tab == effective;
                        // `.sftp-tab-btn`: 10px semibold terminal font, 3px 14px padding, 2px underline in the accent
                        div()
                            .id(tab.id())
                            .test_support()
                            .px(px(14.))
                            .py(px(3.))
                            .cursor_pointer()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .border_b_2()
                            .border_color(if active {
                                ui.term_accent
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .text_color(if active { ui.term_accent } else { muted })
                            .hover(move |d| d.text_color(ui.term_accent))
                            .child(tab.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_bottom_tab(tab, window, cx)
                            }))
                    }))
                    .child(div().flex_1())
                    .child(
                        // `.ai-inline-trigger`
                        div()
                            .id("ai-inline-trigger")
                            .test_support()
                            .px(px(10.))
                            .py(px(3.))
                            .mr(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if self.ai_inline.is_some() {
                                gpui_kit::rgb(0xc084fc)
                            } else {
                                gpui_kit::rgb(0x526071)
                            })
                            .hover(|d| {
                                d.text_color(gpui_kit::rgb(0xc084fc))
                                    .bg(gpui_kit::rgba(0xc084fc1a))
                            })
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(t!(
                                    "terminal.quickCommandTitle"
                                ))
                                .build(window, cx)
                            })
                            .child("⌘I")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_inline(window, cx)),
                            ),
                    ),
            )
            .child({
                let content = div().flex_1().min_h_0().relative();
                let inline = match effective {
                    BottomTab::Commands => self.inline.get("commands").cloned(),
                    BottomTab::Scripts => self.inline.get("scripts").cloned(),
                    BottomTab::Docker => self.inline.get("docker").cloned(),
                    _ => None,
                };
                match (effective, self.sftp.clone(), self.forward.clone()) {
                    (BottomTab::Ai, ..) if self.ai_panel.is_some() => {
                        content.child(self.ai_panel.clone().unwrap())
                    }
                    (BottomTab::Sftp, Some(panel), _) => content.child(panel),
                    (BottomTab::Forward, _, Some(panel)) => content.child(panel),
                    _ if inline.is_some() => content.child(inline.unwrap()),
                    _ => content
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} · {}",
                            effective.label(),
                            t!("terminalRuntime.panelPending")
                        )),
                }
            })
    }
}

fn tab_menu(menu: PopupMenu, ws: Entity<TerminalWorkspace>, key: String) -> PopupMenu {
    let item = |label: String, mode: TabCloseMode, ws: Entity<TerminalWorkspace>, key: String| {
        PopupMenuItem::new(label).on_click(move |_, _, cx| {
            let key = key.clone();
            ws.update(cx, |w, cx| w.close_by_mode(&key, mode, cx));
        })
    };
    menu.item(item(
        t!("terminal.context.closeAll"),
        TabCloseMode::All,
        ws.clone(),
        key.clone(),
    ))
    .item(item(
        t!("terminal.context.closeOthers"),
        TabCloseMode::Others,
        ws.clone(),
        key.clone(),
    ))
    .separator()
    .item(item(
        t!("terminal.context.closeLeft"),
        TabCloseMode::Left,
        ws.clone(),
        key.clone(),
    ))
    .item(item(
        t!("terminal.context.closeRight"),
        TabCloseMode::Right,
        ws,
        key,
    ))
}

impl Render for TerminalPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (has_tabs, connected, remote) = {
            let w = self.workspace.read(cx);
            match w.active_tab() {
                Some(t) => (
                    true,
                    t.state == ConnState::Connected && t.pane.is_some(),
                    t.kind == TabKind::Remote,
                ),
                None => (!w.tabs().is_empty(), false, false),
            }
        };

        if !has_tabs {
            return v_flex()
                .id("terminal-page")
                .flex_1()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme.muted_foreground)
                        .child(t!("terminal.noSessions")),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("terminal-reopen-local")
                                .primary()
                                .label(t!("terminal.reopenLocal"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.workspace.update(cx, |w, cx| {
                                        w.open_local(cx);
                                    });
                                })),
                        )
                        .child(
                            Button::new("open-assets")
                                .label(t!("terminal.connectHost"))
                                .icon(Icon::new(IconName::Plus))
                                .on_click(cx.listener(|this, _, _, cx| this.show_assets(cx))),
                        ),
                )
                .into_any_element();
        }

        let session = self.session_area(cx);
        let main_column = if connected {
            v_resizable("terminal-v")
                .with_state(&self.v_state)
                .child(
                    resizable_panel().child(
                        div()
                            .size_full()
                            .min_h_0()
                            .relative()
                            .child(session)
                            .when_some(self.ai_inline.clone(), |d, inline| {
                                d.child(
                                    div().absolute().right(px(10.)).bottom(px(8.)).child(inline),
                                )
                            }),
                    ),
                )
                .child(
                    resizable_panel()
                        .size(px(292.))
                        .size_range(px(36.)..px(2000.))
                        .flex_none()
                        .child(self.bottom_panel(remote, cx)),
                )
                .into_any_element()
        } else {
            div().size_full().child(session).into_any_element()
        };

        let monitor = self.monitor.clone();
        let body = h_resizable("terminal-h")
            .with_state(&self.h_state)
            .child(
                resizable_panel().child(
                    div()
                        .size_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(main_column),
                ),
            )
            .child(
                resizable_panel()
                    .size(px(280.))
                    .size_range(px(8.)..px(280.))
                    .fixed()
                    .flex_none()
                    .visible(monitor.is_some())
                    .child(div().size_full().when_some(monitor, |d, m| d.child(m))),
            );

        let ui = crate::theme::ui(cx);
        // `.terminal-page-shell-active`: translucent terminal background, hairline border, 12px corners
        v_flex()
            .id("terminal-page")
            .test_support()
            .size_full()
            .min_w_0()
            .flex_1()
            .bg(ui.term_shell_bg)
            .border_1()
            .border_color(ui.term_border)
            .rounded(px(12.))
            .overflow_hidden()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let m = &ev.keystroke.modifiers;
                if ev.keystroke.key == "f" && m.secondary() && !m.shift && !m.alt {
                    this.open_search(window, cx);
                    cx.stop_propagation();
                } else if ev.keystroke.key == "i" && m.secondary() && !m.shift && !m.alt {
                    this.open_inline(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(self.tab_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

#[allow(dead_code)]
fn _ids(_: ElementId) {
    let _ = pane_element_id;
}
