//! The editor window itself: toolbar, optional file tree, code editor, error bar.

use std::collections::HashMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Sizable as _, TitleBar, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_ssh::sftp::TreeNode;

use super::{base_name, decode_text, guess_language, window_label};
use crate::sftp::store::blocking;
use crate::window::{self, WindowKind};

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    File,
    Dir,
}

pub struct EditorWindow {
    host_id: String,
    mode: EditorMode,
    /// The path the window was opened for (file or directory).
    path: String,
    /// File currently shown (file mode: always `path`; dir mode: picked from the tree).
    current_path: Option<String>,
    current_name: Option<String>,
    editor: Option<Entity<EditorState>>,
    original: String,
    loading: bool,
    saving: bool,
    dirty: bool,
    tree: Option<TreeNode>,
    tree_loading: bool,
    /// Explicit expand/collapse choices; unlisted directories follow the default (only the top level is open).
    expanded: HashMap<String, bool>,
    error: Option<String>,
    load_seq: u64,
    _sub: Option<Subscription>,
}

impl EditorWindow {
    /// Open (or focus) the editor window for a remote file or directory.
    pub fn open(host_id: &str, dir: bool, path: &str, cx: &mut App) {
        let label = window_label(host_id, dir, path);
        let title = if dir { base_name(path) } else { base_name(path) };
        let options = window::window_options(WindowKind::Editor, &format!("{title} - {}", t!("editor.remoteEdit")), cx);
        let (host, path) = (host_id.to_string(), path.to_string());
        window::open_or_focus(&label, options, cx, move |window, cx| cx.new(|cx| Self::new(host, dir, path, window, cx)));
    }

    pub fn new(host_id: String, dir: bool, path: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            host_id,
            mode: if dir { EditorMode::Dir } else { EditorMode::File },
            path: path.clone(),
            current_path: None,
            current_name: None,
            editor: None,
            original: String::new(),
            loading: false,
            saving: false,
            dirty: false,
            tree: None,
            tree_loading: false,
            expanded: HashMap::new(),
            error: None,
            load_seq: 0,
            _sub: None,
        };
        match this.mode {
            EditorMode::File => this.load_file(path, window, cx),
            EditorMode::Dir => this.load_tree(cx),
        }
        this.install_close_guard(window, cx);
        this
    }

    // ---- accessors (tests) ----

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn is_loading(&self) -> bool {
        self.loading
    }
    pub fn is_saving(&self) -> bool {
        self.saving
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn current_path(&self) -> Option<&str> {
        self.current_path.as_deref()
    }
    pub fn tree(&self) -> Option<&TreeNode> {
        self.tree.as_ref()
    }
    pub fn text(&self, cx: &App) -> String {
        self.editor.as_ref().map(|e| e.read(cx).value().to_string()).unwrap_or_default()
    }
    pub fn editor_state(&self) -> Option<&Entity<EditorState>> {
        self.editor.as_ref()
    }

    // ---- loading ----

    fn load_tree(&mut self, cx: &mut Context<Self>) {
        self.tree_loading = true;
        self.error = None;
        cx.notify();
        let (ctx, host, path) = (crate::services::ctx(cx), self.host_id.clone(), self.path.clone());
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { ops_ssh::sftp::sftp_read_file_tree(ctx, host, path, Some(3)).await }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.tree_loading = false;
                match result {
                    Ok(tree) => {
                        this.expanded.clear();
                        this.tree = Some(tree);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load `path` into a fresh editor (a new state per file because the grammar is fixed at construction).
    pub fn load_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;
        self.loading = true;
        self.error = None;
        cx.notify();
        let (ctx, host, p) = (crate::services::ctx(cx), self.host_id.clone(), path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move {
                use base64::Engine as _;
                let b64 = ops_ssh::sftp::sftp_read_file(ctx, host, p, Some(MAX_FILE_BYTES)).await?;
                base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                if seq != this.load_seq {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(bytes) => {
                        let name = base_name(&path);
                        let text = decode_text(&bytes);
                        let language = guess_language(Some(&name));
                        let state = cx.new(|cx| EditorState::new(window, cx).language(language));
                        state.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                        this._sub = Some(cx.subscribe(&state, |this, state, ev: &InputEvent, cx| {
                            if matches!(ev, InputEvent::Change) {
                                this.dirty = state.read(cx).value().as_ref() != this.original;
                                cx.notify();
                            }
                        }));
                        this.editor = Some(state);
                        this.original = text;
                        this.dirty = false;
                        this.current_path = Some(path.clone());
                        this.current_name = Some(name);
                        this.update_title(window);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_title(&self, window: &mut Window) {
        let name = self.current_name.clone().unwrap_or_else(|| base_name(&self.path));
        window.set_window_title(&format!("{}{} - {}", if self.dirty { "● " } else { "" }, name, t!("editor.remoteEdit")));
    }

    // ---- saving ----

    /// Write the buffer back over SFTP. Resolves when finished (also used by the close guard).
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        let (Some(path), Some(editor)) = (self.current_path.clone(), self.editor.clone()) else { return cx.spawn(async move |_, _| Ok(())) };
        if self.saving {
            return cx.spawn(async move |_, _| Ok(()));
        }
        let content = editor.read(cx).value().to_string();
        self.saving = true;
        self.error = None;
        cx.notify();
        let (ctx, host) = (crate::services::ctx(cx), self.host_id.clone());
        let saved = content.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = blocking(move || ops_ssh::sftp::sftp_write_file(&ctx.db, &ctx.sftp, &ctx.registry, host, path, content)).await;
            let r = result.clone();
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        this.original = saved;
                        this.dirty = this.editor.as_ref().map(|e| e.read(cx).value().as_ref() != this.original).unwrap_or(false);
                    }
                    Err(e) => this.error = Some(e),
                }
                this.update_title(window);
                cx.notify();
            });
            r
        })
    }

    fn install_close_guard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(this) = weak.upgrade() else { return true };
            if !this.read(cx).dirty {
                return true;
            }
            let (w1, w2) = (weak.clone(), weak.clone());
            window.open_alert_dialog(cx, move |alert, _, _| {
                let (w1, w2) = (w1.clone(), w2.clone());
                alert
                    .confirm()
                    .title(t!("editor.confirmClose"))
                    .description(t!("editor.unsavedPrompt"))
                    .show_cancel(true)
                    .ok_text(t!("editor.save"))
                    .cancel_text(t!("editor.dontSave"))
                    .on_ok(move |_, window, cx| {
                        let Some(this) = w1.upgrade() else { return true };
                        let task = this.update(cx, |this, cx| this.save(window, cx));
                        window
                            .spawn(cx, async move |cx| {
                                let _ = task.await;
                                let _ = cx.update(|window, _| window.remove_window());
                            })
                            .detach();
                        true
                    })
                    .on_cancel(move |_, window, cx| {
                        if let Some(this) = w2.upgrade() {
                            this.update(cx, |this, _| this.dirty = false);
                        }
                        window.remove_window();
                        true
                    })
            });
            false
        });
    }

    // ---- tree ----

    fn is_expanded(&self, depth: usize, path: &str) -> bool {
        self.expanded.get(path).copied().unwrap_or(depth < 1)
    }

    fn visible_rows(&self) -> Vec<(usize, TreeNode)> {
        fn walk(this: &EditorWindow, node: &TreeNode, depth: usize, out: &mut Vec<(usize, TreeNode)>) {
            for child in &node.children {
                out.push((depth, child.clone()));
                if child.is_dir && this.is_expanded(depth, &child.path) {
                    walk(this, child, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(t) = &self.tree {
            walk(self, t, 0, &mut out);
        }
        out
    }

    pub fn toggle_dir(&mut self, depth: usize, path: &str, cx: &mut Context<Self>) {
        let now = self.is_expanded(depth, path);
        self.expanded.insert(path.to_string(), !now);
        cx.notify();
    }

    fn tree_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows = self.visible_rows();
        let title = if self.mode == EditorMode::Dir { base_name(&self.path) } else { t!("editor.remoteDir") };
        v_flex()
            .id("editor-sidebar")
            .test_support()
            .w(px(260.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(div().px_3().py_2().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).border_b_1().border_color(theme.border).child(title))
            .child(if self.tree_loading {
                div().p_3().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("editor.loadingTree")).into_any_element()
            } else if rows.is_empty() {
                div().p_3().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("editor.emptyDir")).into_any_element()
            } else {
                v_flex()
                    .id("editor-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().enumerate().map(|(ix, (depth, node))| {
                        let active = self.current_path.as_deref() == Some(node.path.as_str());
                        let expanded = node.is_dir && self.is_expanded(depth, &node.path);
                        let path = node.path.clone();
                        let is_dir = node.is_dir;
                        h_flex()
                            .id(("editor-tree-row", ix))
                            .test_support()
                            .h(px(24.))
                            .pl(px(8. + depth as f32 * 16.))
                            .pr_2()
                            .gap_1p5()
                            .items_center()
                            .text_size(px(12.))
                            .cursor_pointer()
                            .when(active, |d| d.bg(theme.list_active))
                            .hover(|d| d.bg(theme.list_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if is_dir {
                                    this.toggle_dir(depth, &path, cx);
                                } else {
                                    this.load_file(path.clone(), window, cx);
                                }
                            }))
                            .child(if node.is_dir { if expanded { "▾ 📂" } else { "▸ 📁" } } else { "📄" })
                            .child(div().min_w_0().truncate().child(node.name.clone()))
                    }))
                    .into_any_element()
            })
    }
}

impl Render for EditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let dir = self.mode == EditorMode::Dir;
        let file_label = if dir {
            self.current_name.clone().unwrap_or_else(|| if self.loading { t!("editor.loading") } else { t!("editor.noFile") })
        } else {
            self.current_name.clone().unwrap_or_else(|| if self.path.is_empty() { t!("editor.remoteFile") } else { base_name(&self.path) })
        };
        let path_label = self.current_path.clone().unwrap_or_else(|| self.path.clone());
        self.update_title(window);

        let body: AnyElement = if self.loading {
            let label = if dir { t!("editor.loadingContent") } else { t!("editor.loadingFile") };
            v_flex()
                .id("editor-loading")
                .test_support()
                .size_full()
                .p_4()
                .gap_2()
                .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(label))
                .children([px(320.), px(220.), px(400.), px(160.)].into_iter().map(|w| div().h(px(10.)).w(w).rounded(px(3.)).bg(theme.muted)))
                .into_any_element()
        } else if let Some(editor) = &self.editor {
            div().id("editor-main").test_support().size_full().child(Editor::new(editor).size_full()).into_any_element()
        } else if dir {
            div().id("editor-empty").test_support().size_full().flex().items_center().justify_center().text_size(px(13.)).text_color(theme.muted_foreground).child(t!("editor.selectFile")).into_any_element()
        } else {
            div().into_any_element()
        };

        v_flex()
            .id("editor-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let m = &ev.keystroke.modifiers;
                if ev.keystroke.key == "s" && m.secondary() && !m.shift && !m.alt {
                    this.save(window, cx).detach();
                    cx.stop_propagation();
                }
            }))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .gap_3()
                        .items_center()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .when(self.dirty, |d| d.child(div().id("editor-dirty-dot").test_support().size(px(7.)).rounded_full().bg(theme.warning)))
                                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(file_label)),
                        )
                        .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).text_color(theme.muted_foreground).child(path_label))
                        .when(self.saving, |d| d.child(div().text_size(px(11.)).text_color(theme.muted_foreground).child(t!("editor.saving"))))
                        .child(
                            Button::new("editor-save")
                                .primary()
                                .small()
                                .label(format!("{} ⌘S", t!("editor.save")))
                                .disabled(!self.dirty || self.saving)
                                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx).detach())),
                        ),
                ),
            )
            .child(h_flex().flex_1().min_h_0().when(dir, |d| d.child(self.tree_view(cx))).child(div().flex_1().min_w_0().h_full().child(body)))
            .when_some(self.error.clone().filter(|_| !self.loading), |d, e| {
                d.child(div().id("editor-error").test_support().px_3().py_1().text_size(px(12.)).bg(Hsla { a: 0.15, ..theme.danger }).text_color(theme.danger).child(e))
            })
    }
}
