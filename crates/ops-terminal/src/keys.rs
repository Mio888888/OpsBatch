//! Keyboard and mouse -> terminal byte sequences (xterm-compatible), independent of any UI toolkit.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    pub fn none() -> Self {
        Self::default()
    }

    /// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4); `None` when no modifier is held.
    fn param(self) -> Option<u8> {
        let v = self.shift as u8 | (self.alt as u8) << 1 | (self.ctrl as u8) << 2;
        (v != 0).then_some(v + 1)
    }
}

/// Terminal modes that change the encoding.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyModes {
    /// DECCKM: cursor keys send `ESC O x` instead of `ESC [ x`.
    pub app_cursor: bool,
    /// On macOS Option composes characters (é, ß); only send ESC+key when no composed text exists.
    pub option_composes_text: bool,
}

fn csi_final(final_byte: char, mods: Mods, app_cursor: bool) -> Vec<u8> {
    match mods.param() {
        Some(m) => format!("\x1b[1;{m}{final_byte}").into_bytes(),
        None if app_cursor => format!("\x1bO{final_byte}").into_bytes(),
        None => format!("\x1b[{final_byte}").into_bytes(),
    }
}

fn tilde(code: u8, mods: Mods) -> Vec<u8> {
    match mods.param() {
        Some(m) => format!("\x1b[{code};{m}~").into_bytes(),
        None => format!("\x1b[{code}~").into_bytes(),
    }
}

fn ss3_or_csi(final_byte: char, mods: Mods) -> Vec<u8> {
    match mods.param() {
        Some(m) => format!("\x1b[1;{m}{final_byte}").into_bytes(),
        None => format!("\x1bO{final_byte}").into_bytes(),
    }
}

fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' ' | '@' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Bytes to send for a key press. `key` is the GPUI key name ("enter", "up", "a", ...), `key_char` the text the
/// key would insert. Returns `None` when the key produces nothing (pure modifier, unknown key).
pub fn encode_key(
    key: &str,
    key_char: Option<&str>,
    mods: Mods,
    modes: KeyModes,
) -> Option<Vec<u8>> {
    let with_alt = |mut v: Vec<u8>| {
        if mods.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    let seq = match key {
        "enter" => with_alt(vec![b'\r']),
        "escape" => vec![0x1b],
        "tab" => {
            if mods.shift {
                b"\x1b[Z".to_vec()
            } else {
                with_alt(vec![b'\t'])
            }
        }
        "backspace" => {
            if mods.ctrl {
                vec![0x08]
            } else {
                with_alt(vec![0x7f])
            }
        }
        "up" => csi_final('A', mods, modes.app_cursor),
        "down" => csi_final('B', mods, modes.app_cursor),
        "right" => csi_final('C', mods, modes.app_cursor),
        "left" => csi_final('D', mods, modes.app_cursor),
        "home" => csi_final('H', mods, modes.app_cursor),
        "end" => csi_final('F', mods, modes.app_cursor),
        "pageup" => tilde(5, mods),
        "pagedown" => tilde(6, mods),
        "insert" => tilde(2, mods),
        "delete" => tilde(3, mods),
        "f1" => ss3_or_csi('P', mods),
        "f2" => ss3_or_csi('Q', mods),
        "f3" => ss3_or_csi('R', mods),
        "f4" => ss3_or_csi('S', mods),
        "f5" => tilde(15, mods),
        "f6" => tilde(17, mods),
        "f7" => tilde(18, mods),
        "f8" => tilde(19, mods),
        "f9" => tilde(20, mods),
        "f10" => tilde(21, mods),
        "f11" => tilde(23, mods),
        "f12" => tilde(24, mods),
        "space" => {
            if mods.ctrl {
                vec![0]
            } else {
                with_alt(vec![b' '])
            }
        }
        _ => {
            let mut chars = key.chars();
            let first = chars.next()?;
            let single = chars.next().is_none();
            if mods.ctrl && single {
                with_alt(vec![ctrl_byte(first)?])
            } else if mods.alt && single {
                match key_char.filter(|t| modes.option_composes_text && !t.is_empty() && *t != key)
                {
                    Some(composed) => composed.as_bytes().to_vec(),
                    None => {
                        let text = if mods.shift {
                            key_char.unwrap_or(key)
                        } else {
                            key
                        };
                        let mut v = vec![0x1b];
                        v.extend_from_slice(text.as_bytes());
                        v
                    }
                }
            } else {
                key_char
                    .filter(|t| !t.is_empty())
                    .map(|t| t.as_bytes().to_vec())?
            }
        }
    };
    Some(seq)
}

/// Wrap pasted text for bracketed-paste mode; strips embedded end markers so a paste cannot escape the bracket.
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let clean = text.replace("\x1b[201~", "").replace("\x1b[200~", "");
    format!("\x1b[200~{clean}\x1b[201~").into_bytes()
}

/// Plain paste: newlines become carriage returns like a keyboard would send.
pub fn plain_paste(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// SGR (1006) mouse report. `col`/`row` are 0-based cells. Returns `None` for actions the encoding cannot express.
pub fn encode_mouse_sgr(
    button: MouseButton,
    action: MouseAction,
    mods: Mods,
    col: usize,
    row: usize,
) -> Option<Vec<u8>> {
    let mut b: u32 = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if matches!(button, MouseButton::WheelUp | MouseButton::WheelDown)
        && action != MouseAction::Press
    {
        return None;
    }
    if action == MouseAction::Motion {
        b += 32;
    }
    if mods.shift {
        b += 4;
    }
    if mods.alt {
        b += 8;
    }
    if mods.ctrl {
        b += 16;
    }
    let last = if action == MouseAction::Release {
        'm'
    } else {
        'M'
    };
    Some(format!("\x1b[<{};{};{}{}", b, col + 1, row + 1, last).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: &str, ch: Option<&str>, m: Mods) -> Option<Vec<u8>> {
        encode_key(key, ch, m, KeyModes::default())
    }
    fn s(v: Option<Vec<u8>>) -> String {
        String::from_utf8(v.unwrap())
            .unwrap()
            .escape_default()
            .to_string()
    }
    const CTRL: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: true,
    };
    const ALT: Mods = Mods {
        shift: false,
        alt: true,
        ctrl: false,
    };
    const SHIFT: Mods = Mods {
        shift: true,
        alt: false,
        ctrl: false,
    };

    #[test]
    fn plain_text_and_named_keys() {
        assert_eq!(s(enc("a", Some("a"), Mods::none())), "a");
        assert_eq!(s(enc("a", Some("A"), SHIFT)), "A");
        assert_eq!(
            s(enc("你", Some("你"), Mods::none())),
            "你".escape_default().to_string()
        );
        assert_eq!(s(enc("enter", None, Mods::none())), "\\r");
        assert_eq!(s(enc("backspace", None, Mods::none())), "\\u{7f}");
        assert_eq!(s(enc("escape", None, Mods::none())), "\\u{1b}");
        assert_eq!(s(enc("tab", None, Mods::none())), "\\t");
        assert_eq!(s(enc("tab", None, SHIFT)), "\\u{1b}[Z");
        assert_eq!(s(enc("space", Some(" "), Mods::none())), " ");
        assert!(enc("shift", None, SHIFT).is_none());
    }

    #[test]
    fn ctrl_combinations() {
        assert_eq!(enc("c", Some("c"), CTRL), Some(vec![3]));
        assert_eq!(enc("a", Some("a"), CTRL), Some(vec![1]));
        assert_eq!(enc("z", Some("z"), CTRL), Some(vec![26]));
        assert_eq!(enc("space", None, CTRL), Some(vec![0]));
        assert_eq!(enc("[", Some("["), CTRL), Some(vec![0x1b]));
        assert_eq!(enc("\\", Some("\\"), CTRL), Some(vec![0x1c]));
        assert_eq!(enc("backspace", None, CTRL), Some(vec![8]));
        assert!(enc("1", Some("1"), CTRL).is_none(), "no ctrl mapping for 1");
    }

    #[test]
    fn alt_sends_escape_prefix_unless_the_platform_composes_text() {
        assert_eq!(enc("f", Some("f"), ALT), Some(vec![0x1b, b'f']));
        assert_eq!(enc("enter", None, ALT), Some(vec![0x1b, b'\r']));
        assert_eq!(enc("backspace", None, ALT), Some(vec![0x1b, 0x7f]));
        let mac = KeyModes {
            option_composes_text: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key("s", Some("ß"), ALT, mac),
            Some("ß".as_bytes().to_vec())
        );
        assert_eq!(encode_key("s", Some("s"), ALT, mac), Some(vec![0x1b, b's']));
    }

    #[test]
    fn cursor_keys_follow_application_mode_and_modifiers() {
        assert_eq!(s(enc("up", None, Mods::none())), "\\u{1b}[A");
        let app = KeyModes {
            app_cursor: true,
            ..Default::default()
        };
        assert_eq!(s(encode_key("up", None, Mods::none(), app)), "\\u{1b}OA");
        assert_eq!(s(enc("right", None, CTRL)), "\\u{1b}[1;5C");
        assert_eq!(
            s(encode_key("left", None, ALT, app)),
            "\\u{1b}[1;3D",
            "modifiers force CSI even in app mode"
        );
        assert_eq!(s(enc("home", None, Mods::none())), "\\u{1b}[H");
        assert_eq!(s(enc("end", None, SHIFT)), "\\u{1b}[1;2F");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(s(enc("pageup", None, Mods::none())), "\\u{1b}[5~");
        assert_eq!(s(enc("delete", None, CTRL)), "\\u{1b}[3;5~");
        assert_eq!(s(enc("insert", None, Mods::none())), "\\u{1b}[2~");
        assert_eq!(s(enc("f1", None, Mods::none())), "\\u{1b}OP");
        assert_eq!(s(enc("f4", None, SHIFT)), "\\u{1b}[1;2S");
        assert_eq!(s(enc("f5", None, Mods::none())), "\\u{1b}[15~");
        assert_eq!(s(enc("f12", None, CTRL)), "\\u{1b}[24;5~");
    }

    #[test]
    fn paste_encodings() {
        assert_eq!(
            String::from_utf8(bracketed_paste("ls\nwhoami")).unwrap(),
            "\x1b[200~ls\nwhoami\x1b[201~"
        );
        let evil = bracketed_paste("a\x1b[201~rm -rf /");
        assert_eq!(
            String::from_utf8(evil).unwrap(),
            "\x1b[200~arm -rf /\x1b[201~",
            "cannot close the bracket early"
        );
        assert_eq!(plain_paste("a\r\nb\nc"), b"a\rb\rc");
    }

    #[test]
    fn mouse_sgr_reports() {
        let m = Mods::none();
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::Left,
                MouseAction::Press,
                m,
                0,
                0
            )),
            "\\u{1b}[<0;1;1M"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::Left,
                MouseAction::Release,
                m,
                9,
                4
            )),
            "\\u{1b}[<0;10;5m"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::Right,
                MouseAction::Press,
                CTRL,
                2,
                3
            )),
            "\\u{1b}[<18;3;4M"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::Left,
                MouseAction::Motion,
                m,
                1,
                1
            )),
            "\\u{1b}[<32;2;2M"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::None,
                MouseAction::Motion,
                m,
                1,
                1
            )),
            "\\u{1b}[<35;2;2M"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::WheelUp,
                MouseAction::Press,
                m,
                0,
                0
            )),
            "\\u{1b}[<64;1;1M"
        );
        assert_eq!(
            s(encode_mouse_sgr(
                MouseButton::WheelDown,
                MouseAction::Press,
                SHIFT,
                0,
                0
            )),
            "\\u{1b}[<69;1;1M"
        );
        assert!(encode_mouse_sgr(MouseButton::WheelUp, MouseAction::Release, m, 0, 0).is_none());
    }
}
