//! zh-CN / en-US dictionaries with `{name}` interpolation and a process-wide current language.
//!
//! The JSON files in `/locales` are generated from the Tauri app's `dictionaries.ts` by
//! `scripts/export-locales.mjs`; keys are identical so pages can be ported with their original `t('a.b')` calls.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static ZH_CN: &str = include_str!("../../../locales/zh-CN.json");
static EN_US: &str = include_str!("../../../locales/en-US.json");
// Keys that only exist in the GPUI build (the Tauri build hard-coded them in Rust). Kept out of the generated files.
static ZH_CN_EXTRA: &str = include_str!("../../../locales/extra/zh-CN.json");
static EN_US_EXTRA: &str = include_str!("../../../locales/extra/en-US.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    ZhCn,
    EnUs,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::ZhCn => "zh-CN",
            Language::EnUs => "en-US",
        }
    }
}

/// What the user picked in settings (`languageMode` in `general_settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageMode {
    System,
    ZhCn,
    EnUs,
}

impl LanguageMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "zh-CN" => Some(Self::ZhCn),
            "en-US" => Some(Self::EnUs),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }

    pub fn resolve(self) -> Language {
        match self {
            Self::System => resolve_system_language(),
            Self::ZhCn => Language::ZhCn,
            Self::EnUs => Language::EnUs,
        }
    }
}

/// Any Chinese locale => zh-CN, everything else => en-US (same rule as the Tauri build).
pub fn language_from_locale(locale: &str) -> Language {
    if locale.to_ascii_lowercase().starts_with("zh") {
        Language::ZhCn
    } else {
        Language::EnUs
    }
}

pub fn resolve_system_language() -> Language {
    let locales = sys_locale::get_locales().collect::<Vec<_>>();
    if locales
        .iter()
        .any(|l| language_from_locale(l) == Language::ZhCn)
    {
        Language::ZhCn
    } else {
        Language::EnUs
    }
}

fn parse(json: &'static str, extra: &'static str) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = serde_json::from_str(json).expect("locale json");
    map.extend(serde_json::from_str::<HashMap<String, String>>(extra).expect("extra locale json"));
    map
}

fn dictionary(lang: Language) -> &'static HashMap<String, String> {
    static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Language::ZhCn => ZH.get_or_init(|| parse(ZH_CN, ZH_CN_EXTRA)),
        Language::EnUs => EN.get_or_init(|| parse(EN_US, EN_US_EXTRA)),
    }
}

/// Raw template: requested language, then zh-CN, then the key itself.
pub fn template(lang: Language, key: &str) -> &str {
    dictionary(lang)
        .get(key)
        .or_else(|| dictionary(Language::ZhCn).get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

/// Replace `{name}` with the matching value; unknown placeholders are kept as-is.
pub fn interpolate(template: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    if values.is_empty() || !template.contains('{') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if is_ident(&after[..end]) => {
                let name = &after[..end];
                match values.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(&v.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn translate(lang: Language, key: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    interpolate(template(lang, key), values)
}

// ---- process-wide current language ----

fn current_cell() -> &'static RwLock<Language> {
    static CUR: OnceLock<RwLock<Language>> = OnceLock::new();
    CUR.get_or_init(|| RwLock::new(resolve_system_language()))
}

pub fn current_language() -> Language {
    *current_cell().read().unwrap()
}

pub fn set_current_language(lang: Language) {
    *current_cell().write().unwrap() = lang;
}

/// `t!("common.save")`, `t!("common.totalItems", count = 3)`.
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::translate($crate::current_language(), $key, &[])
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate($crate::current_language(), $key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            let after = &rest[i + 1..];
            if let Some(j) = after.find('}') {
                if is_ident(&after[..j]) {
                    out.insert(after[..j].to_string());
                }
                rest = &after[j + 1..];
            } else {
                break;
            }
        }
        out
    }

    #[test]
    fn both_dictionaries_have_the_same_keys() {
        let zh = dictionary(Language::ZhCn);
        let en = dictionary(Language::EnUs);
        assert!(zh.len() > 1000, "{}", zh.len());
        let only_zh: Vec<_> = zh.keys().filter(|k| !en.contains_key(*k)).collect();
        let only_en: Vec<_> = en.keys().filter(|k| !zh.contains_key(*k)).collect();
        assert!(
            only_zh.is_empty() && only_en.is_empty(),
            "zh-only: {only_zh:?} en-only: {only_en:?}"
        );
    }

    #[test]
    fn placeholders_match_between_languages() {
        let zh = dictionary(Language::ZhCn);
        let en = dictionary(Language::EnUs);
        let bad: Vec<_> = zh
            .iter()
            .filter(|(k, v)| placeholders(v) != placeholders(&en[*k]))
            .map(|(k, _)| k)
            .collect();
        assert!(bad.is_empty(), "placeholder mismatch: {bad:?}");
    }

    #[test]
    fn extra_keys_do_not_shadow_exported_ones_and_exist_in_both_languages() {
        let exported_zh: HashMap<String, String> = serde_json::from_str(ZH_CN).unwrap();
        let extra_zh: HashMap<String, String> = serde_json::from_str(ZH_CN_EXTRA).unwrap();
        let extra_en: HashMap<String, String> = serde_json::from_str(EN_US_EXTRA).unwrap();
        assert!(
            extra_zh.keys().all(|k| !exported_zh.contains_key(k)),
            "extra keys must be new"
        );
        assert_eq!(
            extra_zh.keys().collect::<BTreeSet<_>>(),
            extra_en.keys().collect::<BTreeSet<_>>()
        );
        assert_eq!(template(Language::EnUs, "tray.quit"), "Quit");
        assert_eq!(template(Language::ZhCn, "tray.show"), "显示主窗口");
    }

    #[test]
    fn lookup_falls_back_to_zh_then_key() {
        assert_eq!(template(Language::ZhCn, "common.save"), "保存");
        assert_eq!(template(Language::EnUs, "common.save"), "Save");
        assert_eq!(template(Language::EnUs, "no.such.key"), "no.such.key");
    }

    #[test]
    fn interpolation_replaces_named_values_and_keeps_unknown() {
        assert_eq!(
            translate(Language::EnUs, "common.totalItems", &[("count", &5)]),
            "5 items"
        );
        assert_eq!(interpolate("a {x} b {y}", &[("x", &1)]), "a 1 b {y}");
        assert_eq!(interpolate("brace { alone", &[("x", &1)]), "brace { alone");
        assert_eq!(interpolate("{x}{x}", &[("x", &"ab")]), "abab");
        assert_eq!(
            interpolate("json {\"a\": 1}", &[("a", &9)]),
            "json {\"a\": 1}"
        );
    }

    #[test]
    fn locale_mapping_and_modes() {
        assert_eq!(language_from_locale("zh-Hans-CN"), Language::ZhCn);
        assert_eq!(language_from_locale("zh_TW"), Language::ZhCn);
        assert_eq!(language_from_locale("en-US"), Language::EnUs);
        assert_eq!(language_from_locale("de-DE"), Language::EnUs);
        for m in [LanguageMode::System, LanguageMode::ZhCn, LanguageMode::EnUs] {
            assert_eq!(LanguageMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(LanguageMode::parse("fr"), None);
        assert_eq!(LanguageMode::ZhCn.resolve(), Language::ZhCn);
    }

    #[test]
    fn macro_uses_current_language() {
        set_current_language(Language::EnUs);
        assert_eq!(crate::t!("common.save"), "Save");
        assert_eq!(crate::t!("common.totalItems", count = 2), "2 items");
        set_current_language(Language::ZhCn);
        assert_eq!(crate::t!("common.save"), "保存");
    }
}
