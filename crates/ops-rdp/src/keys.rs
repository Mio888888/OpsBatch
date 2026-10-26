//! Keyboard mapping for the RDP window and the AI agent (port of `getScancodeForKey` / `getRdpKeyboardInputEvents`
//! in `pages/Rdp/rdpProtocol.ts`). Key names are the DOM ones the original used (`Enter`, `ArrowUp`, `KeyA`…);
//! `web_key_name` translates GPUI key names to them.

use super::types::RdpInputEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scancode {
    pub code: u8,
    pub extended: bool,
}

const fn sc(code: u8, extended: bool) -> Option<Scancode> {
    Some(Scancode { code, extended })
}

const CONTROL: Scancode = Scancode { code: 0x1d, extended: false };

/// `key` is the DOM `KeyboardEvent.key`, `code` the DOM `KeyboardEvent.code` (may be empty).
pub fn scancode_for_key(key: &str, code: &str) -> Option<Scancode> {
    if let Some(s) = scancode_for_code(code) {
        return Some(s);
    }
    match key {
        "Backspace" => sc(0x0e, false),
        "Tab" => sc(0x0f, false),
        "Enter" => sc(0x1c, false),
        "Escape" => sc(0x01, false),
        "Delete" => sc(0x53, true),
        "Insert" => sc(0x52, true),
        "Home" => sc(0x47, true),
        "End" => sc(0x4f, true),
        "PageUp" => sc(0x49, true),
        "PageDown" => sc(0x51, true),
        "ArrowUp" => sc(0x48, true),
        "ArrowDown" => sc(0x50, true),
        "ArrowLeft" => sc(0x4b, true),
        "ArrowRight" => sc(0x4d, true),
        "F1" => sc(0x3b, false),
        "F2" => sc(0x3c, false),
        "F3" => sc(0x3d, false),
        "F4" => sc(0x3e, false),
        "F5" => sc(0x3f, false),
        "F6" => sc(0x40, false),
        "F7" => sc(0x41, false),
        "F8" => sc(0x42, false),
        "F9" => sc(0x43, false),
        "F10" => sc(0x44, false),
        "F11" => sc(0x57, false),
        "F12" => sc(0x58, false),
        "Shift" => sc(0x2a, false),
        "Control" => sc(0x1d, false),
        "Alt" => sc(0x38, false),
        " " => sc(0x39, false),
        _ => None,
    }
}

fn scancode_for_code(code: &str) -> Option<Scancode> {
    let digit_row = ["Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9", "Digit0", "Minus", "Equal"];
    if let Some(i) = digit_row.iter().position(|c| *c == code) {
        return sc(0x02 + i as u8, false);
    }
    let rows: [(&str, u8); 3] = [("QWERTYUIOP", 0x10), ("ASDFGHJKL", 0x1e), ("ZXCVBNM", 0x2c)];
    if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
        let ch = letter.as_bytes()[0] as char;
        for (row, base) in rows {
            if let Some(i) = row.find(ch) {
                return sc(base + i as u8, false);
            }
        }
    }
    match code {
        "Backquote" => sc(0x29, false),
        "BracketLeft" => sc(0x1a, false),
        "BracketRight" => sc(0x1b, false),
        "Semicolon" => sc(0x27, false),
        "Quote" => sc(0x28, false),
        "Backslash" => sc(0x2b, false),
        "Comma" => sc(0x33, false),
        "Period" => sc(0x34, false),
        "Slash" => sc(0x35, false),
        "Space" => sc(0x39, false),
        "NumpadEnter" => sc(0x1c, true),
        "ControlLeft" => sc(0x1d, false),
        "ControlRight" => sc(0x1d, true),
        "ShiftLeft" => sc(0x2a, false),
        "ShiftRight" => sc(0x36, false),
        "AltLeft" => sc(0x38, false),
        "AltRight" => sc(0x38, true),
        "MetaLeft" => sc(0x5b, true),
        "MetaRight" => sc(0x5c, true),
        _ => None,
    }
}

/// DOM `code` of a printable US-layout character key, e.g. `a` → `KeyA`, `;` → `Semicolon`.
pub fn dom_code_for_char(ch: char) -> Option<String> {
    Some(match ch {
        'a'..='z' | 'A'..='Z' => format!("Key{}", ch.to_ascii_uppercase()),
        '0'..='9' => format!("Digit{ch}"),
        '-' => "Minus".into(),
        '=' => "Equal".into(),
        '[' => "BracketLeft".into(),
        ']' => "BracketRight".into(),
        ';' => "Semicolon".into(),
        '\'' => "Quote".into(),
        '\\' => "Backslash".into(),
        ',' => "Comma".into(),
        '.' => "Period".into(),
        '/' => "Slash".into(),
        '`' => "Backquote".into(),
        ' ' => "Space".into(),
        _ => return None,
    })
}

/// GPUI key name (`enter`, `up`, `f5`, `a`…) → DOM `key` name.
pub fn web_key_name(gpui_key: &str) -> String {
    match gpui_key {
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "insert" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "space" => " ".into(),
        k if k.len() >= 2 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => k.to_uppercase(),
        k => k.to_string(),
    }
}

/// Modifier state as GPUI reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    pub control: bool,
    pub alt: bool,
    /// ⌘ on macOS, Windows key elsewhere — mapped to Ctrl for shortcuts like the original did.
    pub platform: bool,
}

/// Port of `getRdpKeyboardInputEvents`. `key_char` is the text the key would insert (layout aware).
pub fn keyboard_input_events(key: &str, key_char: Option<&str>, mods: KeyModifiers, down: bool, repeat: bool) -> Vec<RdpInputEvent> {
    let web_key = web_key_name(key);
    if web_key == "Meta" {
        return Vec::new();
    }
    let code = web_key.chars().next().filter(|_| web_key.chars().count() == 1).and_then(dom_code_for_char).unwrap_or_default();
    let special = scancode_for_key(&web_key, &code);
    let has_modifier = mods.control || mods.platform || mods.alt;
    let single_char = web_key.chars().count() == 1;
    let key_scancode = if single_char { scancode_for_key(&web_key, &code) } else { special };
    let ev = |s: Scancode, down: bool| RdpInputEvent::KeyScancode { code: s.code, extended: s.extended, down };

    if mods.platform && single_char {
        if let Some(ks) = key_scancode {
            if repeat && down {
                return vec![ev(ks, true)];
            }
            return if down { vec![ev(CONTROL, true), ev(ks, true)] } else { vec![ev(ks, false), ev(CONTROL, false)] };
        }
    }
    if has_modifier {
        if let Some(ks) = key_scancode {
            return vec![ev(ks, down)];
        }
    }
    if let Some(s) = special {
        return vec![ev(s, down)];
    }
    // Non-US characters etc.: send the text the key produces.
    let text = key_char.filter(|t| t.chars().count() == 1).or(single_char.then_some(web_key.as_str()));
    match text.and_then(|t| t.chars().next()) {
        Some(c) => vec![RdpInputEvent::Unicode { character: c.to_string(), down }],
        None => Vec::new(),
    }
}

/// Press/release events for modifier changes (Shift/Ctrl/Alt); ⌘ is not forwarded.
pub fn modifier_events(prev_shift: bool, shift: bool, prev: KeyModifiers, now: KeyModifiers) -> Vec<RdpInputEvent> {
    let mut out = Vec::new();
    let mut push = |changed: bool, s: Scancode, down: bool| {
        if changed {
            out.push(RdpInputEvent::KeyScancode { code: s.code, extended: s.extended, down });
        }
    };
    push(prev_shift != shift, Scancode { code: 0x2a, extended: false }, shift);
    push(prev.control != now.control, CONTROL, now.control);
    push(prev.alt != now.alt, Scancode { code: 0x38, extended: false }, now.alt);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_ev(code: u8, extended: bool, down: bool) -> RdpInputEvent {
        RdpInputEvent::KeyScancode { code, extended, down }
    }

    #[test]
    fn letters_digits_and_specials_have_scancodes() {
        assert_eq!(scancode_for_key("a", "KeyA"), sc(0x1e, false));
        assert_eq!(scancode_for_key("q", "KeyQ"), sc(0x10, false));
        assert_eq!(scancode_for_key("m", "KeyM"), sc(0x32, false));
        assert_eq!(scancode_for_key("1", "Digit1"), sc(0x02, false));
        assert_eq!(scancode_for_key("0", "Digit0"), sc(0x0b, false));
        assert_eq!(scancode_for_key("=", "Equal"), sc(0x0d, false));
        assert_eq!(scancode_for_key("Enter", ""), sc(0x1c, false));
        assert_eq!(scancode_for_key("ArrowLeft", ""), sc(0x4b, true));
        assert_eq!(scancode_for_key("F11", ""), sc(0x57, false));
        assert_eq!(scancode_for_key("x", "MetaRight"), sc(0x5c, true));
        assert_eq!(scancode_for_key("€", ""), None);
    }

    #[test]
    fn plain_keys_are_scancodes_and_odd_characters_are_unicode() {
        assert_eq!(keyboard_input_events("a", Some("a"), KeyModifiers::default(), true, false), vec![key_ev(0x1e, false, true)]);
        assert_eq!(keyboard_input_events("enter", None, KeyModifiers::default(), false, false), vec![key_ev(0x1c, false, false)]);
        assert_eq!(keyboard_input_events("up", None, KeyModifiers::default(), true, false), vec![key_ev(0x48, true, true)]);
        assert_eq!(
            keyboard_input_events("é", Some("é"), KeyModifiers::default(), true, false),
            vec![RdpInputEvent::Unicode { character: "é".into(), down: true }]
        );
        assert!(keyboard_input_events("meta", None, KeyModifiers::default(), true, false).is_empty());
    }

    #[test]
    fn command_shortcuts_become_control_shortcuts() {
        let cmd = KeyModifiers { platform: true, ..Default::default() };
        assert_eq!(keyboard_input_events("c", Some("c"), cmd, true, false), vec![key_ev(0x1d, false, true), key_ev(0x2e, false, true)]);
        assert_eq!(keyboard_input_events("c", Some("c"), cmd, false, false), vec![key_ev(0x2e, false, false), key_ev(0x1d, false, false)]);
        assert_eq!(keyboard_input_events("c", Some("c"), cmd, true, true), vec![key_ev(0x2e, false, true)]);
        let ctrl = KeyModifiers { control: true, ..Default::default() };
        assert_eq!(keyboard_input_events("v", Some("v"), ctrl, true, false), vec![key_ev(0x2f, false, true)]);
    }

    #[test]
    fn gpui_key_names_map_to_dom_names() {
        assert_eq!(web_key_name("pagedown"), "PageDown");
        assert_eq!(web_key_name("space"), " ");
        assert_eq!(web_key_name("f12"), "F12");
        assert_eq!(web_key_name("a"), "a");
        assert_eq!(keyboard_input_events("space", Some(" "), KeyModifiers::default(), true, false), vec![key_ev(0x39, false, true)]);
    }

    #[test]
    fn modifier_changes_emit_press_and_release() {
        let none = KeyModifiers::default();
        let ctrl = KeyModifiers { control: true, ..none };
        assert_eq!(modifier_events(false, true, none, ctrl), vec![key_ev(0x2a, false, true), key_ev(0x1d, false, true)]);
        assert_eq!(modifier_events(true, false, ctrl, none), vec![key_ev(0x2a, false, false), key_ev(0x1d, false, false)]);
        assert!(modifier_events(false, false, none, none).is_empty());
    }
}
