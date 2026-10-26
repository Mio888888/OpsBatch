//! Application theme: persisted `ThemeSettings` + their projection onto gpui-component's global `Theme`.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Rgba, WindowAppearance};
use ops_core::theme::{parse_css_color, ResolvedTheme, Rgba8, ThemeSettings};

pub struct AppTheme {
    pub settings: ThemeSettings,
    pub resolved: ResolvedTheme,
}

impl Global for AppTheme {}

pub fn hsla_of(c: Rgba8) -> Hsla {
    Rgba { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: c.a }.into()
}

/// CSS colour string from theme data -> Hsla (magenta for unparsable input so mistakes are visible).
pub fn css(s: &str) -> Hsla {
    parse_css_color(s).map(hsla_of).unwrap_or_else(|| hsla_of(Rgba8::rgb(255, 0, 255)))
}

fn os_prefers_dark(cx: &App) -> bool {
    matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

pub fn init(settings: ThemeSettings, cx: &mut App) {
    let resolved = settings.resolve(os_prefers_dark(cx));
    cx.set_global(AppTheme { settings, resolved });
    apply(cx);
}

/// The CSS variables the Tauri build derived from the selected theme (`stores/theme.ts`), as colours.
#[derive(Clone, Copy)]
pub struct UiColors {
    /// `--color-workbench-bg`: window background behind the cards.
    pub page: Hsla,
    pub sidebar: Hsla,
    /// `--color-surface-soft`: the main panel.
    pub main: Hsla,
    /// `--color-surface`: cards, dialogs.
    pub card: Hsla,
    pub header: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub input_bg: Hsla,
    pub input_border: Hsla,
    pub nav_hover: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub primary: Hsla,
    pub primary_strong: Hsla,
    /// `--terminal-bg` and `--terminal-panel-bg` (bottom panel).
    pub terminal_bg: Hsla,
    pub terminal_panel_bg: Hsla,
    /// `--terminal-fg`, `--terminal-accent`, `--terminal-border`, `--terminal-shell-bg` (85%), `--terminal-tab-bg` (92%).
    pub term_fg: Hsla,
    pub term_accent: Hsla,
    pub term_border: Hsla,
    pub term_shell_bg: Hsla,
    pub term_tab_bg: Hsla,
    pub is_dark: bool,
}

/// Text colours: Windows uses the fixed UI palette, other platforms the theme's own line colours.
fn text_colors(r: &ResolvedTheme) -> (Hsla, Hsla, Hsla) {
    if cfg!(target_os = "windows") {
        (css(r.text.primary), css(r.text.secondary), css(r.text.muted))
    } else {
        let p = r.theme.preview;
        (css(p.line_strong), css(p.line_medium), css(p.line_weak))
    }
}

pub fn ui(cx: &App) -> UiColors {
    let r = cx.global::<AppTheme>().resolved;
    let u = r.theme.ui;
    let (text, text_secondary, text_muted) = text_colors(&r);
    let bg = parse_css_color(r.theme.preview.background).unwrap_or(Rgba8::rgb(0, 0, 0));
    let panel = if r.is_dark { ops_core::theme::darken(bg, 8) } else { ops_core::theme::lighten(bg, 4) };
    UiColors {
        page: css(u.page_bg),
        sidebar: css(u.sidebar_bg),
        main: css(u.main_bg),
        card: css(u.card_bg),
        header: css(u.header_bg),
        border: css(u.border),
        border_strong: css(u.border_strong),
        input_bg: css(u.input_bg),
        input_border: css(u.input_border),
        nav_hover: css(u.nav_hover),
        text,
        text_secondary,
        text_muted,
        primary: css(r.accent.base),
        primary_strong: css(r.accent.strong),
        terminal_bg: css(r.theme.preview.background),
        terminal_panel_bg: hsla_of(panel),
        term_fg: css(r.theme.preview.line_strong),
        term_accent: css(r.theme.preview.accent),
        term_border: if r.is_dark { Hsla { h: 0., s: 0., l: 1., a: 0.06 } } else { Hsla { h: 0., s: 0., l: 0., a: 0.08 } },
        term_shell_bg: css(r.theme.preview.background).opacity(0.85),
        term_tab_bg: css(r.theme.preview.background).opacity(0.92),
        is_dark: r.is_dark,
    }
}

pub fn current(cx: &App) -> &AppTheme {
    cx.global::<AppTheme>()
}

/// Change settings, re-resolve, repaint everything and persist.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut ThemeSettings)) {
    let os_dark = os_prefers_dark(cx);
    let settings = {
        let t = cx.global_mut::<AppTheme>();
        edit(&mut t.settings);
        t.resolved = t.settings.resolve(os_dark);
        t.settings.clone()
    };
    apply(cx);
    let ctx = crate::services::ctx(cx);
    ops_core::rt::spawn(async move {
        if let Err(error) = settings.save(&ctx.db).await {
            log::warn!("could not save theme settings: {error}");
        }
    });
}

/// Re-resolve after the OS appearance changed (only matters in "follow system" mode).
pub fn on_system_appearance_changed(cx: &mut App) {
    let os_dark = os_prefers_dark(cx);
    let t = cx.global_mut::<AppTheme>();
    t.resolved = t.settings.resolve(os_dark);
    apply(cx);
}

/// Push the resolved palette into gpui-component's `Theme` (same mapping the CSS variables had in Tauri).
pub fn apply(cx: &mut App) {
    let r = cx.global::<AppTheme>().resolved;
    let scale = cx.global::<AppTheme>().settings.ui_scale;
    crate::scale::set_percent(scale as u32);
    let ui = r.theme.ui;
    let accent = r.accent;
    let mode = if r.is_dark { ThemeMode::Dark } else { ThemeMode::Light };
    // Switching the mode reloads that mode's stock colours, so colours are written in a second pass.
    Theme::change(mode, None, cx);
    Theme::update(cx, |t| {
        // The root view derives the window's rem size from this, so rem-based UI scales with the setting.
        t.font_size = gpui_kit::px(16.0 * scale as f32 / 100.0);
        // --radius-md / --radius-lg of the Tauri build
        t.radius = crate::scale::px(8.0);
        t.radius_lg = crate::scale::px(14.0);
        let c = &mut t.colors;
        let (fg, secondary_fg, muted_fg) = text_colors(&r);
        let border = css(ui.border);
        let border_strong = css(ui.border_strong);
        let card = css(ui.card_bg);
        let main = css(ui.main_bg);
        let sidebar = css(ui.sidebar_bg);
        let header = css(ui.header_bg);
        let hover = css(ui.nav_hover);
        let input_bg = css(ui.input_bg);
        let input_border = css(ui.input_border);
        let primary = css(accent.base);
        let primary_strong = css(accent.strong);
        let white = css("#ffffff");

        c.background = main;
        c.foreground = fg;
        c.border = border;
        c.input = input_border;
        c.muted = card;
        c.muted_foreground = muted_fg;
        c.secondary = card;
        c.secondary_hover = hover;
        c.secondary_active = border_strong;
        c.secondary_foreground = fg;
        c.accent = hover;
        c.accent_foreground = fg;
        c.popover = card;
        c.popover_foreground = fg;
        c.primary = primary;
        c.primary_hover = primary_strong;
        c.primary_active = primary_strong;
        c.primary_foreground = white;
        c.ring = primary;
        c.link = primary;
        c.link_hover = primary_strong;
        c.link_active = primary_strong;
        c.selection = css(r.theme.terminal.selection_background);
        c.caret = fg;

        c.button = card;
        c.button_hover = hover;
        c.button_active = border_strong;
        c.button_foreground = fg;
        c.button_primary = primary;
        c.button_primary_hover = primary_strong;
        c.button_primary_active = primary_strong;
        c.button_primary_foreground = white;
        c.button_secondary = card;
        c.button_secondary_foreground = secondary_fg;

        c.sidebar = sidebar;
        c.sidebar_foreground = fg;
        c.sidebar_border = border;
        c.sidebar_accent = hover;
        c.sidebar_accent_foreground = fg;
        c.sidebar_primary = primary;
        c.sidebar_primary_foreground = white;

        c.list = sidebar;
        c.list_even = sidebar;
        c.list_hover = hover;
        c.list_active = css(&format!("rgba({}, {}, {}, {})", css_rgb(accent.base).0, css_rgb(accent.base).1, css_rgb(accent.base).2, if r.is_dark { "0.1" } else { "0.08" }));
        c.list_active_border = primary;
        c.list_head = header;

        c.title_bar = card;
        c.title_bar_border = border;
        c.status_bar = header;
        c.status_bar_border = border;
        c.tab_bar = header;
        c.tab = header;
        c.tab_active = main;
        c.tab_foreground = muted_fg;
        c.tab_active_foreground = fg;

        c.table = main;
        c.table_head = header;
        c.table_head_foreground = secondary_fg;
        c.table_hover = hover;
        c.table_row_border = border;
        c.table_even = main;
        c.table_active = hover;
        c.table_active_border = primary;

        // Semantic colours are fixed in the Tauri build (antd palette).
        c.danger = css("#ff4d4f");
        c.danger_hover = css("#ff7875");
        c.danger_active = css("#d9363e");
        c.danger_foreground = white;
        c.success = css("#52c41a");
        c.success_hover = css("#73d13d");
        c.success_active = css("#389e0d");
        c.success_foreground = white;
        c.warning = css("#faad14");
        c.warning_hover = css("#ffc53d");
        c.warning_active = css("#d48806");
        c.warning_foreground = css("#202a3a");
        c.info = primary;

        c.scrollbar = Hsla { a: 0.0, ..border };
        c.scrollbar_thumb = Hsla { a: 0.35, ..secondary_fg };
        c.scrollbar_thumb_hover = Hsla { a: 0.55, ..secondary_fg };
        c.skeleton = card;
        c.switch = css(if r.is_dark { "#4b5563" } else { "#c7cdd4" });
        c.switch_thumb = white;
        c.overlay = css("rgba(15, 23, 42, 0.36)");
        c.slider_bar = primary;
        c.slider_thumb = primary;
        c.progress_bar = primary;
        c.drag_border = primary;
        c.drop_target = hover;
        c.window_border = border_strong;
        c.group_box = card;
        c.input = input_border;
        let _ = input_bg;
    });
    Theme::sync_base(cx);
    cx.refresh_windows();
}

fn css_rgb(s: &str) -> (u8, u8, u8) {
    parse_css_color(s).map(|c| (c.r, c.g, c.b)).unwrap_or((0, 0, 0))
}
