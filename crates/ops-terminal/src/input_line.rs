//! Local tracking of what the user typed on the current line, so the danger guard sees the full command at the
//! moment Enter is pressed (no dependence on remote echo or write-queue flushing).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputResult {
    pub buffer: String,
    /// `Some(cmd)` when a newline was seen: the submitted command (trimmed, non-empty).
    pub command: Option<String>,
}

pub fn process_terminal_input(buffer: &str, data: &str) -> InputResult {
    if let Some(idx) = data.find(['\r', '\n']) {
        let command = format!("{buffer}{}", &data[..idx]);
        let command = command.trim();
        return InputResult {
            buffer: String::new(),
            command: (!command.is_empty()).then(|| command.to_string()),
        };
    }
    // Backspace / DEL
    if data == "\x7f" || data == "\x08" {
        let mut b = buffer.to_string();
        b.pop();
        return InputResult {
            buffer: b,
            command: None,
        };
    }
    // Ctrl+C cancels, Ctrl+U clears the line
    if data == "\x03" || data == "\x15" {
        return InputResult {
            buffer: String::new(),
            command: None,
        };
    }
    let first = data.chars().next();
    // Escape sequences (arrows, function keys, ...) are ignored.
    if first == Some('\x1b') {
        return InputResult {
            buffer: buffer.to_string(),
            command: None,
        };
    }
    // Printable characters
    if first.is_some_and(|c| c as u32 >= 32) {
        return InputResult {
            buffer: format!("{buffer}{data}"),
            command: None,
        };
    }
    InputResult {
        buffer: buffer.to_string(),
        command: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_a_command_and_submits_on_enter() {
        let mut buf = String::new();
        for ch in ["l", "s", " ", "-", "l"] {
            let r = process_terminal_input(&buf, ch);
            assert_eq!(r.command, None);
            buf = r.buffer;
        }
        let r = process_terminal_input(&buf, "\r");
        assert_eq!(r.command.as_deref(), Some("ls -l"));
        assert_eq!(r.buffer, "");
    }

    #[test]
    fn backspace_ctrl_c_ctrl_u_and_escape_sequences() {
        assert_eq!(process_terminal_input("abc", "\x7f").buffer, "ab");
        assert_eq!(process_terminal_input("", "\x7f").buffer, "");
        assert_eq!(process_terminal_input("abc", "\x03").buffer, "");
        assert_eq!(process_terminal_input("abc", "\x15").buffer, "");
        assert_eq!(process_terminal_input("abc", "\x1b[A").buffer, "abc");
    }

    #[test]
    fn pasted_multi_character_chunks_and_inline_newlines() {
        assert_eq!(
            process_terminal_input("echo ", "hello 世界").buffer,
            "echo hello 世界"
        );
        let r = process_terminal_input("rm -rf ", "/\nmore");
        assert_eq!(r.command.as_deref(), Some("rm -rf /"));
    }

    #[test]
    fn empty_enter_yields_no_command() {
        assert_eq!(process_terminal_input("   ", "\r").command, None);
    }
}
