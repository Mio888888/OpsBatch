//! ANSI SGR colours for command output (port of `utils/ansi.tsx`): the same palette, everything else stripped.

use std::ops::Range;

/// Colours by SGR parameter string, identical to the Tauri build's table.
pub fn sgr_color(code: &str) -> Option<u32> {
    Some(match code {
        "30" | "1;30" => 0x4e4e4e,
        "31" | "1;31" => 0xff6b6b,
        "32" | "1;32" => 0x51cf66,
        "33" | "1;33" => 0xfcc419,
        "34" | "1;34" => 0x339af0,
        "35" | "1;35" => 0xcc5de8,
        "36" | "1;36" => 0x22b8cf,
        "37" => 0xdee2e6,
        "1;37" => 0xffffff,
        "90" => 0x636363,
        "91" => 0xff8787,
        "92" => 0x69db7c,
        "93" => 0xffe066,
        "94" => 0x74c0fc,
        "95" => 0xe599f7,
        "96" => 0x66d9e8,
        "97" => 0xffffff,
        _ => return None,
    })
}

pub struct Ansi {
    pub text: String,
    /// Byte ranges of `text` with their RGB colour.
    pub spans: Vec<(Range<usize>, u32)>,
}

pub fn parse(input: &str) -> Ansi {
    let b = input.as_bytes();
    let mut text = String::with_capacity(input.len());
    let mut spans: Vec<(Range<usize>, u32)> = Vec::new();
    let mut color: Option<u32> = None;
    let mut span_start = 0usize;
    let mut i = 0;

    let close =
        |text: &String, spans: &mut Vec<(Range<usize>, u32)>, color: Option<u32>, start: usize| {
            if let Some(c) = color {
                if text.len() > start {
                    spans.push((start..text.len(), c));
                }
            }
        };

    while i < b.len() {
        match b[i] {
            0x1b if i + 1 < b.len() && b[i + 1] == b'[' => {
                let mut j = i + 2;
                while j < b.len() && (0x30..=0x3f).contains(&b[j]) {
                    j += 1;
                }
                while j < b.len() && (0x20..=0x2f).contains(&b[j]) {
                    j += 1;
                }
                if j >= b.len() {
                    break; // truncated sequence at the end
                }
                if b[j] == b'm' {
                    let params = &input[i + 2..j];
                    close(&text, &mut spans, color, span_start);
                    color = if params.is_empty() || params == "0" {
                        None
                    } else {
                        sgr_color(params).or(color)
                    };
                    span_start = text.len();
                }
                i = j + 1;
            }
            0x1b if i + 1 < b.len() && b[i + 1] == b']' => {
                // OSC ... BEL or ESC \
                let mut j = i + 2;
                while j < b.len()
                    && b[j] != 0x07
                    && !(b[j] == 0x1b && j + 1 < b.len() && b[j + 1] == b'\\')
                {
                    j += 1;
                }
                i = if j < b.len() && b[j] == 0x07 {
                    j + 1
                } else {
                    j + 2
                };
            }
            0x1b => i += 1,
            b'\r' => {
                // CRLF -> LF; a bare CR (progress redraws) is dropped.
                i += 1;
            }
            _ => {
                let ch_len = input[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                text.push_str(&input[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    close(&text, &mut spans, color, span_start);
    Ansi { text, spans }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_wrap_the_text_until_reset() {
        let a = parse("ok \x1b[32mgreen\x1b[0m tail");
        assert_eq!(a.text, "ok green tail");
        assert_eq!(a.spans, vec![(3..8, 0x51cf66)]);
    }

    #[test]
    fn colour_changes_and_unterminated_colour() {
        let a = parse("\x1b[31mred\x1b[34mblue");
        assert_eq!(a.text, "redblue");
        assert_eq!(a.spans, vec![(0..3, 0xff6b6b), (3..7, 0x339af0)]);
    }

    #[test]
    fn unknown_sgr_and_other_escapes_disappear() {
        let a = parse("a\x1b[1mb\x1b[2Kc\x1b[10;5Hd\x1b]0;title\x07e");
        assert_eq!(a.text, "abcde");
        assert!(a.spans.is_empty());
    }

    #[test]
    fn carriage_returns_are_normalised_and_unicode_survives() {
        let a = parse("你好\r\n世界\x1b[36m!\x1b[m");
        assert_eq!(a.text, "你好\n世界!");
        assert_eq!(a.spans, vec![(13..14, 0x22b8cf)]);
    }

    #[test]
    fn truncated_sequences_do_not_panic() {
        for s in ["\x1b", "\x1b[", "\x1b[3", "x\x1b[31", "\x1b]0;unfinished"] {
            let _ = parse(s);
        }
    }
}
