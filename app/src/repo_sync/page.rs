//! The "Repo sync" mode page. Port of `pages/GitHub/GitHubPage.tsx`.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use crate::widgets::form::page_container;
use ops_i18n::t;
use ops_library::github::{self, PullResult, RepoInfo};

use super::{change_count, result_lines, result_stats};
use crate::library::store::library_store;
use crate::widgets::form::field;
use crate::widgets::toast;

pub struct RepoSyncPage {
    repos: Vec<RepoInfo>,
    pulling: Option<String>,
    result: Option<PullResult>,
    _tasks: Vec<Task<()>>,
}

impl RepoSyncPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self { repos: Vec::new(), pulling: None, result: None, _tasks: Vec::new() };
        this.reload(cx);
        this
    }

    pub fn repos(&self) -> &[RepoInfo] {
        &self.repos
    }
    pub fn result(&self) -> Option<&PullResult> {
        self.result.as_ref()
    }
    pub fn is_pulling(&self) -> bool {
        self.pulling.is_some()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let repos = ops_core::rt::run(async move { github::list_repos(&ctx.db).await }).await.and_then(|r| r).unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.repos = repos;
                cx.notify();
            });
        }));
    }

    /// Run a repo mutation, reload the list afterwards and report failures.
    fn mutate<T: Send + 'static>(&mut self, window: &mut Window, cx: &mut Context<Self>, op: impl FnOnce(std::sync::Arc<ops_ssh::Ctx>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>, ok: Option<String>) {
        let ctx = crate::services::ctx(cx);
        let fut = op(ctx);
        cx.spawn_in(window, async move |this, cx| {
            let r = ops_core::rt::run(fut).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                match r {
                    Ok(_) => {
                        if let Some(m) = ok {
                            toast::success(window, cx, m);
                        }
                        this.reload(cx);
                        // Deleting a repo removes what it imported; keep the libraries in step.
                        library_store(cx).update(cx, |l, cx| l.load_all(cx));
                    }
                    Err(e) => toast::error(window, cx, e),
                }
            });
        })
        .detach();
    }

    pub fn sync(&mut self, repo_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.pulling.is_some() {
            return;
        }
        self.pulling = Some(repo_id.to_string());
        self.result = None;
        cx.notify();
        let (ctx, id) = (crate::services::ctx(cx), repo_id.to_string());
        let language = ops_i18n::current_language().code().to_string();
        cx.spawn_in(window, async move |this, cx| {
            let data_dir = ctx.data_dir().to_path_buf();
            let c = ctx.clone();
            let r = ops_core::rt::run(async move { github::pull_repo(data_dir, &c.db, id, Some(language)).await }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.pulling = None;
                match r {
                    Ok(result) => {
                        toast::success(window, cx, t!("repoSync.syncComplete", count = change_count(&result)));
                        this.result = Some(result);
                        this.reload(cx);
                        library_store(cx).update(cx, |l, cx| l.load_all(cx));
                    }
                    Err(e) => toast::error(window, cx, t!("repoSync.syncFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(window, cx, move |ctx| Box::pin(async move { github::delete_repo(&ctx.db, id).await }), Some(t!("repoSync.deleted")));
    }

    pub fn set_enabled(&mut self, id: String, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(window, cx, move |ctx| Box::pin(async move { github::toggle_repo(&ctx.db, id, enabled).await }), None);
    }

    pub fn set_update_on_startup(&mut self, id: String, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(window, cx, move |ctx| Box::pin(async move { github::set_repo_update_on_startup(&ctx.db, id, on).await }), None);
    }

    pub fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<AddRepoDialog> {
        let page = cx.entity().downgrade();
        let view = cx.new(|cx| AddRepoDialog::new(page, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| d.title(t!("repoSync.addRepoModal")).w(px(480.)).child(shown.clone()));
        view
    }
}

impl Render for RepoSyncPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let weak = cx.entity().downgrade();

        let ui = crate::theme::ui(cx);
        // palette of `.repo-sync-page`: panel / panel-strong / line mixes of the surface with the accent
        let dash = crate::widgets::form::dash(cx);
        let (panel, panel_strong, line) = (dash.panel, dash.strong, dash.line);
        let green: Hsla = gpui_kit::rgb(0x52c41a).into();
        let cards = self.repos.iter().enumerate().map(|(ix, repo)| {
            let is_pulling = self.pulling.as_deref() == Some(repo.id.as_str());
            let (id_sync, id_del, id_en, id_up) = (repo.id.clone(), repo.id.clone(), repo.id.clone(), repo.id.clone());
            let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
            let icon_box = |size: f32, radius: f32, icon: IconName, icon_size: f32| {
                div()
                    .size(px(size))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius))
                    .border_1()
                    .border_color(ui.primary.opacity(0.21).blend(line.opacity(0.0)))
                    .bg(ui.primary.opacity(0.12))
                    .text_color(ui.primary)
                    .child(Icon::new(icon).size(px(icon_size)))
            };
            let meta = |text: String, accent: bool, icon: Option<IconName>| {
                div()
                    .min_w_0()
                    .px(px(9.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(line.opacity(0.68))
                    .bg(panel_strong.opacity(0.7))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .text_color(if accent { ui.primary } else { ui.text_muted })
                    .child(h_flex().gap(px(5.)).items_center().children(icon.map(|i| Icon::new(i).size(px(11.)).flex_none())).child(div().min_w_0().truncate().child(text)))
            };
            // `.repo-sync-card`: 3px status bar on the left, 14px padding, 12px rows
            v_flex()
                .id(("repo-card", ix))
                .test_support()
                .min_w_0()
                .p(px(14.))
                .gap(px(12.))
                .rounded(px(10.))
                .relative()
                .overflow_hidden()
                .border_1()
                .border_color(line)
                // the 3px status bar down the left edge
                .child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(if repo.enabled { green.opacity(0.65) } else { ui.text_muted.opacity(0.46) }))
                .bg(panel)
                .when(!repo.enabled, |d| d.opacity(0.64))
                .child(
                    h_flex()
                        .justify_between()
                        .gap(px(12.))
                        .items_center()
                        .child(
                            h_flex()
                                .gap(px(11.))
                                .items_center()
                                .min_w_0()
                                .child(icon_box(34., 8., IconName::Code, 16.))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap(px(3.))
                                        .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.35)).text_color(ui.text).child(repo.url.clone()))
                                        .child(h_flex().gap(px(4.)).items_center().font_family("Menlo").text_size(px(11.)).text_color(ui.text_muted).child(Icon::new(IconName::Clock).size_3()).child(if repo.branch.is_empty() { "-".to_string() } else { repo.branch.clone() })),
                                ),
                        )
                        .child(
                            // Badge: status dot + text in a bordered 7px chip
                            h_flex()
                                .flex_none()
                                .gap(px(6.))
                                .items_center()
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(7.))
                                .border_1()
                                .border_color(line.opacity(0.86))
                                .bg(panel_strong.opacity(0.78))
                                .text_size(px(12.))
                                .text_color(ui.text_muted)
                                .child(div().size(px(6.)).rounded_full().bg(if repo.enabled { green } else { ui.text_muted }))
                                .child(if repo.enabled { t!("repoSync.enabled") } else { t!("repoSync.paused") }),
                        ),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap(px(8.))
                        .child(meta(repo.last_pulled_at.clone().unwrap_or_else(|| t!("repoSync.neverSynced")), false, Some(IconName::Clock)))
                        .child(meta(if repo.update_on_startup { t!("repoSync.startupUpdateEnabled") } else { t!("repoSync.startupUpdateDisabled") }, true, None)),
                )
                .child(
                    h_flex()
                        .mt(px(2.))
                        .pt(px(11.))
                        .border_t_1()
                        .border_color(line.opacity(0.62))
                        .gap(px(10.))
                        .items_center()
                        .justify_between()
                        .flex_wrap()
                        .child(h_flex().gap(px(6.)).items_center().text_size(px(11.)).text_color(ui.text_muted).child(Switch::new(("repo-enabled", ix)).checked(repo.enabled).on_click(move |v, window, cx| {
                            let (id, v) = (id_en.clone(), *v);
                            let _ = w1.update(cx, |this, cx| this.set_enabled(id, v, window, cx));
                        })).child(if repo.enabled { t!("repoSync.enabledStatus") } else { t!("repoSync.pausedStatus") }))
                        .child(h_flex().gap(px(6.)).items_center().text_size(px(11.)).text_color(ui.text_muted).child(Switch::new(("repo-startup", ix)).checked(repo.update_on_startup).disabled(!repo.enabled).on_click(move |v, window, cx| {
                            let (id, v) = (id_up.clone(), *v);
                            let _ = w2.update(cx, |this, cx| this.set_update_on_startup(id, v, window, cx));
                        })).child(t!("repoSync.updateOnStartup")))
                        .child(
                            h_flex()
                                .gap(px(4.))
                                .child(
                                    Button::new(("repo-sync", ix))
                                        .primary()
                                        .small()
                                        .icon(Icon::new(IconName::RefreshCw))
                                        .label(if is_pulling { t!("repoSync.syncing") } else { t!("repoSync.sync") })
                                        .loading(is_pulling)
                                        .disabled(!repo.enabled || self.pulling.is_some())
                                        .on_click(move |_, window, cx| {
                                            let id = id_sync.clone();
                                            let _ = w3.update(cx, |this, cx| this.sync(&id, window, cx));
                                        }),
                                )
                                .child(Button::new(("repo-delete", ix)).danger().small().icon(Icon::new(IconName::Trash)).label(t!("repoSync.delete")).on_click(move |_, window, cx| {
                                    let id = id_del.clone();
                                    let _ = w4.update(cx, |this, cx| this.delete(id, window, cx));
                                })),
                        ),
                )
        });

        let empty = self.repos.is_empty().then(|| {
            v_flex()
                .id("repo-empty")
                .test_support()
                .items_center()
                .gap_2()
                .p_8()
                .child(Icon::new(IconName::HardDrive).size_8().text_color(theme.muted_foreground))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("repoSync.emptyTitle")))
                .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("repoSync.emptyDesc")))
                .child(Button::new("repo-add-first").primary().icon(Icon::new(IconName::Plus)).label(t!("repoSync.addFirstRepo")).on_click(cx.listener(|this, _, window, cx| drop(this.open_add(window, cx)))))
        });

        let result_panel = self.result.as_ref().map(|r| {
            let (added, updated, deleted, errors, total) = result_stats(r);
            let sections = [(t!("repoSync.added"), added, theme.success), (t!("repoSync.updated"), updated, theme.primary), (t!("repoSync.deletedFiles"), deleted, theme.warning), (t!("repoSync.errors"), errors, theme.danger)];
            v_flex()
                .id("repo-result")
                .test_support()
                .w_full()
                .p(px(15.))
                .gap_3()
                .child(
                    h_flex()
                        .justify_between()
                        .child(v_flex().child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("repoSync.latestResult"))).child(div().font_weight(FontWeight::BOLD).child(t!("repoSync.syncResult"))).child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(if total > 0 { t!("repoSync.totalRecords", count = total) } else { t!("repoSync.noChanges") })))
                        .child(div().px_2().h(px(20.)).rounded_full().bg(Hsla { a: 0.12, ..if errors > 0 { theme.danger } else { theme.success } }).text_color(if errors > 0 { theme.danger } else { theme.success }).text_size(px(11.)).child(if errors > 0 { t!("repoSync.hasErrors") } else { t!("repoSync.syncDone") })),
                )
                .child(h_flex().gap_2().children(sections.into_iter().map(|(label, n, color)| v_flex().flex_1().p_2().rounded(px(6.)).border_1().border_color(theme.border).child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(label)).child(div().text_color(color).font_weight(FontWeight::BOLD).child(n.to_string())))))
                .child(
                    v_flex()
                        .id("repo-result-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .gap_1()
                        .children(result_lines(r).into_iter().enumerate().map(|(i, (sigil, text))| {
                            let color = match sigil {
                                '+' => theme.success,
                                '~' => theme.primary,
                                '-' => theme.warning,
                                _ => theme.danger,
                            };
                            h_flex().id(("repo-result-line", i)).gap_2().text_size(px(11.)).child(div().text_color(color).child(sigil.to_string())).child(div().min_w_0().truncate().font_family("Menlo").child(text))
                        }))
                        .when(total == 0, |d| d.child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("repoSync.noFileChanges")))),
                )
                .child(h_flex().justify_end().child(Button::new("repo-result-close").icon(Icon::new(IconName::X)).label(t!("common.close")).on_click(cx.listener(|this, _, _, cx| {
                    this.result = None;
                    cx.notify();
                }))))
        });

        let hero = h_flex()
            .flex_none()
            .gap(px(18.))
            .px(px(18.))
            .pt(px(18.))
            .pb(px(16.))
            .items_center()
            .justify_between()
            .overflow_hidden()
            .relative()
            .rounded(px(12.))
            .border_1()
            .border_color(line)
            .bg(panel.blend(ui.primary.opacity(0.06)))
            .shadow(crate::widgets::form::dash_shadow(&dash))
            .child(
                h_flex()
                    .gap(px(14.))
                    .items_center()
                    .min_w_0()
                    .child(
                        div()
                            .size(px(44.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(ui.primary.opacity(0.21))
                            .bg(ui.primary.opacity(0.12))
                            .text_color(ui.primary)
                            .child(Icon::new(IconName::Code).size_5()),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_size(px(11.)).font_weight(FontWeight::EXTRA_BOLD).line_height(relative(1.2)).text_color(ui.primary).child(t!("repoSync.sourceManagement").to_uppercase()))
                            .child(div().mt(px(3.)).text_size(px(30.)).font_weight(FontWeight::BOLD).line_height(relative(1.08)).text_color(ui.text).child(t!("repoSync.title")))
                            .child(div().mt(px(8.)).max_w(px(520.)).text_size(px(13.)).line_height(relative(1.58)).text_color(ui.text_muted).child(t!("repoSync.description"))),
                    ),
            )
            .child(Button::new("repo-add").primary().icon(Icon::new(IconName::Plus)).label(t!("repoSync.addRepo")).on_click(cx.listener(|this, _, window, cx| drop(this.open_add(window, cx)))))
            // the 2px accent line along the bottom edge
            .child(
                h_flex()
                    .absolute()
                    .left(px(18.))
                    .right(px(18.))
                    .bottom_0()
                    .h(px(2.))
                    // `linear-gradient(90deg, primary, primary 24%, transparent)` at 66% opacity
                    .child(div().flex_1().h_full().bg(linear_gradient(90., linear_color_stop(ui.primary.opacity(0.66), 0.), linear_color_stop(ui.primary.opacity(0.66 * 0.24), 1.))))
                    .child(div().flex_1().h_full().bg(linear_gradient(90., linear_color_stop(ui.primary.opacity(0.66 * 0.24), 0.), linear_color_stop(ui.primary.opacity(0.0), 1.)))),
            );
        let grid = div()
            .id("repo-grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(1.))
            .pb(px(10.))
            .child(div().grid().grid_cols(2).gap(px(10.)).children(cards).children(empty));
        let page = page_container(cx).bg(crate::widgets::form::dashboard_bg(cx)).id("repo-sync-page").test_support().relative().gap(px(16.)).child(crate::widgets::form::dashboard_backdrop(cx)).child(hero).child(grid);
        // the latest result floats over the bottom-right corner (`.repo-sync-result-panel`)
        page.children(result_panel.map(|p| div().absolute().right(px(22.)).bottom(px(22.)).w(px(360.)).max_h(px(560.)).rounded(px(10.)).border_1().border_color(line).bg(panel).shadow(vec![BoxShadow { color: ui.primary.opacity(0.07), offset: point(px(0.), px(12.)), blur_radius: px(30.), spread_radius: px(0.), inset: false }]).child(p)))
    }
}

pub struct AddRepoDialog {
    page: WeakEntity<RepoSyncPage>,
    pub url: Entity<InputState>,
    pub branch: Entity<InputState>,
    pub token: Entity<InputState>,
    update_on_startup: bool,
    url_missing: bool,
    saving: bool,
}

impl AddRepoDialog {
    fn new(page: WeakEntity<RepoSyncPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            page,
            url: cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/Mio888888/OpsBatch-Library")),
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("main").default_value("main")),
            token: cx.new(|cx| InputState::new(window, cx).placeholder(t!("repoSync.privateRepoHint")).masked(true)),
            update_on_startup: false,
            url_missing: false,
            saving: false,
        }
    }

    pub fn url_missing(&self) -> bool {
        self.url_missing
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let url = self.url.read(cx).value().trim().to_string();
        self.url_missing = url.is_empty();
        if self.url_missing {
            cx.notify();
            return;
        }
        let branch = { let b = self.branch.read(cx).value().trim().to_string(); if b.is_empty() { "main".to_string() } else { b } };
        let token = Some(self.token.read(cx).value().to_string()).filter(|t| !t.is_empty());
        let on = self.update_on_startup;
        let ctx = crate::services::ctx(cx);
        self.saving = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let r = ops_core::rt::run(async move { github::add_repo(&ctx.db, url, branch, token, on).await }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match r {
                    Ok(_) => {
                        toast::success(window, cx, t!("repoSync.repoAdded"));
                        let _ = this.page.update(cx, |p, cx| p.reload(cx));
                        window.close_dialog(cx);
                    }
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AddRepoDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("repo-add-dialog")
            .test_support()
            .gap_3()
            .child(field(t!("repoSync.repoUrl"), Input::new(&self.url), self.url_missing.then(|| SharedString::from(t!("repoSync.repoUrlRequired"))), cx))
            .child(field(t!("repoSync.branch"), Input::new(&self.branch), None, cx))
            .child(field(t!("repoSync.accessToken"), Input::new(&self.token).mask_toggle(), None, cx))
            .child(
                h_flex().gap_2().items_center().child(Switch::new("repo-add-startup").checked(self.update_on_startup).on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.update_on_startup = *v;
                    cx.notify();
                }))).child(v_flex().child(div().text_size(px(13.)).child(t!("repoSync.updateOnStartup"))).child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("repoSync.updateOnStartupExtra")))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("repo-add-cancel").label(t!("common.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("repo-add-ok").primary().label(t!("common.ok")).loading(self.saving).on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))),
            )
    }
}
