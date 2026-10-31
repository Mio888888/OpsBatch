//! Input details of the RDP window that do not need a window: mouse button codes (side buttons included), the wheel
//! accumulator (a trackpad sends many small deltas, the server wants notches) and the key events of text that the
//! local input method composed.

use gpui_kit::{MouseButton, NavigationDirection};
use ops_rdp::RdpInputEvent;

/// The button as the RDP backend numbers it (the DOM numbering): the thumb buttons of a mouse are X1 (back) and
/// X2 (forward).
pub fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        MouseButton::Navigate(NavigationDirection::Back) => Some(3),
        MouseButton::Navigate(NavigationDirection::Forward) => Some(4),
    }
}

/// Pixels of scrolling that make one wheel notch (a mouse wheel click reports one 20 px line).
pub const PIXELS_PER_NOTCH: f32 = 20.0;
/// One notch in RDP wheel units.
pub const NOTCH: i16 = 120;
/// A single scroll event never sends more than this many notches (a fast flick is not a jump of a whole page).
const MAX_NOTCHES_PER_EVENT: i32 = 8;

/// Turns scroll deltas into wheel notches, keeping the remainder so slow trackpad scrolling still adds up.
#[derive(Debug, Default, Clone)]
pub struct WheelAccumulator {
    x: f32,
    y: f32,
}

impl WheelAccumulator {
    /// `dx` / `dy` are the pixels the content moves (GPUI's sign): content moving down means the wheel turned up
    /// (RDP positive); content moving right means the view scrolls left (RDP horizontal negative).
    /// Returns `(vertical, delta)` events, vertical ones first.
    pub fn push(&mut self, dx: f32, dy: f32) -> Vec<(bool, i16)> {
        let mut out = Vec::new();
        out.extend(Self::take(&mut self.y, dy, 1).into_iter().map(|d| (true, d)));
        out.extend(Self::take(&mut self.x, dx, -1).into_iter().map(|d| (false, d)));
        out
    }

    /// Forgets what is left over (the pointer left the picture, or the direction changed a while ago).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn take(acc: &mut f32, delta: f32, sign: i32) -> Option<i16> {
        if delta == 0.0 {
            return None;
        }
        // turning around drops what was left of the other direction
        if (*acc > 0.0) != (delta > 0.0) {
            *acc = 0.0;
        }
        *acc += delta;
        let notches = (*acc / PIXELS_PER_NOTCH).trunc() as i32;
        if notches == 0 {
            return None;
        }
        *acc -= notches as f32 * PIXELS_PER_NOTCH;
        let notches = notches.clamp(-MAX_NOTCHES_PER_EVENT, MAX_NOTCHES_PER_EVENT);
        Some((notches * sign * NOTCH as i32) as i16)
    }
}

/// The key events for text the input method committed: every character is typed as Unicode (the remote keyboard
/// layout does not matter), Enter and Tab as their keys; other control characters are dropped.
pub fn text_events(text: &str) -> Vec<RdpInputEvent> {
    let key = |code: u8, down: bool| RdpInputEvent::KeyScancode { code, extended: false, down };
    let mut out = Vec::new();
    for c in text.chars() {
        match c {
            '\n' | '\r' => out.extend([key(0x1c, true), key(0x1c, false)]),
            '\t' => out.extend([key(0x0f, true), key(0x0f, false)]),
            c if c.is_control() => {}
            c => out.extend([RdpInputEvent::Unicode { character: c.to_string(), down: true }, RdpInputEvent::Unicode { character: c.to_string(), down: false }]),
        }
    }
    out
}

/// UTF-16 length, the unit the platform input method counts in.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::{button_code, text_events, utf16_len, WheelAccumulator, NOTCH};
    use gpui_kit::{MouseButton, NavigationDirection};
    use ops_rdp::RdpInputEvent;

    #[test]
    fn side_buttons_are_x1_and_x2() {
        assert_eq!(button_code(MouseButton::Left), Some(0));
        assert_eq!(button_code(MouseButton::Middle), Some(1));
        assert_eq!(button_code(MouseButton::Right), Some(2));
        assert_eq!(button_code(MouseButton::Navigate(NavigationDirection::Back)), Some(3));
        assert_eq!(button_code(MouseButton::Navigate(NavigationDirection::Forward)), Some(4));
    }

    #[test]
    fn a_mouse_wheel_click_is_one_notch_and_a_trackpad_adds_up() {
        let mut w = WheelAccumulator::default();
        assert_eq!(w.push(0.0, 20.0), vec![(true, NOTCH)]);
        assert_eq!(w.push(0.0, -20.0), vec![(true, -NOTCH)]);
        // 3 px at a time: nothing until 20 px have piled up, then one notch
        let sent: Vec<_> = (0..7).flat_map(|_| w.push(0.0, 3.0)).collect();
        assert_eq!(sent, vec![(true, NOTCH)], "7 x 3 px = 21 px");
        // the remainder (1 px) carries on
        assert_eq!(w.push(0.0, 19.0), vec![(true, NOTCH)]);
    }

    #[test]
    fn horizontal_scrolling_has_the_opposite_sign_and_turning_around_drops_the_remainder() {
        let mut w = WheelAccumulator::default();
        // content moving right = the view scrolls left
        assert_eq!(w.push(20.0, 0.0), vec![(false, -NOTCH)]);
        assert_eq!(w.push(-20.0, 0.0), vec![(false, NOTCH)]);
        let mut w = WheelAccumulator::default();
        assert!(w.push(0.0, 15.0).is_empty());
        assert!(w.push(0.0, -10.0).is_empty(), "a reversal starts over instead of cancelling out");
        assert!(w.push(0.0, -9.0).is_empty());
        assert_eq!(w.push(0.0, -2.0), vec![(true, -NOTCH)]);
        w.reset();
        assert!(w.push(0.0, 19.0).is_empty());
    }

    #[test]
    fn a_fast_flick_is_capped_and_diagonals_send_both_axes() {
        let mut w = WheelAccumulator::default();
        assert_eq!(w.push(0.0, 2000.0), vec![(true, 8 * NOTCH)]);
        let mut w = WheelAccumulator::default();
        assert_eq!(w.push(40.0, 20.0), vec![(true, NOTCH), (false, -2 * NOTCH)]);
    }

    #[test]
    fn composed_text_is_typed_as_unicode() {
        let ev = text_events("你好");
        assert_eq!(ev.len(), 4);
        assert_eq!(ev[0], RdpInputEvent::Unicode { character: "你".into(), down: true });
        assert_eq!(ev[1], RdpInputEvent::Unicode { character: "你".into(), down: false });
        assert_eq!(ev[2], RdpInputEvent::Unicode { character: "好".into(), down: true });
        // characters beyond the BMP stay one event pair (the backend splits them into surrogates)
        assert_eq!(text_events("😀")[0], RdpInputEvent::Unicode { character: "😀".into(), down: true });
        // Enter and Tab are keys, other control characters are dropped
        assert_eq!(text_events("a\n"), vec![RdpInputEvent::Unicode { character: "a".into(), down: true }, RdpInputEvent::Unicode { character: "a".into(), down: false }, RdpInputEvent::KeyScancode { code: 0x1c, extended: false, down: true }, RdpInputEvent::KeyScancode { code: 0x1c, extended: false, down: false }]);
        assert_eq!(text_events("\t")[0], RdpInputEvent::KeyScancode { code: 0x0f, extended: false, down: true });
        assert!(text_events("\u{1b}\u{7f}").is_empty());
        assert!(text_events("").is_empty());
    }

    #[test]
    fn the_input_method_counts_in_utf16_units() {
        assert_eq!((utf16_len("ni"), utf16_len("你好"), utf16_len("😀")), (2, 2, 2));
    }
}
