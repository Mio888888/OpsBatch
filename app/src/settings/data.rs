//! Data section: database location and backup export (import / clear stay disabled, as in the Tauri build).

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon};
use gpui_kit::*;
use ops_i18n::t;

use crate::widgets::form::field;
use crate::widgets::toast;

pub struct DataSection {
    path: Entity<InputState>,
}

impl DataSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shown = crate::services::ctx(cx)
            .core
            .data_dir()
            .join("opsbatch.db")
            .display()
            .to_string();
        Self {
            path: cx.new(|cx| InputState::new(window, cx).default_value(shown)),
        }
    }

    pub fn default_backup_name() -> String {
        format!("opsbatch-{}.db", chrono::Local::now().format("%Y-%m-%d"))
    }

    /// Copy the database to `destination` and report the result with a toast.
    pub fn export_to(&mut self, destination: String, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |_, cx| {
            let r = ops_core::rt::run(async move {
                ops_core::settings::export_database_backup(&ctx.core, destination).await
            })
            .await
            .and_then(|r| r);
            let _ = cx.update(|window, cx| match r {
                Ok(()) => toast::success(window, cx, t!("settings.backupExported")),
                Err(e) => toast::error(window, cx, t!("common.exportFailed", error = e)),
            });
        })
        .detach();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some(&Self::default_backup_name()));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.export_to(path.display().to_string(), window, cx)
                });
            }
        })
        .detach();
    }
}

impl Render for DataSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("settings-data")
            .test_support()
            .gap_4()
            .max_w(px(520.))
            .child(field(
                t!("settings.dataPath"),
                Input::new(&self.path).disabled(true),
                None,
                cx,
            ))
            .child(field(
                t!("settings.dataBackup"),
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("data-export")
                            .icon(Icon::new(IconName::Database))
                            .label(t!("settings.exportData"))
                            .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                    )
                    .child(
                        Button::new("data-import")
                            .label(t!("settings.importData"))
                            .disabled(true)
                            .tooltip(t!("settings.importDataTitle")),
                    ),
                None,
                cx,
            ))
            .child(div().h(px(1.)).bg(theme.border))
            .child(field(
                t!("settings.clearData"),
                Button::new("data-clear")
                    .label(t!("settings.clearAllData"))
                    .disabled(true)
                    .tooltip(t!("settings.clearAllDataTitle")),
                None,
                cx,
            ))
    }
}
