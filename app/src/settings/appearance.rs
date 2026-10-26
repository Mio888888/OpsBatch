//! Appearance settings: theme chips, accent, UI scale, motion, terminal font/size/scrollback. Every change is
//! applied and persisted immediately through `theme::update`.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, v_flex, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_core::theme::{
    accent_by_id, normalize_terminal_font_family, ThemeDef, ThemeSettings, ThemeSettingsMode, ThemeTone, ACCENTS, THEMES, UI_SCALE_OPTIONS,
};
use ops_i18n::t;

use super::{group, row};
use crate::theme::{self, css};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

const FONT_SUGGESTIONS: [&str; 9] = ["Cascadia Code", "JetBrains Mono", "Fira Code", "Maple Mono", "Sarasa Mono SC", "SF Mono", "Menlo", "Monaco", "Courier New"];

const STYLE_SUFFIXES: [&str; 45] = [
    "Bold Italic", "BoldItalic", "Bold Oblique", "BoldOblique", "Extra Light Italic", "ExtraLightItalic", "Extra Light", "ExtraLight", "Ultra Light Italic",
    "UltraLightItalic", "Ultra Light", "UltraLight", "Semi Bold Italic", "SemiBoldItalic", "Semi Bold", "SemiBold", "Semibold", "Semi Light Italic",
    "SemiLightItalic", "Semi Light", "SemiLight", "Light Italic", "LightItalic", "Demi Bold", "DemiBold", "Demi Bold Italic", "DemiBoldItalic", "Extra Bold",
    "ExtraBold", "Extra Bold Italic", "ExtraBoldItalic", "Regular", "Roman", "Medium", "Book", "Bold", "Italic", "Oblique", "Light", "Thin", "Black", "Heavy",
    "Condensed", "Expanded", "Narrow",
];

/// `"Name", monospace` (empty for an empty name).
pub fn quote_font_family(name: &str) -> String {
    let escaped = name.trim().replace('\\', "\\\\").replace('"', "\\\"");
    if escaped.is_empty() {
        String::new()
    } else {
        format!("\"{escaped}\", monospace")
    }
}

/// First family of a CSS font stack, unquoted.
pub fn font_family_label(stack: &str) -> String {
    let normalized = normalize_terminal_font_family(Some(stack));
    let first = normalized.split(',').next().unwrap_or("").trim();
    first.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Drop weight/style words (`JetBrains Mono Bold` -> `JetBrains Mono`).
pub fn strip_font_style_suffix(family: &str) -> String {
    let original = family.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut normalized = original.replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let lower = normalized.to_lowercase();
        let hit = STYLE_SUFFIXES.iter().map(|s| format!(" {}", s.to_lowercase())).find(|m| lower.ends_with(m.as_str()) && lower.len() > m.len());
        match hit {
            Some(marker) => normalized = normalized[..normalized.len() - marker.len()].trim().to_string(),
            None => break,
        }
    }
    if normalized.is_empty() {
        original
    } else {
        normalized
    }
}

/// Suggestions + installed families + the current one, deduplicated case-insensitively and sorted.
pub fn merge_font_families(values: impl IntoIterator<Item = String>, current: &str) -> Vec<String> {
    let mut by_key = std::collections::BTreeMap::new();
    for v in values.into_iter().chain((!current.is_empty()).then(|| current.to_string())) {
        let family = strip_font_style_suffix(&v);
        if !family.is_empty() {
            by_key.entry(family.to_lowercase()).or_insert(family);
        }
    }
    let mut out: Vec<String> = by_key.into_values().collect();
    out.sort_by_key(|f| f.to_lowercase());
    out
}

fn ui_scale_label(value: u32) -> String {
    if value == 115 {
        t!("settings.uiScaleRecommended", scale = value)
    } else {
        format!("{value}%")
    }
}

pub struct AppearanceSection {
    scale: Entity<ChoiceSelect>,
    font: Entity<ChoiceSelect>,
    stack: Entity<InputState>,
    size: Entity<InputState>,
    scrollback: Entity<InputState>,
    system_fonts: Vec<String>,
    loading_fonts: bool,
    font_total: usize,
    _subs: Vec<Subscription>,
}

impl AppearanceSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = theme::current(cx).settings.clone();
        let scale = cx.new(|cx| new_select(UI_SCALE_OPTIONS.iter().map(|v| Choice::new(v.to_string(), ui_scale_label(*v))).collect(), &s.ui_scale.to_string(), window, cx));
        let label = font_family_label(&s.terminal_font_family);
        let font = cx.new(|cx| new_select(font_choices(&merge_font_families(FONT_SUGGESTIONS.map(String::from), &label)), &label, window, cx).searchable(true));
        let stack = cx.new(|cx| InputState::new(window, cx).placeholder(t!("settings.terminalFontFamilyPlaceholder")).default_value(s.terminal_font_family.clone()));
        let size = cx.new(|cx| InputState::new(window, cx).default_value(s.terminal_font_size.to_string()));
        let scrollback = cx.new(|cx| InputState::new(window, cx).default_value(s.terminal_scrollback.to_string()));
        let subs = vec![
            cx.subscribe(&scale, |this, _, ev, cx| {
                if let Some(v) = confirmed_value(ev).and_then(|v| v.parse::<u32>().ok()) {
                    this.patch(cx, |s| s.ui_scale = v);
                }
            }),
            cx.subscribe_in(&font, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    let stack = quote_font_family(&v);
                    this.stack.update(cx, |s, cx| s.set_value(stack.clone(), window, cx));
                    this.patch(cx, move |s| s.terminal_font_family = stack);
                }
            }),
            cx.subscribe_in(&stack, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let v = input.read(cx).value().to_string();
                    this.patch(cx, move |s| s.terminal_font_family = v);
                }
                InputEvent::Blur => {
                    let v = normalize_terminal_font_family(Some(input.read(cx).value().as_ref()));
                    input.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
                    this.patch(cx, move |s| s.terminal_font_family = v);
                }
                _ => {}
            }),
            cx.subscribe_in(&size, window, |this, input, ev: &InputEvent, window, cx| this.number_event(input, ev, 10, 24, window, cx, |s| s.terminal_font_size, |s, v| s.terminal_font_size = v)),
            cx.subscribe_in(&scrollback, window, |this, input, ev: &InputEvent, window, cx| this.number_event(input, ev, 500, 50000, window, cx, |s| s.terminal_scrollback, |s, v| s.terminal_scrollback = v)),
        ];
        let mut this = Self { scale, font, stack, size, scrollback, system_fonts: Vec::new(), loading_fonts: true, font_total: 0, _subs: subs };
        this.load_fonts(window, cx);
        this
    }

    /// In-range numbers apply while typing; leaving the field restores a valid value.
    fn number_event(&mut self, input: &Entity<InputState>, ev: &InputEvent, min: u32, max: u32, window: &mut Window, cx: &mut Context<Self>, current: fn(&ThemeSettings) -> u32, apply: fn(&mut ThemeSettings, u32)) {
        match ev {
            InputEvent::Change => {
                if let Ok(v) = input.read(cx).value().trim().parse::<u32>() {
                    if (min..=max).contains(&v) {
                        self.patch(cx, move |s| apply(s, v));
                    }
                }
            }
            InputEvent::Blur => {
                let v = input.read(cx).value().trim().parse::<f64>().ok().map(|v| (v.round().max(0.0) as u64).clamp(min as u64, max as u64) as u32);
                if let Some(v) = v {
                    input.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
                    self.patch(cx, move |s| apply(s, v));
                } else {
                    let restore = current(&theme::current(cx).settings);
                    input.update(cx, |s, cx| s.set_value(restore.to_string(), window, cx));
                }
            }
            _ => {}
        }
    }

    fn load_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let fonts = crate::sftp::store::blocking(|| ops_core::rt::block_on(ops_core::settings::list_system_font_families())).await.unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                this.system_fonts = fonts.into_iter().map(|f| f.family).collect();
                this.loading_fonts = false;
                let current = font_family_label(&theme::current(cx).settings.terminal_font_family);
                let merged = merge_font_families(FONT_SUGGESTIONS.map(String::from).into_iter().chain(this.system_fonts.clone()), &current);
                this.font_total = merged.len();
                this.font.update(cx, |s, cx| {
                    s.set_items(font_choices(&merged), window, cx);
                    s.set_selected_value(&current, window, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn patch(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut ThemeSettings)) {
        theme::update(cx, edit);
        cx.notify();
    }

    pub fn fonts_loaded(&self) -> bool {
        !self.loading_fonts
    }

    pub fn font_count(&self) -> usize {
        self.font_total
    }

    pub fn stack_input(&self) -> &Entity<InputState> {
        &self.stack
    }
    pub fn size_input(&self) -> &Entity<InputState> {
        &self.size
    }
    pub fn scrollback_input(&self) -> &Entity<InputState> {
        &self.scrollback
    }
}

fn font_choices(families: &[String]) -> Vec<Choice> {
    families.iter().map(|f| Choice::new(f.clone(), f.clone())).collect()
}

fn theme_chip(index: usize, def: &'static ThemeDef, selected: bool, cx: &mut Context<AppearanceSection>) -> impl IntoElement {
    let ui = theme::ui(cx);
    let p = def.preview;
    let line = |w: f32, color: &str, top: f32| div().h(px(2.)).w(relative(w)).rounded(px(1.)).bg(css(color)).mt(px(top));
    // `.appearance-theme-chip`: 26x18 thumbnail + name, 1px border, 8px corners
    h_flex()
        .id(("set-theme", index))
        .test_support()
        .gap(px(6.))
        .items_center()
        .pt(px(3.))
        .pb(px(3.))
        .pl(px(3.))
        .pr(px(7.))
        .cursor_pointer()
        .rounded(px(8.))
        .border_1()
        .border_color(if selected { ui.primary } else { ui.border })
        .text_color(if selected { ui.primary } else { ui.text_secondary })
        .hover(move |d| d.border_color(if selected { ui.primary } else { ui.border_strong }).text_color(if selected { ui.primary } else { ui.text }))
        .child(
            v_flex()
                .w(px(26.))
                .h(px(18.))
                .flex_none()
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(gpui_kit::rgba(0x00000026))
                .bg(css(p.background))
                .child(line(0.7, p.line_medium, 3.))
                .child(line(0.5, p.line_weak, 2.))
                .child(line(0.8, p.accent, 2.))
                .child(line(0.4, p.line_strong, 2.)),
        )
        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).line_height(relative(1.3)).whitespace_nowrap().child(def.name))
        .on_click(cx.listener(move |this, _, _, cx| {
            let (id, tone) = (def.id.to_string(), def.tone);
            this.patch(cx, move |s| {
                s.theme_mode = ThemeSettingsMode::Manual;
                s.terminal_theme_id = id;
                s.terminal_theme_tone = tone;
            });
        }))
}

impl Render for AppearanceSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = theme::current(cx).settings.clone();
        let ui = theme::ui(cx);
        let manual = s.theme_mode == ThemeSettingsMode::Manual;
        let chips = |tone: ThemeTone, cx: &mut Context<Self>| {
            h_flex().gap(px(6.)).flex_wrap().justify_end().max_w(px(460.)).children(
                THEMES.iter().enumerate().filter(|(_, d)| d.tone == tone).map(|(i, d)| theme_chip(i, d, manual && s.terminal_theme_id == d.id, cx).into_any_element()).collect::<Vec<_>>(),
            )
        };
        let accent_row = h_flex().gap(px(8.)).flex_wrap().justify_end().max_w(px(460.)).children(ACCENTS.iter().enumerate().map(|(i, a)| {
            let selected = accent_by_id(&s.accent_color_id).map(|c| c.id) == Some(a.id);
            let id = a.id.to_string();
            let base = css(a.base);
            // `.appearance-accent-dot`: 20px, inset white ring; the selected one grows 12% with a 2px gap + 2px outline
            div()
                .id(("set-accent", i))
                .test_support()
                .size(px(if selected { 22.4 } else { 20. }))
                .rounded_full()
                .cursor_pointer()
                .bg(base)
                .border_2()
                .border_color(gpui_kit::transparent_black())
                .shadow(if selected {
                    vec![
                        BoxShadow { color: ui.card, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false },
                        BoxShadow { color: base, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(4.), inset: false },
                    ]
                } else {
                    vec![BoxShadow { color: gpui_kit::rgba(0xffffff2e).into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: true }]
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.patch(cx, move |s| s.accent_color_id = id);
                }))
        }));
        // `.appearance-system-toggle`: 32x28 square with a monitor glyph; highlighted while "follow system" is on
        let weak = cx.entity().downgrade();
        let system_on = !manual;
        let system_toggle = div()
            .id("set-theme-system")
            .test_support()
            .w(px(32.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .border_1()
            .cursor_pointer()
            .border_color(if system_on { ui.primary } else { ui.border })
            .text_color(if system_on { ui.primary } else { ui.text_muted })
            .when(system_on, |d| d.bg(ui.primary.opacity(if ui.is_dark { 0.1 } else { 0.08 })))
            .child(Icon::new(IconName::Monitor).size_3p5())
            .on_click(cx.listener(|this, _, _, cx| this.patch(cx, |s| s.theme_mode = ThemeSettingsMode::System)));
        let ui_c = crate::theme::ui(cx);
        let num = move |input: &Entity<InputState>, suffix: Option<String>, id: &'static str, min: i64, max: i64| h_flex().gap_2().items_center().child(div().w(px(120.)).child(crate::widgets::form::spin(Input::new(input), id, input, min, Some(max), ui_c))).children(suffix.map(|x| div().text_size(px(12.)).child(x)));
        v_flex()
            .id("settings-appearance")
            .test_support()
            .gap_5()
            .child(group(
                t!("settings.appearanceGroupTheme"),
                t!("settings.appearanceGroupThemeDesc"),
                vec![
                    row(t!("settings.themeSystem"), None, system_toggle, cx),
                    row(t!("settings.darkThemes"), None, chips(ThemeTone::Dark, cx), cx),
                    row(t!("settings.lightThemes"), None, chips(ThemeTone::Light, cx), cx),
                    row(t!("settings.accentColor"), Some(t!("settings.accentColorDesc").into()), accent_row, cx),
                ],
                cx,
            ))
            .child(group(
                t!("settings.appearanceGroupInterface"),
                t!("settings.appearanceGroupInterfaceDesc"),
                vec![
                    row(t!("settings.uiScale"), Some(t!("settings.uiScaleDesc").into()), div().w(px(150.)).child(Select::new(&self.scale)), cx),
                    row(
                        t!("settings.reduceMotion"),
                        Some(t!("settings.reduceMotionDesc").into()),
                        Switch::new("set-reduce-motion").checked(s.reduce_motion).on_click(move |on, _, cx| {
                            let on = *on;
                            let _ = weak.update(cx, |this, cx| this.patch(cx, move |s| s.reduce_motion = on));
                        }),
                        cx,
                    ),
                ],
                cx,
            ))
            .child(group(
                t!("settings.appearanceGroupTerminal"),
                t!("settings.appearanceGroupTerminalDesc"),
                vec![
                    row(
                        t!("settings.terminalFontFamily"),
                        None,
                        div().w(px(220.)).child(Select::new(&self.font).search_placeholder(t!("settings.terminalFontSearchPlaceholder")).placeholder(if self.loading_fonts { t!("settings.loadingFonts") } else { t!("settings.selectTerminalFont") })),
                        cx,
                    ),
                    row(t!("settings.terminalFontFamilyStack"), Some(t!("settings.terminalFontFamilyHint").into()), div().w(px(280.)).child(Input::new(&self.stack)), cx),
                    row(t!("settings.terminalFontSize"), None, num(&self.size, None, "set-font-size", 10, 24), cx),
                    row(t!("settings.scrollback"), None, num(&self.scrollback, Some(t!("settings.lines")), "set-scrollback", 500, 50000), cx),
                ],
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{font_family_label, merge_font_families, quote_font_family, strip_font_style_suffix};

    #[test]
    fn font_names_are_quoted_and_labelled() {
        assert_eq!(quote_font_family("Fira Code"), "\"Fira Code\", monospace");
        assert_eq!(quote_font_family("  "), "");
        assert_eq!(quote_font_family("A\"B"), "\"A\\\"B\", monospace");
        assert_eq!(font_family_label("\"Fira Code\", monospace"), "Fira Code");
        assert_eq!(font_family_label("Menlo, monospace"), "Menlo");
    }

    #[test]
    fn style_words_are_stripped_from_installed_families() {
        assert_eq!(strip_font_style_suffix("JetBrains Mono Bold"), "JetBrains Mono");
        assert_eq!(strip_font_style_suffix("Fira-Code-Light Italic"), "Fira Code");
        assert_eq!(strip_font_style_suffix("Menlo"), "Menlo");
        assert_eq!(strip_font_style_suffix("Regular"), "Regular");
    }

    #[test]
    fn families_are_merged_case_insensitively_and_sorted() {
        let merged = merge_font_families(["menlo", "Menlo Bold", "Zed Mono", "Fira Code"].map(String::from), "Courier New");
        assert_eq!(merged, ["Courier New", "Fira Code", "menlo", "Zed Mono"]);
    }
}
