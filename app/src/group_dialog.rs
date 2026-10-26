//! New / rename group dialog (port of the group Modal in MainLayout.tsx).

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;

use crate::stores::assets::AssetsStore;
use crate::widgets::form::field;
use crate::widgets::toast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupDialogMode {
    /// Create under `parent` (None = top level).
    New { parent: Option<String> },
    Rename { id: String, name: String },
    /// The implicit ungrouped group: only its display name can change (local setting).
    RenameDefault { name: String },
}

pub struct GroupDialog {
    store: Entity<AssetsStore>,
    mode: GroupDialogMode,
    pub input: Entity<InputState>,
    required_error: bool,
    error: Option<String>,
    saving: bool,
}

impl GroupDialog {
    pub fn open(store: Entity<AssetsStore>, mode: GroupDialogMode, window: &mut Window, cx: &mut App) {
        let _ = Self::open_with_handle(store, mode, window, cx);
    }

    pub fn open_with_handle(store: Entity<AssetsStore>, mode: GroupDialogMode, window: &mut Window, cx: &mut App) -> Entity<GroupDialog> {
        let title: SharedString = match &mode {
            GroupDialogMode::New { .. } => t!("assets.newGroup"),
            _ => t!("assets.editGroup"),
        }
        .into();
        let view = cx.new(|cx| GroupDialog::new(store, mode, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |dialog, _, _| dialog.title(title.clone()).w(px(520.)).child(shown.clone()));
        let first = view.read(cx).input.clone();
        first.update(cx, |s, cx| s.focus(window, cx));
        view
    }

    fn new(store: Entity<AssetsStore>, mode: GroupDialogMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = match &mode {
            GroupDialogMode::New { .. } => String::new(),
            GroupDialogMode::Rename { name, .. } | GroupDialogMode::RenameDefault { name } => name.clone(),
        };
        let placeholder = t!("assets.groupName");
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(initial));
        Self { store, mode, input, required_error: false, error: None, saving: false }
    }

    pub fn has_required_error(&self) -> bool {
        self.required_error
    }

    pub fn mode(&self) -> &GroupDialogMode {
        &self.mode
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.input.read(cx).value().trim().to_string();
        self.required_error = name.is_empty();
        if self.required_error {
            cx.notify();
            return;
        }
        let mode = self.mode.clone();
        let created = matches!(mode, GroupDialogMode::New { .. });
        let task = self.store.update(cx, |s, cx| match mode {
            GroupDialogMode::New { parent } => {
                let t = s.add_group(name, parent, cx);
                Some(cx.spawn(async move |_, _| t.await.map(|_| ())))
            }
            GroupDialogMode::Rename { id, .. } => s.rename_group(&id, name, cx).map(|t| cx.spawn(async move |_, _| t.await.map(|_| ()))),
            GroupDialogMode::RenameDefault { .. } => {
                s.set_default_group_name(name, cx);
                None
            }
        });
        let Some(task) = task else {
            toast::success(window, cx, t!("assets.groupUpdated"));
            window.close_dialog(cx);
            return;
        };
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        toast::success(window, cx, if created { t!("assets.groupCreated") } else { t!("assets.groupUpdated") });
                        window.close_dialog(cx);
                    }
                    Err(e) => {
                        this.error = Some(t!("common.operationFailed", error = e));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for GroupDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("group-dialog")
            .test_support()
            .gap_3()
            .pt(px(16.))
            .child(field(
                t!("assets.groupName"),
                Input::new(&self.input),
                self.required_error.then(|| t!("common.required").into()),
                cx,
            ))
            .when_some(self.error.clone(), |d, e| d.child(div().text_size(px(12.)).text_color(theme.danger).child(e)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("group-cancel").label(t!("common.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(
                        Button::new("group-save")
                            .primary()
                            .label(t!("common.ok"))
                            .loading(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
