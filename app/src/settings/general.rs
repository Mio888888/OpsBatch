//! General settings: language, default concurrency/timeout, tray behaviour, log retention.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, v_flex, Icon};
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::{t, LanguageMode};
use std::collections::HashMap;

use super::{group, row};
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::toast;

pub const DEFAULT_CONCURRENCY: u32 = 10;
pub const DEFAULT_TIMEOUT: u32 = 30;
pub const DEFAULT_RETENTION_DAYS: u32 = 90;

/// Parse a number field; unusable input falls back to `default`, everything is clamped into `min..=max`.
pub fn clamp_number(text: &str, min: u32, max: u32, default: u32) -> u32 {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (v.round().max(0.0) as u64).clamp(min as u64, max as u64) as u32).unwrap_or(default)
}

/// Values stored by "Save" (same keys and string formats as the Tauri build).
pub fn general_map(concurrency: u32, timeout: u32, minimize_to_tray: bool, retention_days: u32) -> HashMap<String, String> {
    HashMap::from([
        ("defaultConcurrency".to_string(), concurrency.to_string()),
        ("defaultTimeout".to_string(), timeout.to_string()),
        ("minimizeToTray".to_string(), minimize_to_tray.to_string()),
        ("logRetentionDays".to_string(), retention_days.to_string()),
    ])
}

pub struct GeneralSection {
    language: Entity<ChoiceSelect>,
    concurrency: Entity<InputState>,
    timeout: Entity<InputState>,
    retention: Entity<InputState>,
    minimize: bool,
    loaded: bool,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl GeneralSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let modes = vec![
            Choice::new(LanguageMode::System.as_str(), t!("settings.language.followSystem")),
            Choice::new(LanguageMode::ZhCn.as_str(), t!("settings.language.zhCN")),
            Choice::new(LanguageMode::EnUs.as_str(), t!("settings.language.enUS")),
        ];
        let current = crate::language::current_mode(cx).as_str();
        let language = cx.new(|cx| new_select(modes, current, window, cx));
        let subs = vec![cx.subscribe_in(&language, window, |_, _, ev, window, cx| {
            if let Some(mode) = confirmed_value(ev).and_then(|v| LanguageMode::parse(&v)) {
                if mode != crate::language::current_mode(cx) {
                    crate::language::set_mode(mode, cx);
                    toast::success(window, cx, t!("settings.languageSaved"));
                }
            }
        })];
        let number = |window: &mut Window, cx: &mut Context<Self>, v: u32| cx.new(|cx| InputState::new(window, cx).default_value(v.to_string()));
        let mut this = Self {
            language,
            concurrency: number(window, cx, DEFAULT_CONCURRENCY),
            timeout: number(window, cx, DEFAULT_TIMEOUT),
            retention: number(window, cx, DEFAULT_RETENTION_DAYS),
            minimize: true,
            loaded: false,
            saving: false,
            _subs: subs,
        };
        this.load(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn_in(window, async move |this, cx| {
            let map = ops_core::rt::run(async move { ops_core::settings::get_general_settings(&ctx.db).await }).await.and_then(|r| r).unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                let get = |k: &str, min, max, d| clamp_number(map.get(k).map(String::as_str).unwrap_or(""), min, max, d);
                let (c, t, r) = (get("defaultConcurrency", 1, 100, DEFAULT_CONCURRENCY), get("defaultTimeout", 1, 3600, DEFAULT_TIMEOUT), get("logRetentionDays", 7, 365, DEFAULT_RETENTION_DAYS));
                this.concurrency.update(cx, |s, cx| s.set_value(c.to_string(), window, cx));
                this.timeout.update(cx, |s, cx| s.set_value(t.to_string(), window, cx));
                this.retention.update(cx, |s, cx| s.set_value(r.to_string(), window, cx));
                this.minimize = map.get("minimizeToTray").map(|v| v != "false").unwrap_or(true);
                this.loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn minimize_to_tray(&self) -> bool {
        self.minimize
    }

    pub fn set_minimize_to_tray(&mut self, on: bool, cx: &mut Context<Self>) {
        self.minimize = on;
        cx.notify();
    }

    pub fn inputs(&self) -> [&Entity<InputState>; 3] {
        [&self.concurrency, &self.timeout, &self.retention]
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let read = |s: &Entity<InputState>, min, max, d, cx: &App| clamp_number(s.read(cx).value().as_ref(), min, max, d);
        let (c, t, r) = (read(&self.concurrency, 1, 100, DEFAULT_CONCURRENCY, cx), read(&self.timeout, 1, 3600, DEFAULT_TIMEOUT, cx), read(&self.retention, 7, 365, DEFAULT_RETENTION_DAYS, cx));
        // show the clamped values the user actually gets
        self.concurrency.update(cx, |s, cx| s.set_value(c.to_string(), window, cx));
        self.timeout.update(cx, |s, cx| s.set_value(t.to_string(), window, cx));
        self.retention.update(cx, |s, cx| s.set_value(r.to_string(), window, cx));
        let map = general_map(c, t, self.minimize, r);
        let ctx = crate::services::ctx(cx);
        self.saving = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = ops_core::rt::run(async move { ops_core::settings::save_general_settings(&ctx.db, map).await }).await.and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match res {
                    Ok(()) => toast::success(window, cx, t!("settings.generalSaved")),
                    Err(e) => toast::error(window, cx, t!("common.operationFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for GeneralSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let ui = crate::theme::ui(cx);
        // `.ui-input-addon`: the unit is a cell attached to the input's right edge (120px in all)
        let num = move |input: &Entity<InputState>, suffix: Option<String>, id: &'static str, min: i64, max: i64| match suffix {
            None => div().w(px(120.)).child(crate::widgets::form::spin(Input::new(input), id, input, min, Some(max), ui)).into_any_element(),
            Some(unit) => gpui_kit::component::h_flex()
                .w(px(120.))
                .h(px(32.))
                .overflow_hidden()
                .rounded(px(8.))
                .border_1()
                .border_color(ui.input_border)
                .bg(ui.input_bg)
                .child(div().flex_1().min_w_0().child(crate::widgets::form::spin(Input::new(input).appearance(false), id, input, min, Some(max), ui)))
                .child(div().h_full().px(px(10.)).flex().items_center().border_l_1().border_color(ui.input_border).bg(if ui.is_dark { gpui_kit::white().opacity(0.04) } else { gpui_kit::rgb(0xf5f5f5).into() }).text_size(px(12.)).text_color(if ui.is_dark { ui.text_muted } else { gpui_kit::rgb(0x7c8794).into() }).child(unit))
                .into_any_element(),
        };
        v_flex()
            .id("settings-general")
            .test_support()
            .gap_5()
            .child(group(t!("settings.generalGroupLanguage"), t!("settings.languageExtra"), vec![row(t!("settings.language"), None, div().w(px(180.)).child(Select::new(&self.language)), cx)], cx))
            .child(group(
                t!("settings.generalGroupExecution"),
                t!("settings.generalGroupExecutionDesc"),
                vec![
                    row(t!("settings.defaultConcurrency"), Some(t!("settings.defaultConcurrencyDesc").into()), num(&self.concurrency, None, "set-concurrency", 1, 100), cx),
                    row(t!("settings.defaultTimeout"), Some(t!("settings.defaultTimeoutDesc").into()), num(&self.timeout, Some(t!("settings.seconds")), "set-timeout", 1, 3600), cx),
                ],
                cx,
            ))
            .child(group(
                t!("settings.generalGroupBehavior"),
                t!("settings.generalGroupBehaviorDesc"),
                vec![
                    row(
                        t!("settings.minimizeToTray"),
                        Some(t!("settings.minimizeToTrayDesc").into()),
                        Switch::new("set-tray").checked(self.minimize).on_click(move |on, _, cx| {
                            let on = *on;
                            let _ = weak.update(cx, |this, cx| this.set_minimize_to_tray(on, cx));
                        }),
                        cx,
                    ),
                    row(t!("settings.logRetentionDays"), Some(t!("settings.logRetentionDaysDesc").into()), num(&self.retention, Some(t!("settings.days")), "set-retention", 7, 365), cx),
                ],
                cx,
            ))
            .child(h_flex().justify_end().child(Button::new("set-general-save").primary().icon(Icon::new(IconName::Save)).label(t!("common.save")).loading(self.saving).on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))))
    }
}

#[cfg(test)]
mod tests {
    use super::{clamp_number, general_map};

    #[test]
    fn numbers_are_clamped_and_fall_back() {
        assert_eq!(clamp_number("500", 1, 100, 10), 100);
        assert_eq!(clamp_number("0", 1, 100, 10), 1);
        assert_eq!(clamp_number("-4", 7, 365, 90), 7);
        assert_eq!(clamp_number(" 12.6 ", 1, 100, 10), 13);
        assert_eq!(clamp_number("abc", 1, 3600, 30), 30);
        assert_eq!(clamp_number("", 1, 3600, 30), 30);
    }

    #[test]
    fn saved_map_uses_the_tauri_keys() {
        let m = general_map(8, 60, false, 30);
        assert_eq!(m["defaultConcurrency"], "8");
        assert_eq!(m["defaultTimeout"], "60");
        assert_eq!(m["minimizeToTray"], "false");
        assert_eq!(m["logRetentionDays"], "30");
    }
}
