//! GPUI key → X11 keysym (what the original got from noVNC's keyboard handling).

pub const XK_BACKSPACE: u32 = 0xff08;
pub const XK_TAB: u32 = 0xff09;
pub const XK_RETURN: u32 = 0xff0d;
pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_INSERT: u32 = 0xff63;
pub const XK_DELETE: u32 = 0xffff;
pub const XK_HOME: u32 = 0xff50;
pub const XK_LEFT: u32 = 0xff51;
pub const XK_UP: u32 = 0xff52;
pub const XK_RIGHT: u32 = 0xff53;
pub const XK_DOWN: u32 = 0xff54;
pub const XK_PAGE_UP: u32 = 0xff55;
pub const XK_PAGE_DOWN: u32 = 0xff56;
pub const XK_END: u32 = 0xff57;
pub const XK_F1: u32 = 0xffbe;
pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_CAPS_LOCK: u32 = 0xffe5;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_SUPER_L: u32 = 0xffeb;

/// Keysym of a Unicode character: Latin-1 maps to itself, everything else to `0x01000000 | cp`.
pub fn keysym_for_char(c: char) -> u32 {
    let cp = c as u32;
    if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) {
        cp
    } else {
        0x0100_0000 | cp
    }
}

/// `key` is GPUI's key name; `key_char` the text the key produces (layout and shift aware).
pub fn keysym_for_key(key: &str, key_char: Option<&str>) -> Option<u32> {
    let special = match key {
        "backspace" => XK_BACKSPACE,
        "tab" => XK_TAB,
        "enter" => XK_RETURN,
        "escape" => XK_ESCAPE,
        "insert" => XK_INSERT,
        "delete" => XK_DELETE,
        "home" => XK_HOME,
        "end" => XK_END,
        "pageup" => XK_PAGE_UP,
        "pagedown" => XK_PAGE_DOWN,
        "left" => XK_LEFT,
        "right" => XK_RIGHT,
        "up" => XK_UP,
        "down" => XK_DOWN,
        "capslock" => XK_CAPS_LOCK,
        "space" => 0x20,
        k if k.len() >= 2 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            let n: u32 = k[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            XK_F1 + n - 1
        }
        _ => {
            let text = key_char.filter(|t| t.chars().count() == 1).unwrap_or(key);
            let mut chars = text.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            return Some(keysym_for_char(c));
        }
    };
    Some(special)
}

/// Modifier keys held, for press/release events on modifier changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub platform: bool,
}

/// `(keysym, down)` for every modifier that changed.
pub fn modifier_changes(prev: Mods, now: Mods) -> Vec<(u32, bool)> {
    let mut out = Vec::new();
    for (changed, sym, down) in [
        (prev.shift != now.shift, XK_SHIFT_L, now.shift),
        (prev.control != now.control, XK_CONTROL_L, now.control),
        (prev.alt != now.alt, XK_ALT_L, now.alt),
        (prev.platform != now.platform, XK_SUPER_L, now.platform),
    ] {
        if changed {
            out.push((sym, down));
        }
    }
    out
}

/// The Ctrl+Alt+Del chord as `(keysym, down)` events.
pub fn ctrl_alt_del() -> [(u32, bool); 6] {
    [(XK_CONTROL_L, true), (XK_ALT_L, true), (XK_DELETE, true), (XK_DELETE, false), (XK_ALT_L, false), (XK_CONTROL_L, false)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_keysyms() {
        assert_eq!(keysym_for_key("a", Some("a")), Some(0x61));
        assert_eq!(keysym_for_key("a", Some("A")), Some(0x41));
        assert_eq!(keysym_for_key("enter", None), Some(XK_RETURN));
        assert_eq!(keysym_for_key("f5", None), Some(XK_F1 + 4));
        assert_eq!(keysym_for_key("f99", None), None);
        assert_eq!(keysym_for_key("space", Some(" ")), Some(0x20));
        assert_eq!(keysym_for_key("é", Some("é")), Some(0xe9));
        assert_eq!(keysym_for_key("中", Some("中")), Some(0x0100_4e2d));
        assert_eq!(keysym_for_key("x", Some("")), Some(0x78), "falls back to the key name");
        assert_eq!(keysym_for_key("unknownkey", None), None);
    }

    #[test]
    fn modifier_changes_and_chord() {
        let none = Mods::default();
        let both = Mods { shift: true, platform: true, ..none };
        assert_eq!(modifier_changes(none, both), vec![(XK_SHIFT_L, true), (XK_SUPER_L, true)]);
        assert_eq!(modifier_changes(both, none), vec![(XK_SHIFT_L, false), (XK_SUPER_L, false)]);
        assert!(modifier_changes(both, both).is_empty());
        let chord = ctrl_alt_del();
        assert_eq!(chord[2], (XK_DELETE, true));
        assert_eq!(chord[5], (XK_CONTROL_L, false));
    }
}
