//! Entrance / switch animations (port of the GSAP section of `MainLayout.tsx`). They run only when neither the
//! "reduce motion" setting nor the operating system asks for less motion.

use std::process::Command;
use std::time::Duration;

use gpui_kit::{
    ease_out_quint, Animation, AnimationExt as _, App, ElementId, Global, IntoElement, Styled,
};

/// Length of the page fade (the original used ~0.2 s tweens).
pub const PAGE_FADE: Duration = Duration::from_millis(180);

/// What the OS says about animations, read once in the background at start-up.
pub struct OsMotion {
    pub reduce: bool,
}

impl Global for OsMotion {}

pub fn init(cx: &mut App) {
    cx.set_global(OsMotion { reduce: false });
    cx.spawn(async move |cx| {
        let reduce = cx
            .background_executor()
            .spawn(async { detect_os_reduce_motion() })
            .await;
        let _ = cx.update(|cx| {
            cx.global_mut::<OsMotion>().reduce = reduce;
            cx.refresh_windows();
        });
    })
    .detach();
}

/// True when animations must not play.
pub fn reduced(cx: &App) -> bool {
    let os = cx.try_global::<OsMotion>().is_some_and(|m| m.reduce);
    crate::theme::current(cx)
        .settings
        .effective_reduce_motion(os)
}

/// Fade `element` in whenever its `key` is new (e.g. the selected page); returns it untouched when motion is reduced.
pub fn fade_in<E: IntoElement + Styled + 'static>(
    element: E,
    key: impl Into<ElementId>,
    cx: &App,
) -> gpui_kit::AnyElement {
    if reduced(cx) {
        return element.into_any_element();
    }
    element
        .with_animation(
            key,
            Animation::new(PAGE_FADE).with_easing(ease_out_quint()),
            |el, delta| el.opacity(delta),
        )
        .into_any_element()
}

fn detect_os_reduce_motion() -> bool {
    let run = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };
    if cfg!(target_os = "macos") {
        parse_os_reduce_motion(
            "macos",
            &run(
                "defaults",
                &["read", "com.apple.universalaccess", "reduceMotion"],
            )
            .unwrap_or_default(),
        )
    } else if cfg!(target_os = "windows") {
        parse_os_reduce_motion(
            "windows",
            &run(
                "reg",
                &[
                    "query",
                    r"HKCU\Control Panel\Desktop\WindowMetrics",
                    "/v",
                    "MinAnimate",
                ],
            )
            .unwrap_or_default(),
        )
    } else {
        parse_os_reduce_motion(
            "linux",
            &run(
                "gsettings",
                &["get", "org.gnome.desktop.interface", "enable-animations"],
            )
            .unwrap_or_default(),
        )
    }
}

/// Interpret the output of the platform query (empty output = nothing known = animations allowed).
pub fn parse_os_reduce_motion(os: &str, output: &str) -> bool {
    let out = output.trim();
    match os {
        // `defaults read` prints 1 / 0
        "macos" => out == "1",
        // `reg query` prints `MinAnimate    REG_SZ    0` when window animations are switched off
        "windows" => out
            .lines()
            .any(|l| l.contains("MinAnimate") && l.trim_end().ends_with('0')),
        // `gsettings get` prints true / false
        _ => out == "false",
    }
}

#[cfg(test)]
mod tests {
    use super::parse_os_reduce_motion;

    #[test]
    fn os_answers_are_interpreted_per_platform() {
        assert!(parse_os_reduce_motion("macos", "1\n"));
        assert!(!parse_os_reduce_motion("macos", "0\n"));
        assert!(!parse_os_reduce_motion("macos", ""));
        assert!(parse_os_reduce_motion(
            "windows",
            "\nHKEY_CURRENT_USER\\...\\WindowMetrics\n    MinAnimate    REG_SZ    0\n"
        ));
        assert!(!parse_os_reduce_motion(
            "windows",
            "    MinAnimate    REG_SZ    1\n"
        ));
        assert!(parse_os_reduce_motion("linux", "false\n"));
        assert!(!parse_os_reduce_motion("linux", "true\n"));
        assert!(!parse_os_reduce_motion("linux", ""));
    }
}
