/// `opsbatch-bg <cmd>` / `opsbatch bg <cmd>` typed in a terminal runs `<cmd>` detached from the terminal.
pub fn parse_opsbatch_background_command(command: &str) -> Option<String> {
    let trimmed = command.trim();
    let rest = trimmed
        .strip_prefix("opsbatch-bg")
        .or_else(|| {
            let after = trimmed.strip_prefix("opsbatch")?;
            // `opsbatch` + whitespace + `bg`
            let after_ws = after.trim_start();
            if after_ws.len() == after.len() {
                return None;
            }
            after_ws.strip_prefix("bg")
        })?;
    // The keyword must be followed by whitespace (not glued to more word characters).
    let body = rest.strip_prefix(|c: char| c.is_whitespace())?;
    let body = body.trim();
    (!body.is_empty()).then(|| body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opsbatch_background_terminal_commands() {
        assert_eq!(parse_opsbatch_background_command("opsbatch-bg ./scripts/worker.sh --port 8080").as_deref(), Some("./scripts/worker.sh --port 8080"));
        assert_eq!(parse_opsbatch_background_command("opsbatch bg npm run worker").as_deref(), Some("npm run worker"));
    }

    #[test]
    fn ignores_non_background_terminal_commands() {
        assert_eq!(parse_opsbatch_background_command("npm run dev &"), None);
        assert_eq!(parse_opsbatch_background_command("opsbatch-bg   "), None);
    }

    #[test]
    fn requires_a_word_boundary_and_multiline_bodies_survive() {
        assert_eq!(parse_opsbatch_background_command("opsbatch-bgx foo"), None);
        assert_eq!(parse_opsbatch_background_command("opsbatchbg foo"), None);
        assert_eq!(parse_opsbatch_background_command("opsbatch bgfoo"), None);
        assert_eq!(parse_opsbatch_background_command("  opsbatch   bg   a\nb ").as_deref(), Some("a\nb"));
    }
}
