//! Left sidebar with the host tree (port of the asset panel in MainLayout.tsx, read/select side; editing dialogs,
//! context menus and drag & drop arrive with M3).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use crate::asset_menu::{self, MenuEnv};
use crate::host_dialog::HostDialog;
use crate::asset_tree::{auto_expanded_groups, build_rows, host_matches, CheckState, ExpandState, HostKind, TreeRow, DEFAULT_GROUP_ID};
use crate::stores::assets::{AssetsEvent, AssetsStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetPanelEvent {
    Close,
    /// Open the host in its session window (terminal tab / RDP / VNC — wired in M4/M8).
    OpenHost(String),
    /// Broadcast terminal / batch upload requested for these Linux hosts (wired in M5).
    BroadcastTerminal(Vec<String>),
    BatchUpload(Vec<String>),
}

/// Drag payload: a host row being dragged onto a group row.
#[derive(Clone)]
pub struct DraggedHost {
    pub id: String,
    pub name: String,
}

struct DragPreview(String);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.primary)
            .text_size(px(12.))
            .text_color(theme.popover_foreground)
            .child(self.0.clone())
    }
}

pub struct AssetPanel {
    assets: Entity<AssetsStore>,
    search: Entity<InputState>,
    query: String,
    expand: ExpandState,
    /// "Recent connections" block above the tree (open by default).
    recent_open: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<AssetPanelEvent> for AssetPanel {}

impl AssetPanel {
    pub fn new(assets: Entity<AssetsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("assets.searchPlaceholder")));
        let subs = vec![
            cx.observe(&assets, |_, _, cx| cx.notify()),
            cx.subscribe(&assets, |_, _, _: &AssetsEvent, cx| cx.notify()),
            cx.subscribe_in(&search, window, |this: &mut Self, state, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.query = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];
        Self { assets, search, query: String::new(), expand: ExpandState::default(), recent_open: true, _subs: subs }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn recent_open(&self) -> bool {
        self.recent_open
    }

    /// The "recent connections" block: a collapsible header and the latest hosts (filtered by the search box).
    fn recent_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let hosts: Vec<_> = self.assets.read(cx).recent_hosts().into_iter().filter(|h| host_matches(h, &self.query)).cloned().collect();
        let open = self.recent_open;
        let mut rows = Vec::new();
        for (index, host) in hosts.iter().enumerate() {
            let id = host.id.clone();
            let kind_icon = match HostKind::of(host) {
                HostKind::Linux => IconName::Code,
                HostKind::Windows | HostKind::Vnc => IconName::Server,
            };
            rows.push(
                h_flex()
                    .id(("recent-host", index))
                    .test_support()
                    .min_h(px(30.))
                    .pl(px(16.))
                    .pr(px(8.))
                    .py(px(3.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(ui.primary.opacity(0.1)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(AssetPanelEvent::OpenHost(id.clone()))))
                    .child(Icon::new(kind_icon).size_4().text_color(ui.text_secondary))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .line_height(relative(1.25))
                            .child(div().truncate().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text).child(host.name.clone()))
                            .child(div().truncate().text_size(px(11.)).text_color(ui.text_muted).child(format!("{}@{}:{}", host.username, host.ip, host.port))),
                    )
                    .into_any_element(),
            );
        }
        let count = rows.len();
        v_flex()
            .id("asset-recent")
            .test_support()
            .pb(px(6.))
            .child(
                h_flex()
                    .id("asset-recent-toggle")
                    .test_support()
                    .min_h(px(30.))
                    .pl(px(16.))
                    .pr(px(4.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(ui.primary.opacity(0.1)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.recent_open = !this.recent_open;
                        cx.notify();
                    }))
                    .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size_3p5().text_color(ui.text_muted))
                    .child(Icon::new(IconName::Clock).size_4().text_color(ui.text_secondary))
                    .child(div().flex_1().truncate().text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(ui.text_secondary).child(t!("assets.recent")))
                    .child(div().px(px(6.)).py(px(1.)).rounded_full().bg(ui.main).text_size(px(11.)).line_height(relative(1.4)).text_color(ui.text_muted).child(count.to_string())),
            )
            .when(open && rows.is_empty(), |d| d.child(div().id("asset-recent-empty").test_support().pl(px(40.)).py(px(4.)).text_size(px(11.)).text_color(ui.text_muted).child(t!("assets.noRecent"))))
            .when(open, |d| d.children(rows))
    }

    fn action_button(
        &self,
        id: &'static str,
        icon: IconName,
        label: String,
        primary: bool,
        disabled: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let b = Button::new(id).small().icon(Icon::new(icon)).label(label).disabled(disabled).px(px(8.)).text_size(px(11.)).flex_shrink_0();
        let b = if primary { b.primary() } else { b };
        b.on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}

fn check_box(state: CheckState, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let filled = state != CheckState::None;
    div()
        .size(px(14.))
        .flex_shrink_0()
        .rounded(px(3.))
        .border_1()
        .border_color(if filled { theme.primary } else { theme.input })
        .bg(if filled { theme.primary } else { theme.transparent })
        .flex()
        .items_center()
        .justify_center()
        .when(state == CheckState::All, |d| d.child(Icon::new(IconName::Check).size_3().text_color(theme.primary_foreground)))
        .when(state == CheckState::Some, |d| d.child(Icon::new(IconName::Minus).size_3().text_color(theme.primary_foreground)))
}

impl Render for AssetPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ui = crate::theme::ui(cx);
        let store = self.assets.read(cx);
        let default_name = if store.default_group_name.trim().is_empty() {
            t!("assets.defaultGroup")
        } else {
            store.default_group_name.clone()
        };
        let rows = build_rows(&store.hosts, &store.groups, &self.query, &default_name, &self.expand, &store.selected);
        let total = store.hosts.len();
        let visible = store.hosts.iter().filter(|h| host_matches(h, &self.query)).count();
        let selected_count = store.selected.len();
        let loading = store.loading;
        let has_selection = selected_count > 0;
        let linux_targets = store.selected_linux_host_ids();
        let auto = auto_expanded_groups(&store.hosts, &store.groups);
        let error = store.error.clone();
        let env = MenuEnv { store: self.assets.clone(), panel: cx.entity().downgrade() };

        let mut list = Vec::with_capacity(rows.len());
        for (index, row) in rows.into_iter().enumerate() {
            match row {
                TreeRow::Group { id, name, depth, host_count, expanded, check } => {
                    let toggle_id = id.clone();
                    let check_id = id.clone();
                    let auto = auto.clone();
                    let drop_id = id.clone();
                    let menu_id = id.clone();
                    let menu_env = env.clone();
                    let drop_env = env.clone();
                    list.push(
                        h_flex()
                            .id(("group", index))
                            .test_support()
                            .min_h(px(30.))
                            .my(px(2.))
                            .pl(px(16. + depth as f32 * 24.))
                            .pr(px(4.))
                            .gap(px(6.))
                            .items_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(ui.primary.opacity(0.1)))
                            .drag_over::<DraggedHost>(|s, _, _, cx| s.bg(cx.theme().drop_target).border_1().border_color(cx.theme().primary))
                            .on_drop(cx.listener(move |_, dragged: &DraggedHost, window, cx| {
                                let target = (drop_id != DEFAULT_GROUP_ID).then(|| drop_id.clone());
                                asset_menu::move_host(&drop_env, &dragged.id, target, window, cx);
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.expand.toggle(&toggle_id, &auto);
                                cx.notify();
                            }))
                            .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size_3p5().text_color(theme.muted_foreground))
                            .child(
                                div()
                                    .id(("group-check", index))
                                    .test_support()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        let query = this.query.clone();
                                        this.assets.update(cx, |s, cx| s.toggle_group(&check_id, &query, cx));
                                    }))
                                    .child(check_box(check, cx)),
                            )
                            .child(Icon::new(IconName::FolderOpen).size_4().text_color(ui.text_secondary))
                            .child(div().flex_1().truncate().text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(ui.text_secondary).child(name))
                            // `.asset-tree-count`: pill on the soft surface
                            .child(div().px(px(6.)).py(px(1.)).rounded_full().bg(ui.main).text_size(px(11.)).line_height(relative(1.4)).text_color(ui.text_muted).child(host_count.to_string()))
                            .context_menu(move |menu, window, cx| asset_menu::group_menu(menu_env.clone(), menu_id.clone(), menu, window, cx))
                            .into_any_element(),
                    );
                }
                TreeRow::Host { id, depth, checked } => {
                    let Some(host) = self.assets.read(cx).host(&id).cloned() else { continue };
                    let kind = HostKind::of(&host);
                    let kind_label = match kind {
                        HostKind::Linux => t!("assets.linuxHost"),
                        HostKind::Windows => t!("assets.windowsHost"),
                        HostKind::Vnc => t!("assets.vncHost"),
                    };
                    let toggle_id = id.clone();
                    let _group_name: SharedString = format!("host-row-{index}").into();
                    let menu_env = env.clone();
                    let menu_host = host.clone();
                    let open_id = id.clone();
                    let edit_host = host.clone();
                    let del_host = host.clone();
                    let edit_env = env.clone();
                    let del_env = env.clone();
                    let drag_id = id.clone();
                    let drag_name = host.name.clone();
                    let open_icon = if kind == HostKind::Linux { IconName::Code } else { IconName::Server };
                    let tip_open = match kind {
                        HostKind::Linux => t!("assets.openTerminal"),
                        HostKind::Windows => t!("assets.openRdp"),
                        HostKind::Vnc => t!("assets.openVnc"),
                    };
                    let check_id = id.clone();
                    let icon_btn = |id: (&'static str, usize), icon: IconName, tip: String, danger: bool, on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>| {
                        // `.asset-tree-host-icon-button`: 24px, 6px corners, 72% opacity, fills on hover
                        let hover_bg: Hsla = if danger { gpui_kit::rgb(0xff4d4f).into() } else { ui.primary };
                        div()
                            .id(id)
                            .test_support()
                            .size(px(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .opacity(0.72)
                            .text_color(ui.text_secondary)
                            .hover(move |d| d.opacity(1.).bg(hover_bg).text_color(gpui_kit::white()))
                            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                            .child(Icon::new(icon).size_4())
                            .on_click(move |e, w, cx| {
                                cx.stop_propagation();
                                on_click(e, w, cx)
                            })
                    };
                    let open_btn = {
                        let id = open_id.clone();
                        icon_btn(("host-open", index), open_icon, tip_open, false, Box::new(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(AssetPanelEvent::OpenHost(id.clone())))))
                    };
                    let row_open_id = id.clone();
                    list.push(
                        h_flex()
                            .id(("host", index))
                            .test_support()
                            .min_h(px(30.))
                            .my(px(2.))
                            .pl(px(16. + depth as f32 * 24.))
                            .pr(px(4.))
                            .py(px(4.))
                            .gap(px(6.))
                            .items_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(ui.primary.opacity(0.1)))
                            .when(checked, |d| d.bg(ui.primary.opacity(0.1)))
                            .on_drag(DraggedHost { id: drag_id, name: drag_name }, |d, _, _, cx| cx.new(|_| DragPreview(d.name.clone())))
                            // clicking the row opens the session (ctrl / cmd + click selects instead), like the tree title
                            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                                if ev.modifiers().secondary() {
                                    this.assets.update(cx, |s, cx| s.toggle_host(&toggle_id, cx));
                                } else {
                                    cx.emit(AssetPanelEvent::OpenHost(row_open_id.clone()));
                                }
                            }))
                            .child(
                                div()
                                    .id(("host-check", index))
                                    .test_support()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.assets.update(cx, |s, cx| s.toggle_host(&check_id, cx));
                                    }))
                                    .child(check_box(if checked { CheckState::All } else { CheckState::None }, cx)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .line_height(relative(1.25))
                                    .child(div().truncate().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text).child(host.name.clone()))
                                    .child(div().truncate().text_size(px(11.)).text_color(ui.text_muted).child(format!("{}@{}:{}", host.username, host.ip, host.port)))
                                    .child(
                                        h_flex()
                                            .gap(px(6.))
                                            .text_size(px(11.))
                                            .text_color(ui.text_muted)
                                            .child(kind_label)
                                            .when(!host.remark.is_empty(), |d| d.child(div().truncate().child(host.remark.clone()))),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap(px(2.))
                                    .child(open_btn)
                                    .child(icon_btn(("host-edit", index), IconName::SquarePen, t!("common.edit"), false, Box::new(move |_, window, cx| HostDialog::open(edit_env.store.clone(), Some(edit_host.clone()), None, window, cx))))
                                    .child(icon_btn(("host-delete", index), IconName::Trash, t!("common.delete"), true, Box::new(move |_, window, cx| asset_menu::confirm_delete_host(&del_env, &del_host, window, cx)))),
                            )
                            .context_menu(move |menu, window, cx| asset_menu::host_menu(menu_env.clone(), menu_host.clone(), menu, window, cx))
                            .into_any_element(),
                    );
                }
            }
        }

        let recent = self.recent_block(cx);
        let body: AnyElement = if loading && total == 0 {
            div().p_4().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("assets.loadingHosts")).into_any_element()
        } else if total == 0 {
            div().p_4().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("assets.noHosts")).into_any_element()
        } else if visible == 0 {
            div().p_4().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("assets.noMatchedHosts")).into_any_element()
        } else {
            v_flex().children(list).into_any_element()
        };

        let summary = t!("assets.selectedSummary", selected = selected_count, visible = visible, total = total);
        let linux_for_broadcast = linux_targets.clone();
        let linux_for_upload = linux_targets;

        let ui = crate::theme::ui(cx);
        // `.workbench-asset-panel` (380px, 12px/14px gutter on the window background) holding the white
        // `.asset-sidebar-panel` card: 14px padding, 10px gaps, 14px corners, hairline border and a soft shadow
        div()
            .id("asset-panel")
            .test_support()
            .w(px(380.))
            .min_w(px(320.))
            .h_full()
            .flex_none()
            .pr(px(12.))
            .pb(px(14.))
            .bg(ui.page)
            .child(
        v_flex()
            .size_full()
            .gap(px(10.))
            .p(px(14.))
            .overflow_hidden()
            .bg(ui.card)
            .rounded(px(14.))
            .border_1()
            .border_color(ui.border)
            .shadow(vec![BoxShadow { color: gpui_kit::rgba(0x26304419).into(), offset: point(px(0.), px(16.)), blur_radius: px(36.), spread_radius: px(0.), inset: false }])
            .text_color(ui.text)
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .child(
                        v_flex().child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(t!("assets.title"))),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("assets-refresh")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::RefreshCw))
                                    .loading(loading)
                                    .tooltip(t!("assets.refresh"))
                                    .on_click(cx.listener(|this, _, _, cx| this.assets.update(cx, |s, cx| s.reload(cx)))),
                            )
                            .child(
                                Button::new("assets-hide")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::X))
                                    .tooltip(t!("assets.hide"))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AssetPanelEvent::Close))),
                            ),
                    ),
            )
            .child(div().child(Input::new(&self.search).prefix(Icon::new(IconName::Search).size_3p5()).cleanable(true)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.action_button("assets-add", IconName::Plus, t!("common.add"), true, false, |this, window, cx| HostDialog::open(this.assets.clone(), None, None, window, cx), cx))
                    .child(self.action_button("assets-cloud", IconName::Server, t!("assets.cloudImport"), false, false, |this, window, cx| crate::cloud_dialog::CloudImportDialog::open(this.assets.clone(), window, cx), cx))
                    .child(self.action_button(
                        "assets-broadcast",
                        IconName::Code,
                        t!("assets.broadcastTerminal"),
                        true,
                        !has_selection,
                        move |_, _, cx| cx.emit(AssetPanelEvent::BroadcastTerminal(linux_for_broadcast.clone())),
                        cx,
                    ))
                    .child(self.action_button(
                        "assets-upload",
                        IconName::Upload,
                        t!("assets.batchUpload"),
                        false,
                        !has_selection,
                        move |_, _, cx| cx.emit(AssetPanelEvent::BatchUpload(linux_for_upload.clone())),
                        cx,
                    )),
            )
            .child(div().id("asset-summary").test_support().text_size(px(12.)).line_height(relative(1.35)).text_color(ui.text_muted).child(summary))
            .when_some(error, |d, e| d.child(div().text_size(px(11.)).text_color(theme.danger).child(e)))
            .child(
                div()
                    .id("asset-tree")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(recent)
                    .child(body)
                    .context_menu(move |menu, _, _| asset_menu::empty_menu(env.clone(), menu)),
            ),
            )
    }
}
