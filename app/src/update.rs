//! In-app update (port of `hooks/useUpdate.ts` + `UpdateModal.tsx`): check, download with progress, verify, hand
//! the installer to the OS, then "restart" (quit) so it can replace this version.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_core::update::{check_app_update_from, download_update, AppUpdateInfo, UpdateSource};
use ops_i18n::t;

use crate::widgets::toast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    Downloading,
    Installing,
    Ready,
    Error,
}

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Available(String),
    UpToDate(String),
    CheckFailed(String),
    Ready,
    InstallFailed(String),
}

type Opener = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

pub struct UpdateStore {
    pub info: Option<AppUpdateInfo>,
    pub check_busy: bool,
    pub phase: Phase,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub installer: Option<PathBuf>,
    source: UpdateSource,
    current_version: String,
    platform: (String, String),
    download_dir: PathBuf,
    opener: Opener,
}

impl EventEmitter<UpdateEvent> for UpdateStore {}

pub struct UpdateHandle(pub Entity<UpdateStore>);
impl Global for UpdateHandle {}

pub fn update_store(cx: &App) -> Entity<UpdateStore> {
    cx.global::<UpdateHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let dir = crate::services::ctx(cx).core.data_dir().join("updates");
    let store = cx.new(|_| UpdateStore {
        info: None,
        check_busy: false,
        phase: Phase::Idle,
        downloaded: 0,
        total: None,
        error: None,
        installer: None,
        source: UpdateSource::from_env(),
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: (std::env::consts::OS.to_string(), std::env::consts::ARCH.to_string()),
        download_dir: dir,
        opener: Arc::new(|p| ops_core::update::open_installer(p)),
    });
    cx.set_global(UpdateHandle(store));
}

/// `1.5 MB`-style size (port of `formatUpdateBytes`).
pub fn format_bytes(value: u64) -> String {
    if value == 0 {
        return "-".into();
    }
    let units = ["B", "KB", "MB", "GB"];
    let (mut size, mut unit) = (value as f64, 0);
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    let precision = if size >= 100.0 || unit == 0 { 0 } else { 1 };
    format!("{size:.precision$} {}", units[unit])
}

impl UpdateStore {
    /// Point the store at another source / version / platform / download dir (tests, mirrors).
    pub fn configure(&mut self, source: UpdateSource, current_version: &str, os: &str, arch: &str, download_dir: PathBuf, opener: Opener) {
        self.source = source;
        self.current_version = current_version.to_string();
        self.platform = (os.to_string(), arch.to_string());
        self.download_dir = download_dir;
        self.opener = opener;
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Downloading | Phase::Installing)
    }

    pub fn has_update(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.has_update)
    }

    pub fn percent(&self) -> u32 {
        self.total.filter(|t| *t > 0).map(|t| ((self.downloaded as f64 / t as f64) * 100.0).round().min(100.0) as u32).unwrap_or(0)
    }

    pub fn check(&mut self, silent: bool, cx: &mut Context<Self>) {
        if self.check_busy {
            return;
        }
        self.check_busy = true;
        cx.notify();
        let (source, current) = (self.source.clone(), self.current_version.clone());
        cx.spawn(async move |this, cx| {
            let r = ops_core::rt::run(async move { check_app_update_from(&source, &current).await }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.check_busy = false;
                match r {
                    Ok(info) => {
                        if !silent {
                            cx.emit(if info.has_update { UpdateEvent::Available(info.latest_version.clone().unwrap_or_default()) } else { UpdateEvent::UpToDate(info.current_version.clone()) });
                        }
                        this.info = Some(info);
                    }
                    Err(e) => {
                        if !silent {
                            cx.emit(UpdateEvent::CheckFailed(e));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn reset_install(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Idle;
        self.downloaded = 0;
        self.total = None;
        self.error = None;
        cx.notify();
    }

    /// Download + verify the installer of the latest release, then hand it to the OS.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(tag) = self.info.as_ref().filter(|i| i.has_update).and_then(|i| i.latest_version.clone()) else {
            return;
        };
        self.phase = Phase::Checking;
        self.downloaded = 0;
        self.total = None;
        self.error = None;
        cx.notify();
        let (source, dir, (os, arch), opener) = (self.source.clone(), self.download_dir.clone(), self.platform.clone(), self.opener.clone());
        let (progress_tx, progress_rx) = flume::unbounded::<(u64, Option<u64>)>();
        let (done_tx, done_rx) = flume::bounded(1);
        ops_core::rt::spawn(async move {
            let r = download_update(&source, &tag, &os, &arch, &dir, move |d, t| {
                let _ = progress_tx.send((d, t));
            })
            .await;
            let _ = done_tx.send_async(r).await;
        });
        cx.spawn(async move |this, cx| {
            while let Ok((downloaded, total)) = progress_rx.recv_async().await {
                if this
                    .update(cx, |this, cx| {
                        this.phase = Phase::Downloading;
                        this.downloaded = downloaded;
                        this.total = total;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            let downloaded = done_rx.recv_async().await.map_err(|_| "更新任务已取消".to_string()).and_then(|r| r);
            let outcome = match downloaded {
                Ok(path) => {
                    let _ = this.update(cx, |this, cx| {
                        this.phase = Phase::Installing;
                        cx.notify();
                    });
                    let p = path.clone();
                    crate::sftp::store::blocking(move || opener(&p)).await.map(|_| path)
                }
                Err(e) => Err(e),
            };
            let _ = this.update(cx, |this, cx| match outcome {
                Ok(path) => {
                    this.downloaded = this.total.unwrap_or(this.downloaded);
                    this.installer = Some(path);
                    this.phase = Phase::Ready;
                    cx.emit(UpdateEvent::Ready);
                    cx.notify();
                }
                Err(e) => {
                    this.phase = Phase::Error;
                    this.error = Some(e.clone());
                    cx.emit(UpdateEvent::InstallFailed(e));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// The "software update" dialog.
pub struct UpdateModal {
    store: Entity<UpdateStore>,
    _subs: Vec<Subscription>,
}

impl UpdateModal {
    /// Open the dialog; the first time it also looks for an update, like the Tauri button.
    pub fn open(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let store = update_store(cx);
        if store.read(cx).info.is_none() {
            store.update(cx, |s, cx| s.check(false, cx));
        }
        let view = cx.new(|cx| Self::new(store, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| d.title(t!("appUpdate.modalTitle")).w(px(560.)).overlay_closable(false).child(shown.clone()));
        view
    }

    pub fn new(store: Entity<UpdateStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe_in(&store, window, |_, _, ev: &UpdateEvent, window, cx| match ev {
            UpdateEvent::Available(v) => toast::success(window, cx, t!("appUpdate.available", version = v)),
            UpdateEvent::UpToDate(v) => toast::success(window, cx, t!("appUpdate.upToDate", version = v)),
            UpdateEvent::CheckFailed(e) => toast::error(window, cx, t!("appUpdate.checkFailed", error = e)),
            UpdateEvent::Ready => toast::success(window, cx, t!("appUpdate.installReady")),
            UpdateEvent::InstallFailed(e) => toast::error(window, cx, t!("appUpdate.installFailed", error = e)),
        });
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self { store, _subs: vec![sub] }
    }
}

impl Render for UpdateModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let s = self.store.read(cx);
        let (info, phase, busy, check_busy) = (s.info.clone(), s.phase, s.busy(), s.check_busy);
        let has_update = s.has_update();
        let percent = s.percent();
        let progress_label = match s.total {
            Some(total) => format!("{} / {}", format_bytes(s.downloaded), format_bytes(total)),
            None => format_bytes(s.downloaded),
        };
        let error = s.error.clone();
        let title = match &info {
            Some(i) if i.has_update => t!("appUpdate.available", version = i.latest_version.clone().unwrap_or_default()),
            Some(i) => t!("appUpdate.upToDate", version = i.current_version.clone()),
            None => t!("appUpdate.checking"),
        };
        let desc = match &info {
            Some(i) if i.has_update => t!("appUpdate.availableDesc"),
            Some(_) => t!("appUpdate.upToDateDesc"),
            None => t!("appUpdate.checkingDesc"),
        };
        let header = match phase {
            Phase::Ready => t!("appUpdate.readyToRestart"),
            Phase::Installing => t!("appUpdate.installing"),
            Phase::Checking => t!("appUpdate.prepareDownload"),
            Phase::Error => t!("appUpdate.downloadError"),
            _ => t!("appUpdate.downloadProgress"),
        };
        let meta = match phase {
            Phase::Ready => t!("appUpdate.readyDescManual"),
            Phase::Error => error.clone().unwrap_or_else(|| t!("appUpdate.downloadError")),
            Phase::Idle => t!("appUpdate.inAppInstallDesc"),
            _ => progress_label.clone(),
        };
        let action_label = match phase {
            Phase::Ready => t!("appUpdate.restartNow"),
            Phase::Checking => t!("appUpdate.prepareDownload"),
            Phase::Installing => t!("appUpdate.installing"),
            Phase::Downloading => t!("appUpdate.downloading"),
            Phase::Error => t!("common.retry"),
            Phase::Idle => t!("appUpdate.downloadAndInstall"),
        };
        let action_disabled = busy || check_busy || !has_update;
        let ui = crate::theme::ui(cx);
        let mix = crate::widgets::form::mix;
        // `.app-update-*` (workbench.css), macOS tokens: caption 10px, body 13px
        let value_box = |label: String, value: String, id: &'static str| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(5.))
                .px(px(12.))
                .py(px(11.))
                .bg(ui.card)
                .border_1()
                .border_color(ui.border)
                .rounded(px(8.))
                .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.1)).text_color(ui.text_muted).child(label))
                .child(div().id(id).test_support().truncate().text_size(px(13.)).font_weight(FontWeight::BOLD).line_height(relative(1.15)).text_color(ui.text).child(value))
        };
        let (status_bg, status_border) = if has_update { (mix(ui.input_bg, theme.primary, 0.10), mix(ui.border, theme.primary, 0.42)) } else { (ui.input_bg, ui.border) };
        v_flex()
            .id("update-modal")
            .test_support()
            .gap(px(14.))
            .pt(px(4.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .p(px(12.))
                    .rounded(px(8.))
                    .bg(status_bg)
                    .border_1()
                    .border_color(status_border)
                    .items_center()
                    .child(
                        div()
                            .size(px(34.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(theme.primary.opacity(0.12))
                            .border_1()
                            .border_color(theme.primary.opacity(0.22))
                            .child(Icon::new(IconName::CircleFadingArrowUp).size_4().text_color(theme.primary)),
                    )
                    .child(
                        v_flex()
                            .child(div().id("update-status").test_support().text_size(px(13.)).font_weight(FontWeight::BOLD).line_height(relative(1.15)).text_color(ui.text).child(title))
                            .child(div().mt(px(3.)).text_size(px(10.)).line_height(relative(1.15)).text_color(ui.text_secondary).child(desc)),
                    ),
            )
            .child(h_flex().gap(px(10.)).child(value_box(t!("appUpdate.currentVersion"), info.as_ref().map(|i| i.current_version.clone()).unwrap_or_else(|| "-".into()), "update-current")).child(value_box(t!("appUpdate.latestVersion"), info.as_ref().and_then(|i| i.latest_version.clone()).unwrap_or_else(|| "-".into()), "update-latest")))
            .when(has_update, |d| {
                let (bg, border) = match phase {
                    Phase::Ready => (mix(ui.card, theme.success, 0.10), mix(ui.border, theme.success, 0.42)),
                    Phase::Error => (mix(ui.card, theme.danger, 0.08), mix(ui.border, theme.danger, 0.38)),
                    _ => (ui.card, ui.border),
                };
                d.child(
                    v_flex()
                        .gap(px(8.))
                        .px(px(12.))
                        .py(px(11.))
                        .rounded(px(8.))
                        .bg(bg)
                        .border_1()
                        .border_color(border)
                        .child(h_flex().justify_between().gap(px(10.)).text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(ui.text_secondary).child(div().id("update-phase").test_support().child(header)).child(div().text_color(ui.text).child(if self.store.read(cx).total.is_some() { format!("{percent}%") } else { progress_label.clone() })))
                        .child(Progress::new("update-progress").value(if phase == Phase::Ready { 100.0 } else { percent as f32 }).loading(matches!(phase, Phase::Checking | Phase::Installing)))
                        .child(div().id("update-meta").test_support().min_h(px(18.)).text_size(px(10.)).text_color(if phase == Phase::Error { theme.danger } else { ui.text_muted }).child(meta)),
                )
            })
            .child(
                v_flex()
                    .overflow_hidden()
                    .bg(ui.card)
                    .border_1()
                    .border_color(ui.border)
                    .rounded(px(8.))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(10.))
                            .border_b_1()
                            .border_color(ui.border)
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text_muted)
                            .child(div().child(info.as_ref().and_then(|i| i.release_title.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| t!("appUpdate.releaseNotes"))))
                            .child(div().text_size(px(9.)).child(info.as_ref().and_then(|i| i.published_at.clone()).unwrap_or_default())),
                    )
                    .child(
                        div()
                            .id("update-notes")
                            .test_support()
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .p(px(12.))
                            .text_size(px(10.))
                            .line_height(relative(1.65))
                            .text_color(ui.text_secondary)
                            .child(info.as_ref().and_then(|i| i.release_notes.clone()).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| t!("appUpdate.noReleaseNotes"))),
                    ),
            )
            .child(
                h_flex()
                    .mt(px(4.))
                    .gap_2()
                    .justify_end()
                    .child(Button::new("update-close").label(t!("common.close")).disabled(busy).on_click(|_, window, cx| window.close_dialog(cx)))
                    .when(phase == Phase::Error, |d| d.child(Button::new("update-reset").label(t!("common.cancel")).on_click(cx.listener(|this, _, _, cx| this.store.update(cx, |s, cx| s.reset_install(cx))))))
                    .child(Button::new("update-check").icon(Icon::new(IconName::RefreshCw)).label(t!("appUpdate.check")).loading(check_busy).disabled(busy || check_busy).on_click(cx.listener(|this, _, _, cx| this.store.update(cx, |s, cx| s.check(false, cx)))))
                    .child(Button::new("update-action").primary().label(action_label).loading(busy).disabled(action_disabled && phase != Phase::Ready).on_click(cx.listener(move |this, _, _, cx| {
                        if phase == Phase::Ready {
                            // "restart": quit so the installer can replace this version
                            cx.dispatch_action(&crate::app::Quit);
                        } else {
                            this.store.update(cx, |s, cx| s.install(cx));
                        }
                    }))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn sizes_use_one_decimal_below_100() {
        assert_eq!(format_bytes(0), "-");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(150 * 1024 * 1024), "150 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
