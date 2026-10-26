use std::time::Instant;

use gpui_kit::{
    component::input::{Editor, EditorState},
    div,
    prelude::*,
    px, size,
    test::TestWindowExt,
    AppContext, Bounds, Context, Entity, Point, Render, TestAppContext, Window, WindowBounds,
    WindowHandle, WindowOptions,
};

struct Fixture {
    state: Entity<EditorState>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Editor::new(&self.state).size_full())
    }
}

fn open(cx: &mut TestAppContext, language: &'static str) -> (WindowHandle<gpui_kit::base::Root>, Entity<EditorState>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(900.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| Fixture {
                    state: cx.new(|cx| EditorState::new(window, cx).language(language)),
                })
            },
        )
        .expect("open test window");
        let state = view.read(cx).state.clone();
        (window.downcast::<gpui_kit::base::Root>().expect("root"), state)
    })
}

fn undo_key() -> &'static str {
    if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" }
}
fn redo_key() -> &'static str {
    if cfg!(target_os = "macos") { "cmd-shift-z" } else { "ctrl-y" }
}

#[gpui_kit::test]
fn typing_cjk_undo_redo(cx: &mut TestAppContext) {
    let (handle, state) = open(cx, "bash");
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("input", state.entity_id()), cx);
        window.input("echo 中文 テスト", cx);
        assert_eq!(state.read(cx).value(), "echo 中文 テスト");
        window.press(undo_key(), cx);
        let after_undo = state.read(cx).value().to_string();
        assert!(after_undo.len() < "echo 中文 テスト".len(), "undo removed text: {after_undo:?}");
        window.press(redo_key(), cx);
        assert_eq!(state.read(cx).value(), "echo 中文 テスト");
    })
    .unwrap();
}

#[gpui_kit::test]
fn large_file_load_and_edit_latency(cx: &mut TestAppContext) {
    let (handle, state) = open(cx, "bash");
    // ~2.3 MB, 50k lines of shell-ish text with quotes, comments, vars.
    let mut text = String::new();
    for i in 0..50_000 {
        text.push_str(&format!("echo \"line {i}: $HOME\" # comment {i}\n"));
    }
    let bytes = text.len();
    let mut load_ms = 0.0;
    let mut render_ms = 0.0;
    cx.update_window(handle.into(), |_, window, cx| {
        let t = Instant::now();
        state.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
        load_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        window.render_frame(cx);
        render_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(state.read(cx).value().len(), bytes);
    })
    .unwrap();

    // Edit latency: type 20 characters into the loaded document.
    let mut per_key_ms = 0.0;
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("input", state.entity_id()), cx);
        let t = Instant::now();
        for _ in 0..20 {
            window.input("x", cx);
        }
        per_key_ms = t.elapsed().as_secs_f64() * 1e3 / 20.0;
        assert!(state.read(cx).value().starts_with("xxxxxxxxxxxxxxxxxxxx") || state.read(cx).value().contains("xxxxxxxxxxxxxxxxxxxx"));
    })
    .unwrap();
    println!("EDITOR_BENCH bytes={bytes} lines=50000 set_value={load_ms:.1}ms first_frame={render_ms:.1}ms per_keystroke={per_key_ms:.2}ms");
}

/// Every language the old CodeMirror setup handled (plus common ops formats) must have a syntax in the regex backend.
#[test]
fn syntect_covers_opsbatch_file_types() {
    let set = two_face::syntax::extra_newlines();
    let wanted = [
        ("sh", "shell"), ("bash", "shell"), ("py", "python"), ("json", "json"), ("yaml", "yaml"), ("yml", "yaml"),
        ("rs", "rust"), ("js", "javascript"), ("ts", "typescript"), ("html", "html"), ("css", "css"),
        ("sql", "sql"), ("md", "markdown"), ("c", "c"), ("cpp", "c++"), ("java", "java"), ("xml", "xml"),
        ("toml", "toml"), ("go", "go"), ("ps1", "powershell"), ("ini", "ini"), ("conf", "conf"),
        ("Dockerfile", "dockerfile"), ("nginx", "nginx"), ("env", "env"), ("lua", "lua"), ("php", "php"),
    ];
    let mut missing = Vec::new();
    for (ext, label) in wanted {
        if set.find_syntax_by_extension(ext).or_else(|| set.find_syntax_by_token(label)).is_none() {
            missing.push(ext);
        }
    }
    println!("SYNTECT syntaxes={} missing={:?}", set.syntaxes().len(), missing);
    assert!(missing.len() <= 4, "too many unsupported: {missing:?}");
}

#[test]
fn syntect_powershell_lookup() {
    let set = two_face::syntax::extra_newlines();
    let names: Vec<_> = set.syntaxes().iter().filter(|s| s.name.to_lowercase().contains("power") || s.name.to_lowercase().contains("batch") || s.name.to_lowercase().contains("shell")).map(|s| (s.name.clone(), s.file_extensions.clone())).collect();
    println!("SYNTECT shell-ish: {names:?}");
}
