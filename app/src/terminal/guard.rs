//! What happens to text the user types before it reaches the shell: `opsbatch bg <cmd>` interception and the
//! dangerous-command confirmation (press Enter twice). Port of the `onData` handler in TerminalView.tsx.

use ops_terminal::background::parse_opsbatch_background_command;
use ops_terminal::danger::{check_dangerous_command, default_compiled_danger_rules, CompiledDangerRule};
use ops_terminal::input_line::process_terminal_input;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    /// Send the data to the shell unchanged.
    Forward,
    /// Run the command detached from the terminal (the line typed so far is cleared with Ctrl+U first).
    Background(String),
    /// Nothing is sent; the user must press Enter again to confirm. Carries the matched rule names.
    Blocked(Vec<String>),
}

pub struct InputGuard {
    rules: Vec<CompiledDangerRule>,
    line: String,
    pending_confirm: Option<String>,
}

impl Default for InputGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl InputGuard {
    /// Starts with the built-in rules so a terminal is protected before the stored rules are loaded.
    pub fn new() -> Self {
        Self { rules: default_compiled_danger_rules(), line: String::new(), pending_confirm: None }
    }

    pub fn set_rules(&mut self, rules: Vec<CompiledDangerRule>) {
        self.rules = rules;
    }

    /// `cursor_line` yields the text of the terminal row under the cursor (used when the local line buffer is
    /// empty, e.g. after history recall, and as the identity of a pending confirmation).
    pub fn process(&mut self, data: &str, cursor_line: impl FnOnce() -> String) -> GuardAction {
        let result = process_terminal_input(&self.line, data);
        self.line = result.buffer;

        if !data.contains(['\r', '\n']) {
            self.pending_confirm = None;
            return GuardAction::Forward;
        }

        let line_text = cursor_line().trim().to_string();
        if self.pending_confirm.as_deref() == Some(line_text.as_str()) {
            self.pending_confirm = None;
            return GuardAction::Forward;
        }

        let command = result.command.unwrap_or_else(|| line_text.clone());
        if command.is_empty() {
            return GuardAction::Forward;
        }
        if let Some(bg) = parse_opsbatch_background_command(&command) {
            return GuardAction::Background(bg);
        }
        let matched = check_dangerous_command(&self.rules, &command);
        if !matched.is_empty() {
            self.pending_confirm = Some(line_text);
            return GuardAction::Blocked(matched);
        }
        GuardAction::Forward
    }

    /// The tracked line is stale after anything that rewrites the shell's line behind our back.
    pub fn reset_line(&mut self) {
        self.line.clear();
        self.pending_confirm = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(g: &mut InputGuard, text: &str, screen: &str) {
        for c in text.chars() {
            assert_eq!(g.process(&c.to_string(), || screen.to_string()), GuardAction::Forward);
        }
    }

    #[test]
    fn ordinary_commands_pass_through() {
        let mut g = InputGuard::new();
        type_text(&mut g, "ls -la", "$ ls -la");
        assert_eq!(g.process("\r", || "$ ls -la".into()), GuardAction::Forward);
    }

    #[test]
    fn dangerous_command_needs_a_second_enter() {
        let mut g = InputGuard::new();
        type_text(&mut g, "rm -rf /", "$ rm -rf /");
        let first = g.process("\r", || "$ rm -rf /".into());
        assert!(matches!(first, GuardAction::Blocked(ref names) if !names.is_empty()), "{first:?}");
        // The shell never saw the first Enter; the second one on the same line is let through.
        assert_eq!(g.process("\r", || "$ rm -rf /".into()), GuardAction::Forward);
        // ... and only once.
        type_text(&mut g, "rm -rf /", "$ rm -rf /");
        assert!(matches!(g.process("\r", || "$ rm -rf /".into()), GuardAction::Blocked(_)));
    }

    #[test]
    fn editing_the_line_cancels_the_confirmation() {
        let mut g = InputGuard::new();
        type_text(&mut g, "rm -rf /", "$ rm -rf /");
        assert!(matches!(g.process("\r", || "$ rm -rf /".into()), GuardAction::Blocked(_)));
        // User edits (backspace) then submits a harmless line.
        assert_eq!(g.process("\x7f", || "$ rm -rf ".into()), GuardAction::Forward);
        assert_eq!(g.process("\r", || "$ rm -rf".into()), GuardAction::Forward);
    }

    #[test]
    fn history_recall_is_checked_through_the_screen_line() {
        let mut g = InputGuard::new();
        // Arrow keys do not touch the local buffer, so the command only exists on screen.
        assert_eq!(g.process("\x1b[A", || "$ rm -rf /".into()), GuardAction::Forward);
        assert!(matches!(g.process("\r", || "$ rm -rf /".into()), GuardAction::Blocked(_)));
    }

    #[test]
    fn background_command_is_intercepted() {
        let mut g = InputGuard::new();
        type_text(&mut g, "opsbatch bg sleep 100", "$ opsbatch bg sleep 100");
        assert_eq!(g.process("\r", || "$ opsbatch bg sleep 100".into()), GuardAction::Background("sleep 100".into()));
    }

    #[test]
    fn pasted_multi_char_chunks_are_tracked() {
        let mut g = InputGuard::new();
        assert_eq!(g.process("rm -rf /tmp/x", || "$ rm -rf /tmp/x".into()), GuardAction::Forward);
        assert!(matches!(g.process("\r", || "$ rm -rf /tmp/x".into()), GuardAction::Blocked(_)));
    }

    #[test]
    fn empty_enter_is_forwarded() {
        let mut g = InputGuard::new();
        assert_eq!(g.process("\r", || "$".into()), GuardAction::Forward);
    }
}
