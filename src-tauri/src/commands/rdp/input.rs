use ironrdp::input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};

use super::types::{RdpInputEvent, RdpMouseButton};

pub(super) fn input_operations(
    event: RdpInputEvent,
    width: u16,
    height: u16,
) -> Result<Vec<Operation>, String> {
    match event {
        RdpInputEvent::MouseMove { x, y } => Ok(vec![Operation::MouseMove(mouse_position(
            x, y, width, height,
        ))]),
        RdpInputEvent::MouseButton { x, y, button, down } => {
            let button = MouseButton::from_web_button(button)
                .ok_or_else(|| format!("不支持的鼠标按钮: {button}"))?;
            let mut operations = vec![Operation::MouseMove(mouse_position(x, y, width, height))];
            operations.push(if down {
                Operation::MouseButtonPressed(button)
            } else {
                Operation::MouseButtonReleased(button)
            });
            Ok(operations)
        }
        RdpInputEvent::Wheel {
            x,
            y,
            delta,
            vertical,
        } => Ok(vec![
            Operation::MouseMove(mouse_position(x, y, width, height)),
            Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: delta,
            }),
        ]),
        RdpInputEvent::KeyScancode {
            code,
            extended,
            down,
        } => {
            let scancode = Scancode::from_u8(extended, code);
            Ok(vec![if down {
                Operation::KeyPressed(scancode)
            } else {
                Operation::KeyReleased(scancode)
            }])
        }
        RdpInputEvent::Unicode { character, down } => {
            let ch = character
                .chars()
                .next()
                .ok_or_else(|| "Unicode 输入字符不能为空".to_string())?;
            Ok(vec![if down {
                Operation::UnicodeKeyPressed(ch)
            } else {
                Operation::UnicodeKeyReleased(ch)
            }])
        }
    }
}

pub(super) fn validate_input_event(event: &RdpInputEvent) -> Result<(), String> {
    match event {
        RdpInputEvent::MouseButton { button, .. } => mouse_button_from_web(*button).map(|_| ()),
        RdpInputEvent::Unicode { character, .. } if character.chars().next().is_none() => {
            Err("Unicode 输入字符不能为空".to_string())
        }
        _ => Ok(()),
    }
}

pub(super) fn mouse_button_from_web(button: u8) -> Result<RdpMouseButton, String> {
    match button {
        0 => Ok(RdpMouseButton::Left),
        1 => Ok(RdpMouseButton::Middle),
        2 => Ok(RdpMouseButton::Right),
        3 => Ok(RdpMouseButton::X1),
        4 => Ok(RdpMouseButton::X2),
        _ => Err(format!("不支持的鼠标按钮: {}", button)),
    }
}

fn mouse_position(x: u16, y: u16, width: u16, height: u16) -> MousePosition {
    MousePosition {
        x: x.min(width.saturating_sub(1)),
        y: y.min(height.saturating_sub(1)),
    }
}
