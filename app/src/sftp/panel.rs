//! Two-pane SFTP browser (local | remote) with bookmarks, preview and transfer queue.
//! Port of `components/SftpPanel.tsx`.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, ResizableState};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Icon, Selectable as _, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_core::pathnames::{basename_from_path, dirname_from_path, join_path};
use ops_i18n::t;
use ops_ssh::sftp::FileEntry;

use super::audio::{format_clock, AudioPlayback};
use super::kinds::{self, PreviewKind};
use super::store::{
    sftp_store, Bookmark, Direction, SessionState, SftpStore, Side, TransferStatus,
};
use crate::widgets::prompt::TextPrompt;
use crate::widgets::toast;

pub const ROW_HEIGHT: f32 = 23.0;

#[derive(Debug, Clone)]
pub enum SftpPanelEvent {
    /// Open a remote file or directory in the editor window.
    OpenEditor {
        host_id: String,
        dir: bool,
        path: String,
    },
}

/// Drag payload: a file row dragged from one pane onto the other.
#[derive(Clone)]
pub struct DraggedFile {
    pub side: Side,
    pub entry: FileEntry,
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
            .child(self.0.clone())
    }
}

pub struct SftpPanel {
    store: Entity<SftpStore>,
    host_id: String,
    local_input: Entity<InputState>,
    remote_input: Entity<InputState>,
    split: Entity<ResizableState>,
    local_scroll: UniformListScrollHandle,
    remote_scroll: UniformListScrollHandle,
    local_focus: FocusHandle,
    remote_focus: FocusHandle,
    show_transfers: bool,
    /// Playback of the previewed audio file: (file path, player or the reason it could not start).
    audio: Option<(String, Result<AudioPlayback, String>)>,
    _audio_tick: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SftpPanelEvent> for SftpPanel {}

impl SftpPanel {
    pub fn new(host_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = sftp_store(cx);
        let local_input = cx.new(|cx| InputState::new(window, cx).placeholder("/"));
        let remote_input = cx.new(|cx| InputState::new(window, cx).placeholder("/"));
        let subs = vec![
            cx.observe_in(&store, window, |this, store, window, cx| {
                this.sync_inputs(&store, window, cx);
                this.sync_audio(cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &local_input,
                window,
                |this, input, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        let p = input.read(cx).value().trim().to_string();
                        if !p.is_empty() {
                            this.store
                                .update(cx, |s, cx| s.navigate(Side::Local, &p, cx));
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &remote_input,
                window,
                |this, input, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        let p = input.read(cx).value().trim().to_string();
                        if !p.is_empty() {
                            this.store
                                .update(cx, |s, cx| s.navigate(Side::Remote, &p, cx));
                        }
                    }
                },
            ),
        ];
        let mut this = Self {
            store: store.clone(),
            host_id: host_id.clone(),
            local_input,
            remote_input,
            split: cx.new(|_| ResizableState::default()),
            local_scroll: UniformListScrollHandle::new(),
            remote_scroll: UniformListScrollHandle::new(),
            local_focus: cx.focus_handle(),
            remote_focus: cx.focus_handle(),
            show_transfers: true,
            audio: None,
            _audio_tick: None,
            _subs: subs,
        };
        this.connect(cx);
        this.sync_inputs(&store, window, cx);
        this
    }

    /// Start (or drop) audio playback to follow the preview overlay.
    fn sync_audio(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .store
            .read(cx)
            .preview
            .as_ref()
            .filter(|p| kinds::preview_kind(&p.entry.name) == PreviewKind::Audio)
            .and_then(|p| p.data.clone().map(|d| (p.entry.path.clone(), d)));
        match wanted {
            Some((path, data)) => {
                if self.audio.as_ref().is_some_and(|(p, _)| *p == path) {
                    return;
                }
                self.audio = Some((path, AudioPlayback::start(data)));
                // a light ticker keeps the clock and progress bar moving while something plays
                self._audio_tick = Some(cx.spawn(async move |this, cx| loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            if this.audio.is_some() {
                                cx.notify()
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }));
            }
            None => {
                self.audio = None;
                self._audio_tick = None;
            }
        }
    }

    /// Test seam: is the audio of the preview playing (None = no audio preview, Err = could not start).
    pub fn audio_state(&self) -> Option<Result<(bool, Duration), String>> {
        self.audio.as_ref().map(|(_, a)| {
            a.as_ref()
                .map(|a| (!a.is_paused() && !a.finished(), a.position()))
                .map_err(Clone::clone)
        })
    }

    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    pub fn store(&self) -> &Entity<SftpStore> {
        &self.store
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let host = self.host_id.clone();
        self.store.update(cx, |s, cx| s.open_session(&host, cx));
    }

    fn sync_inputs(
        &mut self,
        store: &Entity<SftpStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (local, remote) = {
            let s = store.read(cx);
            (s.local_path.clone(), s.remote_path.clone())
        };
        for (input, value) in [
            (self.local_input.clone(), local),
            (self.remote_input.clone(), remote),
        ] {
            if input.read(cx).value().as_ref() != value
                && !input.read(cx).focus_handle(cx).is_focused(window)
            {
                input.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
    }

    // ---- actions ----

    fn choose_local_dir(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(dir) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        let result = this.store.update(cx, |s, cx| s.authorize_local(&dir, cx));
                        if let Err(e) = result {
                            log::warn!("could not authorize {}: {e}", dir.display());
                        }
                    });
                }
            }
        })
        .detach();
    }

    fn open_entry(&mut self, side: Side, entry: FileEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.store
                .update(cx, |s, cx| s.navigate(side, &entry.path, cx));
        } else if side == Side::Remote {
            cx.emit(SftpPanelEvent::OpenEditor {
                host_id: self.host_id.clone(),
                dir: false,
                path: entry.path,
            });
        } else if kinds::is_previewable(&entry) {
            self.store
                .update(cx, |s, cx| s.open_preview(Side::Local, entry, cx));
        }
    }

    fn transfer(&mut self, side: Side, entry: &FileEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            return;
        }
        let path = entry.path.clone();
        self.store.update(cx, |s, cx| {
            if side == Side::Local {
                s.upload(&path, cx)
            } else {
                s.download(&path, cx)
            }
        });
    }

    fn handle_drop(&mut self, target: Side, dragged: &DraggedFile, cx: &mut Context<Self>) {
        if dragged.side == target || dragged.entry.is_dir {
            return;
        }
        self.transfer(dragged.side, &dragged.entry, cx);
    }

    fn upload_external(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for p in paths.paths() {
            if p.is_file() {
                let p = p.to_string_lossy().to_string();
                self.store.update(cx, |s, cx| s.upload(&p, cx));
            }
        }
    }

    fn navigate_keys(&mut self, side: Side, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let (entries, selected) = {
            let s = self.store.read(cx);
            (s.entries(side).to_vec(), s.selected(side).cloned())
        };
        if entries.is_empty() {
            return;
        }
        let idx = selected
            .as_ref()
            .and_then(|sel| entries.iter().position(|e| e.path == sel.path));
        let new_ix = match ev.keystroke.key.as_str() {
            "enter" => {
                if let Some(sel) = selected {
                    self.open_entry(side, sel, cx);
                }
                return;
            }
            "down" => idx.map(|i| (i + 1).min(entries.len() - 1)).unwrap_or(0),
            "up" => idx.map(|i| i.saturating_sub(1)).unwrap_or(0),
            _ => return,
        };
        let path = entries[new_ix].path.clone();
        self.store
            .update(cx, |s, cx| s.select(side, Some(path), cx));
        let handle = if side == Side::Local {
            &self.local_scroll
        } else {
            &self.remote_scroll
        };
        handle.scroll_to_item(new_ix, ScrollStrategy::Nearest);
        cx.stop_propagation();
    }

    // ---- dialogs ----

    pub fn prompt_create(
        &mut self,
        side: Side,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextPrompt> {
        let store = self.store.clone();
        let dir = store.read(cx).path(side).to_string();
        let title = if folder {
            t!("sftp.newFolder")
        } else {
            t!("sftp.newFile")
        };
        let placeholder = if folder {
            t!("sftp.newFolderPrompt")
        } else {
            t!("sftp.newFilePrompt")
        };
        TextPrompt::open(
            title,
            placeholder,
            "",
            Rc::new(move |name, _, cx| {
                let path = join_path(&dir, &name);
                store.update(cx, |s, cx| {
                    if folder {
                        s.mkdir(side, path, cx)
                    } else {
                        s.create_file(side, path, cx)
                    }
                })
            }),
            window,
            cx,
        )
    }

    pub fn prompt_rename(
        &mut self,
        side: Side,
        entry: FileEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextPrompt> {
        let store = self.store.clone();
        let old_name = entry.name.clone();
        TextPrompt::open(
            t!("sftp.rename"),
            t!("sftp.renamePrompt"),
            &old_name,
            Rc::new(move |name, _, cx| {
                let new_path = join_path(&dirname_from_path(&entry.path), &name);
                if name == entry.name {
                    return cx.spawn(async move |_| Ok(()));
                }
                store.update(cx, |s, cx| s.rename(side, entry.path.clone(), new_path, cx))
            }),
            window,
            cx,
        )
    }

    pub fn confirm_delete(
        &mut self,
        side: Side,
        entry: FileEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let store = store.clone();
            let entry = entry.clone();
            alert
                .confirm()
                .title(t!("common.delete"))
                .description(t!("sftp.deleteConfirm", name = entry.name.clone()))
                .show_cancel(true)
                .ok_text(t!("common.delete"))
                .cancel_text(t!("common.cancel"))
                .on_ok(move |_, window, cx| {
                    let task = store.update(cx, |s, cx| {
                        s.remove(side, entry.path.clone(), entry.is_dir, cx)
                    });
                    window
                        .spawn(cx, async move |cx| {
                            if let Err(e) = task.await {
                                let _ = cx.update(|window, cx| {
                                    toast::error(
                                        window,
                                        cx,
                                        t!("common.operationFailed", error = e),
                                    )
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }

    fn extract(&mut self, entry: FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let task = self
            .store
            .update(cx, |s, cx| s.extract_archive(entry.path.clone(), cx));
        window
            .spawn(cx, async move |cx| {
                if let Err(e) = task.await {
                    let _ = cx.update(|window, cx| {
                        toast::error(window, cx, t!("common.operationFailed", error = e))
                    });
                }
            })
            .detach();
    }

    fn entry_menu(
        panel: WeakEntity<Self>,
        side: Side,
        entry: FileEntry,
        menu: PopupMenu,
    ) -> PopupMenu {
        let mut menu = menu;
        let act = |label: String,
                   f: Rc<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>,
                   panel: &WeakEntity<Self>| {
            let panel = panel.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let f = f.clone();
                let _ = panel.update(cx, |this, cx| f(this, window, cx));
            })
        };
        if side == Side::Remote {
            let e = entry.clone();
            menu = menu.item(act(
                if entry.is_dir {
                    t!("sftp.ideOpenDir")
                } else {
                    t!("sftp.ideOpenFile")
                },
                Rc::new(move |this, _, cx| {
                    cx.emit(SftpPanelEvent::OpenEditor {
                        host_id: this.host_id.clone(),
                        dir: e.is_dir,
                        path: e.path.clone(),
                    })
                }),
                &panel,
            ));
        }
        if !entry.is_dir {
            if kinds::is_previewable(&entry) {
                let e = entry.clone();
                menu = menu.item(act(
                    t!("sftp.preview"),
                    Rc::new(move |this, _, cx| {
                        this.store
                            .update(cx, |s, cx| s.open_preview(side, e.clone(), cx))
                    }),
                    &panel,
                ));
            }
            let e = entry.clone();
            menu = menu.item(act(
                if side == Side::Local {
                    t!("sftp.uploadToRemote")
                } else {
                    t!("sftp.downloadToLocal")
                },
                Rc::new(move |this, _, cx| this.transfer(side, &e, cx)),
                &panel,
            ));
            if kinds::is_archive(&entry) && side == Side::Remote {
                let e = entry.clone();
                menu = menu.item(act(
                    t!("sftp.extract"),
                    Rc::new(move |this, window, cx| this.extract(e.clone(), window, cx)),
                    &panel,
                ));
            }
        }
        let e = entry.clone();
        menu = menu.item(act(
            t!("sftp.rename"),
            Rc::new(move |this, window, cx| drop(this.prompt_rename(side, e.clone(), window, cx))),
            &panel,
        ));
        let e = entry.clone();
        menu = menu.item(act(
            t!("common.delete"),
            Rc::new(move |this, window, cx| this.confirm_delete(side, e.clone(), window, cx)),
            &panel,
        ));
        menu = menu.separator();
        if side == Side::Remote {
            menu = menu.item(act(
                t!("sftp.newFile"),
                Rc::new(move |this, window, cx| {
                    drop(this.prompt_create(Side::Remote, false, window, cx))
                }),
                &panel,
            ));
        }
        menu.item(act(
            t!("sftp.newFolder"),
            Rc::new(move |this, window, cx| drop(this.prompt_create(side, true, window, cx))),
            &panel,
        ))
    }

    // ---- rendering ----

    fn file_row(
        &self,
        side: Side,
        ix: usize,
        entry: &FileEntry,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let weak = cx.entity().downgrade();
        let click_entry = entry.clone();
        let drag_entry = entry.clone();
        let menu_entry = entry.clone();
        let id: (&'static str, usize) = (
            if side == Side::Local {
                "sftp-row-local"
            } else {
                "sftp-row-remote"
            },
            ix,
        );
        h_flex()
            .id(id)
            .test_support()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .items_center()
            .text_size(px(12.))
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.list_active))
            .hover(|d| d.bg(theme.list_hover))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let e = click_entry.clone();
                this.store
                    .update(cx, |s, cx| s.select(side, Some(e.path.clone()), cx));
                if ev.click_count() >= 2 {
                    this.open_entry(side, e, cx);
                }
            }))
            .when(!drag_entry.is_dir, |d| {
                let de = drag_entry.clone();
                d.on_drag(
                    DraggedFile {
                        side,
                        entry: de.clone(),
                    },
                    move |_, _, _, cx| cx.new(|_| DragPreview(de.name.clone())),
                )
            })
            .child(div().w(px(18.)).child(kinds::file_icon(entry)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(entry.is_dir, |d| d.text_color(theme.primary))
                    .child(entry.name.clone()),
            )
            .child(
                div()
                    .w(px(60.))
                    .text_right()
                    .text_color(theme.muted_foreground)
                    .child(if entry.is_dir {
                        String::new()
                    } else {
                        kinds::format_size(entry.size)
                    }),
            )
            .child(
                div()
                    .w(px(120.))
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(entry.modified.clone().unwrap_or_default()),
            )
            .context_menu(move |menu, _, _| {
                Self::entry_menu(weak.clone(), side, menu_entry.clone(), menu)
            })
            .into_any_element()
    }

    fn pane(&self, side: Side, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (entries, loading, error, path, selected, ready, session) = {
            let s = self.store.read(cx);
            (
                s.entries(side).to_vec(),
                if side == Side::Local {
                    s.local_loading
                } else {
                    s.remote_loading
                },
                if side == Side::Local {
                    s.local_error.clone()
                } else {
                    s.remote_error.clone()
                },
                s.path(side).to_string(),
                s.selected(side).map(|e| e.path.clone()),
                s.session == SessionState::Ready,
                s.session.clone(),
            )
        };
        let (input, scroll, focus) = if side == Side::Local {
            (&self.local_input, &self.local_scroll, &self.local_focus)
        } else {
            (&self.remote_input, &self.remote_scroll, &self.remote_focus)
        };
        let label = if side == Side::Local {
            t!("sftp.local")
        } else {
            t!("sftp.remote")
        };
        let list_id = if side == Side::Local {
            "sftp-list-local"
        } else {
            "sftp-list-remote"
        };
        let remote_unready = side == Side::Remote && !ready;

        let header = h_flex()
            .gap_1()
            .items_center()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(input).small().disabled(remote_unready)),
            )
            .when(side == Side::Local, |d| {
                d.child(
                    Button::new("sftp-choose-local")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::FolderOpen))
                        .tooltip(t!("sftp.chooseLocalDir"))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_local_dir(cx))),
                )
            })
            .when(remote_unready, |d| {
                d.child(
                    Button::new("sftp-connect-icon")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Cloud))
                        .tooltip(t!("sftp.connectSftp"))
                        .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                )
            })
            .when(!remote_unready, |d| {
                d.child(
                    Button::new(if side == Side::Local {
                        "sftp-up-local"
                    } else {
                        "sftp-up-remote"
                    })
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::ArrowUp))
                    .tooltip(t!("sftp.parentDir"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store.update(cx, |s, cx| s.go_up(side, cx))
                    })),
                )
                .child(
                    Button::new(if side == Side::Local {
                        "sftp-refresh-local"
                    } else {
                        "sftp-refresh-remote"
                    })
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::RefreshCw))
                    .tooltip(t!("common.refresh"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store.update(cx, |s, cx| s.refresh(side, cx))
                    })),
                )
            });

        let muted = theme.muted_foreground;
        let body: AnyElement = if remote_unready {
            let (text, button) = match session {
                SessionState::Failed(e) => (
                    Some(t!("sftp.initFailed", error = e)),
                    t!("sftp.retryConnect"),
                ),
                _ => (None, t!("sftp.connectSftp")),
            };
            let connecting = self.store.read(cx).session == SessionState::Connecting;
            v_flex()
                .id("sftp-remote-unready")
                .test_support()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .when_some(text, |d, text| {
                    d.child(
                        div()
                            .px_3()
                            .text_size(px(12.))
                            .text_color(theme.danger)
                            .child(text),
                    )
                })
                .child(
                    Button::new("sftp-connect")
                        .icon(Icon::new(IconName::Cloud))
                        .label(button)
                        .loading(connecting)
                        .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                )
                .into_any_element()
        } else if loading {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(muted)
                .child(t!("sftp.loadingFiles"))
                .into_any_element()
        } else if let Some(e) = error {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(e)
                .into_any_element()
        } else if entries.is_empty() {
            if side == Side::Local && path.is_empty() {
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Button::new("sftp-choose-local-empty")
                            .icon(Icon::new(IconName::FolderOpen))
                            .label(t!("sftp.chooseLocalDir"))
                            .on_click(cx.listener(|this, _, _, cx| this.choose_local_dir(cx))),
                    )
                    .into_any_element()
            } else {
                div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(t!("sftp.emptyDir"))
                    .into_any_element()
            }
        } else {
            let count = entries.len();
            let entries = Arc::new(entries);
            let sel = selected.clone();
            uniform_list(
                if side == Side::Local {
                    "sftp-uniform-local"
                } else {
                    "sftp-uniform-remote"
                },
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    range
                        .map(|ix| {
                            let e = &entries[ix];
                            let is_sel = sel.as_deref() == Some(e.path.as_str());
                            this.file_row(side, ix, e, is_sel, cx)
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(scroll)
            .size_full()
            .into_any_element()
        };

        v_flex()
            .id(if side == Side::Local {
                "sftp-pane-local"
            } else {
                "sftp-pane-remote"
            })
            .test_support()
            .size_full()
            .min_w_0()
            .child(header)
            .child(
                div()
                    .id(list_id)
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .track_focus(focus)
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                        this.navigate_keys(side, ev, cx)
                    }))
                    .drag_over::<DraggedFile>(move |s, d, _, cx| {
                        if d.side != side {
                            s.bg(cx.theme().drop_target)
                        } else {
                            s
                        }
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedFile, _, cx| {
                        this.handle_drop(side, d, cx)
                    }))
                    .when(side == Side::Remote, |d| {
                        d.drag_over::<ExternalPaths>(|s, _, _, cx| s.bg(cx.theme().drop_target))
                            .on_drop(cx.listener(|this, p: &ExternalPaths, _, cx| {
                                this.upload_external(p, cx)
                            }))
                    })
                    .child(body),
            )
            .into_any_element()
    }

    fn bookmark_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let bookmarks: Vec<Bookmark> = self.store.read(cx).bookmarks.clone();
        let weak = cx.entity().downgrade();
        h_flex()
            .id("sftp-bookmarks")
            .test_support()
            .gap_1()
            .px_2()
            .py_0p5()
            .flex_wrap()
            .border_b_1()
            .border_color(theme.border)
            .children(bookmarks.into_iter().enumerate().map(|(ix, bm)| {
                let go = bm.clone();
                let rm = bm.id.clone();
                let tooltip = format!(
                    "{}: {}",
                    if bm.side == Side::Local {
                        t!("sftp.local")
                    } else {
                        t!("sftp.remote")
                    },
                    bm.path
                );
                h_flex()
                    .id(("sftp-bookmark", ix))
                    .gap_1()
                    .px_1p5()
                    .rounded(px(4.))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .bg(theme.muted)
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let b = go.clone();
                        this.store
                            .update(cx, |s, cx| s.navigate(b.side, &b.path, cx));
                    }))
                    .child(format!(
                        "{} {}",
                        if bm.side == Side::Local { "■" } else { "□" },
                        bm.name
                    ))
                    .child(
                        div()
                            .id(("sftp-bookmark-remove", ix))
                            .text_color(theme.muted_foreground)
                            .child("✗")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = rm.clone();
                                this.store.update(cx, |s, cx| s.remove_bookmark(&id, cx));
                                cx.stop_propagation();
                            })),
                    )
            }))
            .child(
                Button::new("sftp-bookmark-add")
                    .ghost()
                    .xsmall()
                    .label(t!("sftp.addBookmark"))
                    .dropdown_menu(move |menu, _, _| {
                        let (w1, w2) = (weak.clone(), weak.clone());
                        menu.item(PopupMenuItem::new(t!("sftp.localCurrentDir")).on_click(
                            move |_, _, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.store
                                        .update(cx, |s, cx| s.add_bookmark(Side::Local, cx))
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new(t!("sftp.remoteCurrentDir")).on_click(
                                move |_, _, cx| {
                                    let _ = w2.update(cx, |this, cx| {
                                        this.store
                                            .update(cx, |s, cx| s.add_bookmark(Side::Remote, cx))
                                    });
                                },
                            ),
                        )
                    }),
            )
    }

    fn transfer_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        let s = self.store.read(cx);
        if s.transfers.is_empty() {
            return None;
        }
        let active = s.active_transfers();
        let completed = s
            .transfers
            .iter()
            .filter(|t| matches!(t.status, TransferStatus::Done | TransferStatus::Error))
            .count();
        let rows: Vec<_> = s.transfers.iter().rev().take(20).cloned().collect();
        Some(
            v_flex()
                .id("sftp-transfers")
                .test_support()
                .max_h(px(120.))
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(t!(
                            "sftp.transferQueue",
                            active = active,
                            completed = completed
                        )),
                )
                .child(
                    v_flex()
                        .id("sftp-transfers-list")
                        .overflow_y_scroll()
                        .children(rows.into_iter().enumerate().map(|(ix, tr)| {
                            let tail: AnyElement = match tr.status {
                                TransferStatus::Done => div()
                                    .text_color(theme.success)
                                    .child(format!(
                                        "{} · {}",
                                        kinds::format_duration(tr.duration_ms),
                                        kinds::format_speed(tr.speed)
                                    ))
                                    .into_any_element(),
                                TransferStatus::Error => div()
                                    .text_color(theme.danger)
                                    .child(t!("sftp.transferFailed"))
                                    .into_any_element(),
                                _ => div()
                                    .w(px(80.))
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(theme.border)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(tr.progress / 100.0))
                                            .rounded_full()
                                            .bg(theme.primary),
                                    )
                                    .into_any_element(),
                            };
                            h_flex()
                                .id(("sftp-transfer", ix))
                                .gap_2()
                                .px_2()
                                .text_size(px(11.))
                                .items_center()
                                .child(if tr.direction == Direction::Upload {
                                    "↑"
                                } else {
                                    "↓"
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(tr.file_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_color(theme.muted_foreground)
                                        .child(kinds::format_size(tr.file_size)),
                                )
                                .child(tail)
                        })),
                )
                .into_any_element(),
        )
    }

    fn audio_controls(&self, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let body = v_flex()
            .id("sftp-preview-audio")
            .test_support()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4();
        match self.audio.as_ref().map(|(_, a)| a) {
            Some(Ok(audio)) => {
                let total = audio.info().duration;
                let pos = audio.position().min(total.unwrap_or(audio.position()));
                let percent = total
                    .filter(|t| !t.is_zero())
                    .map(|t| (pos.as_secs_f32() / t.as_secs_f32() * 100.0).clamp(0.0, 100.0))
                    .unwrap_or(0.0);
                let playing = !audio.is_paused() && !audio.finished();
                body.child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.to_string()),
                )
                .child(
                    div().w(px(320.)).child(
                        gpui_kit::component::progress::Progress::new("sftp-audio-progress")
                            .value(percent),
                    ),
                )
                .child(
                    div()
                        .id("sftp-audio-clock")
                        .test_support()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} / {}",
                            format_clock(pos),
                            total.map(format_clock).unwrap_or_else(|| "--:--".into())
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("sftp-audio-toggle")
                                .primary()
                                .small()
                                .icon(Icon::new(if playing {
                                    IconName::Pause
                                } else {
                                    IconName::Play
                                }))
                                .label(if playing {
                                    t!("sftp.audioPause")
                                } else {
                                    t!("sftp.audioPlay")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some((_, Ok(a))) = this.audio.as_ref() {
                                        if a.finished() {
                                            let _ = a.seek(Duration::ZERO);
                                        }
                                        if a.is_paused() || a.finished() {
                                            a.play()
                                        } else {
                                            a.pause()
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("sftp-audio-restart")
                                .small()
                                .label(t!("sftp.audioRestart"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some((_, Ok(a))) = this.audio.as_ref() {
                                        let _ = a.seek(Duration::ZERO);
                                        a.play();
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
            }
            Some(Err(e)) => body
                .child(
                    div()
                        .id("sftp-audio-error")
                        .test_support()
                        .text_color(theme.danger)
                        .child(t!("sftp.audioError", error = e.clone())),
                )
                .into_any_element(),
            None => body.into_any_element(),
        }
    }

    fn preview_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        let p = self.store.read(cx).preview.clone()?;
        let kind = kinds::preview_kind(&p.entry.name);
        let body: AnyElement = if p.loading {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(t!("sftp.loadingFiles"))
                .into_any_element()
        } else if let Some(e) = &p.error {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(e.clone())
                .into_any_element()
        } else if p.external {
            v_flex()
                .id("sftp-preview-external")
                .test_support()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(p.entry.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} · {}",
                            kinds::format_size(p.entry.size),
                            t!("sftp.externalHint")
                        )),
                )
                .child(
                    Button::new("sftp-open-external")
                        .primary()
                        .small()
                        .label(t!("sftp.openWithSystem"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, |s, cx| s.open_preview_externally(cx))
                        })),
                )
                .into_any_element()
        } else if let Some(bytes) = &p.data {
            let mono = |text: String| {
                div()
                    .id("sftp-preview-text")
                    .p_3()
                    .size_full()
                    .overflow_y_scroll()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(text)
                    .into_any_element()
            };
            match kind {
                PreviewKind::Image => {
                    let format = match kinds::mime_for_image(&p.entry.name) {
                        "image/jpeg" => Some(ImageFormat::Jpeg),
                        "image/gif" => Some(ImageFormat::Gif),
                        "image/svg+xml" => Some(ImageFormat::Svg),
                        "image/webp" => Some(ImageFormat::Webp),
                        "image/bmp" => Some(ImageFormat::Bmp),
                        "image/png" => Some(ImageFormat::Png),
                        _ => None,
                    };
                    match format {
                        Some(f) => div()
                            .size_full()
                            .p_3()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                img(ImageSource::Image(Arc::new(Image::from_bytes(
                                    f,
                                    bytes.clone(),
                                ))))
                                .max_w_full()
                                .max_h_full(),
                            )
                            .into_any_element(),
                        None => mono(kinds::hex_dump(bytes, 16)),
                    }
                }
                PreviewKind::Audio => self.audio_controls(&p.entry.name, cx),
                PreviewKind::Text => {
                    let text = String::from_utf8_lossy(bytes);
                    let shown: String = if text.chars().count() > 100_000 {
                        format!(
                            "{}{}",
                            text.chars().take(100_000).collect::<String>(),
                            t!("sftp.truncated")
                        )
                    } else {
                        text.into_owned()
                    };
                    mono(shown)
                }
                _ => mono(kinds::hex_dump(bytes, 16)),
            }
        } else {
            div().into_any_element()
        };
        Some(
            v_flex()
                .id("sftp-preview")
                .test_support()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(theme.popover)
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(p.entry.name.clone()),
                        )
                        .child(
                            Button::new("sftp-preview-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |s, cx| s.close_preview(cx))
                                })),
                        ),
                )
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element(),
        )
    }
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ready = self.store.read(cx).session == SessionState::Ready;
        let (total, active) = {
            let s = self.store.read(cx);
            (s.transfers.len(), s.active_transfers())
        };
        let local = self.pane(Side::Local, cx);
        let remote = self.pane(Side::Remote, cx);
        let queue = if self.show_transfers {
            self.transfer_queue(cx)
        } else {
            None
        };
        let preview = self.preview_overlay(cx);
        v_flex()
            .id("sftp-panel")
            .test_support()
            .relative()
            .size_full()
            .min_h_0()
            .when(ready, |d| d.child(self.bookmark_bar(cx)))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("sftp-split")
                        .with_state(&self.split)
                        .child(
                            resizable_panel()
                                .size_range(px(160.)..px(100000.))
                                .child(local),
                        )
                        .child(
                            resizable_panel()
                                .size_range(px(160.)..px(100000.))
                                .child(remote),
                        ),
                ),
            )
            .when_some(queue, |d, q| d.child(q))
            .when(total > 0, |d| {
                d.child(
                    div().absolute().top(px(2.)).right(px(8.)).child(
                        Button::new("sftp-toggle-transfers")
                            .ghost()
                            .xsmall()
                            .label(if active > 0 {
                                active.to_string()
                            } else {
                                "✓".to_string()
                            })
                            .tooltip(if self.show_transfers {
                                t!("sftp.hideTransferQueue")
                            } else {
                                t!("sftp.showTransferQueue")
                            })
                            .selected(self.show_transfers)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_transfers = !this.show_transfers;
                                cx.notify();
                            })),
                    ),
                )
            })
            .when_some(preview, |d, p| d.child(p))
            .bg(theme.background)
    }
}

#[allow(dead_code)]
fn _unused() {
    let _ = basename_from_path;
}
