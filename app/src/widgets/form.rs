use crate::scale::px;
use gpui_kit::component::{v_flex, ActiveTheme as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Label above a control, with an optional error line (red) under it.
pub fn field(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    error: Option<SharedString>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    v_flex()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(crate::theme::ui(cx).text_secondary)
                .child(label.into()),
        )
        .child(control)
        .when_some(error, |d, e| {
            d.child(div().text_size(px(11.)).text_color(theme.danger).child(e))
        })
}

/// Section heading with a dim explanatory line.
pub fn section(
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme.muted_foreground)
                .child(hint.into()),
        )
}

/// Switch with its title + dim description to the right, as in the Tauri host form.
pub fn switch_row(
    id: &'static str,
    checked: bool,
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    use gpui_kit::component::switch::Switch;
    let theme = cx.theme();
    gpui_kit::component::h_flex()
        .gap_2()
        .items_center()
        .child(
            Switch::new(id)
                .checked(checked)
                .on_click(move |c, w, cx| on_change(*c, w, cx)),
        )
        .child(
            v_flex()
                .child(div().text_size(px(13.)).child(title.into()))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(hint.into()),
                ),
        )
}

/// `.page-container`: the rounded white card most pages of the Tauri build live in (18px corners, 22px padding).
pub fn page_container(cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    v_flex()
        .size_full()
        .p(px(22.))
        .overflow_hidden()
        .bg(ui.card)
        .border_1()
        .border_color(ui.border)
        .rounded(px(18.))
        .shadow(vec![BoxShadow {
            color: gpui_kit::rgba(0x26304414).into(),
            offset: point(px(0.), px(18.)),
            blur_radius: px(48.),
            spread_radius: px(0.),
            inset: false,
        }])
}

/// `color-mix(in srgb, a (1-t), b t)`: premultiplied interpolation, so translucent colours mix like in CSS.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let alpha = a.a + (b.a - a.a) * t;
    if alpha <= 0.0 {
        return Hsla {
            h: 0.,
            s: 0.,
            l: 0.,
            a: 0.,
        };
    }
    let ch = |x: f32, y: f32| ((x * a.a) * (1.0 - t) + (y * b.a) * t) / alpha;
    Rgba {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: alpha,
    }
    .into()
}

/// Colours of `.workflow-page` / `.repo-sync-page` (light CSS and its `[data-theme-tone="dark"]` override).
#[derive(Clone, Copy)]
pub struct Dash {
    pub bg: Hsla,
    pub panel: Hsla,
    pub strong: Hsla,
    pub line: Hsla,
    pub shadow: Hsla,
    pub shadow_blur: f32,
    pub grid_v: Hsla,
    pub grid_h: Hsla,
    pub glow_primary: (Hsla, f32),
    pub glow_warning: (Hsla, f32),
}

pub fn dash(cx: &App) -> Dash {
    let ui = crate::theme::ui(cx);
    let warning = cx.theme().warning;
    if ui.is_dark {
        Dash {
            bg: rgb(0x10141c).into(),
            panel: ui.card.opacity(0.76),
            strong: mix(ui.card, ui.main, 0.38),
            line: mix(ui.border, ui.primary, 0.082).opacity(0.85),
            shadow: gpui_kit::black().opacity(0.22),
            shadow_blur: 46.0,
            grid_v: ui.border.opacity(0.48),
            grid_h: ui.border.opacity(0.36),
            glow_primary: (ui.primary.opacity(0.18), 0.38),
            glow_warning: (warning.opacity(0.07), 0.34),
        }
    } else {
        Dash {
            bg: mix(ui.card, ui.main, 0.72),
            panel: mix(ui.card, ui.primary, 0.033).opacity(0.91),
            strong: mix(ui.card, ui.primary, 0.084).opacity(0.83),
            line: mix(ui.border_strong, ui.primary, 0.111).opacity(0.81),
            shadow: ui.primary.opacity(0.09),
            shadow_blur: 48.0,
            grid_v: ui.border.opacity(0.42),
            grid_h: ui.border.opacity(0.32),
            glow_primary: (ui.primary.opacity(0.16), 0.38),
            glow_warning: (warning.opacity(0.09), 0.32),
        }
    }
}

pub fn dashboard_bg(cx: &App) -> Hsla {
    dash(cx).bg
}

/// `box-shadow: 0 18px <blur> <colour>` of the dashboard cards.
pub fn dash_shadow(d: &Dash) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: d.shadow,
        offset: point(px(0.), px(18.)),
        blur_radius: px(d.shadow_blur),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The grid (48px lines) and the two soft glows behind `.workflow-page` / `.repo-sync-page`; place it as the
/// first child of a `relative` container.
pub fn dashboard_backdrop(cx: &App) -> impl IntoElement {
    let d = dash(cx);
    let (line_v, line_h) = (d.grid_v, d.grid_h);
    let (glow_a, glow_b) = (d.glow_primary, d.glow_warning);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x0, y0, w, h) = (
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            );
            // radial glows first so the grid stays crisp on top: centre as a fraction of the box, radius as a fraction of the farthest corner
            for (cx_f, cy_f, frac, color) in [
                (0.12, -0.12, glow_a.1, glow_a.0),
                (0.88, 0.08, glow_b.1, glow_b.0),
            ] {
                let (cx_, cy_) = (x0 + w * cx_f, y0 + h * cy_f);
                let far = ((cx_ - x0).max(x0 + w - cx_).powi(2)
                    + (cy_ - y0).max(y0 + h - cy_).powi(2))
                .sqrt();
                let radius = far * frac;
                const STEPS: usize = 14;
                for i in 0..STEPS {
                    let r = radius * (STEPS - i) as f32 / STEPS as f32;
                    let ring = Bounds::new(
                        point(px(cx_ - r), px(cy_ - r)),
                        size(px(r * 2.), px(r * 2.)),
                    );
                    window.paint_quad(
                        fill(ring, color.opacity(1.0 / STEPS as f32)).corner_radii(px(r)),
                    );
                }
            }
            let mut x = 0.;
            while x < w {
                window.paint_quad(fill(
                    Bounds::new(point(px(x0 + x), px(y0)), size(px(1.), px(h))),
                    line_v,
                ));
                x += 48.;
            }
            let mut y = 0.;
            while y < h {
                window.paint_quad(fill(
                    Bounds::new(point(px(x0), px(y0 + y)), size(px(w), px(1.))),
                    line_h,
                ));
                y += 48.;
            }
        },
    )
    .absolute()
    .size_full()
}

/// Colour names of the original's `<Tag color=..>`.
#[derive(Clone, Copy)]
pub enum TagColor {
    Blue,
    Green,
    Red,
    Orange,
}

/// `.ui-tag`: a pill, 1/7 padding, 12px text. Dark tones override every coloured variant with the neutral look.
pub fn ui_tag(text: impl Into<SharedString>, color: Option<TagColor>, cx: &App) -> Div {
    let ui = crate::theme::ui(cx);
    let (fg, bg, border): (Hsla, Hsla, Hsla) = if ui.is_dark {
        (
            ui.text_secondary,
            gpui_kit::white().opacity(0.06),
            gpui_kit::white().opacity(0.1),
        )
    } else {
        match color {
            None => (
                gpui_kit::rgb(0x526071).into(),
                gpui_kit::rgb(0xf4f6f8).into(),
                gpui_kit::rgba(0x26304414).into(),
            ),
            Some(TagColor::Blue) => (
                gpui_kit::rgb(0x0958d9).into(),
                gpui_kit::rgb(0xe6f4ff).into(),
                gpui_kit::rgb(0x91caff).into(),
            ),
            Some(TagColor::Green) => (
                gpui_kit::rgb(0x237804).into(),
                gpui_kit::rgb(0xf6ffed).into(),
                gpui_kit::rgb(0xb7eb8f).into(),
            ),
            Some(TagColor::Red) => (
                gpui_kit::rgb(0xa8071a).into(),
                gpui_kit::rgb(0xfff1f0).into(),
                gpui_kit::rgb(0xffa39e).into(),
            ),
            Some(TagColor::Orange) => (
                gpui_kit::rgb(0xad6800).into(),
                gpui_kit::rgb(0xfff7e6).into(),
                gpui_kit::rgb(0xffd591).into(),
            ),
        }
    };
    div()
        .flex()
        .items_center()
        .px(px(7.))
        .py(px(1.))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_size(px(12.))
        .line_height(relative(1.4))
        .text_color(fg)
        .child(text.into())
}

/// Native `<input type="number">` look: two tiny step arrows at the right edge that appear while the pointer is over
/// the field. `input` is the field itself, `state` its value; the arrows add / subtract one inside `[min, max]`.
pub fn spin(
    input: impl IntoElement,
    id: &str,
    state: &Entity<gpui_kit::component::input::InputState>,
    min: i64,
    max: Option<i64>,
    ui: crate::theme::UiColors,
) -> impl IntoElement {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;
    let group: SharedString = format!("{id}-spin").into();
    let step = |dir: &'static str, delta: i64, icon: IconName| {
        let state = state.clone();
        div()
            .id(SharedString::from(format!("{id}-{dir}")))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(ui.text_muted)
            .hover(move |d| d.text_color(ui.text).bg(ui.text.opacity(0.08)))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                let current = state.read(cx).value().trim().parse::<i64>().unwrap_or(min);
                let next = (current + delta).max(min).min(max.unwrap_or(i64::MAX));
                state.update(cx, |s, cx| s.set_value(next.to_string(), window, cx));
            })
            .child(Icon::new(icon).size(px(8.)))
            .test_support()
    };
    div().relative().group(group.clone()).child(input).child(
        v_flex()
            .absolute()
            .right(px(2.))
            .top(px(2.))
            .bottom(px(2.))
            .w(px(14.))
            .rounded(px(3.))
            .overflow_hidden()
            .bg(ui.input_bg)
            .opacity(0.)
            .group_hover(group, |d| d.opacity(1.))
            .child(step("up", 1, IconName::ChevronUp))
            .child(step("down", -1, IconName::ChevronDown)),
    )
}
