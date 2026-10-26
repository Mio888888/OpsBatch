pub const SECRET_PLACEHOLDER: &str = "***keychain***";
pub const MAX_EXECUTION_CONCURRENCY: u32 = 32;
pub const MAX_EXECUTION_OUTPUT_BYTES: usize = 512 * 1024;
pub const MAX_RAG_SEARCH_LIMIT: i64 = 50;
pub const DEFAULT_RAG_SEARCH_LIMIT: i64 = 10;
pub const MAX_SFTP_TREE_NODES: usize = 2_000;

pub fn clamp_execution_concurrency(value: u32) -> u32 {
    value.clamp(1, MAX_EXECUTION_CONCURRENCY)
}

pub fn clamp_rag_limit(value: Option<i64>) -> i64 {
    value
        .unwrap_or(DEFAULT_RAG_SEARCH_LIMIT)
        .clamp(1, MAX_RAG_SEARCH_LIMIT)
}

pub fn truncate_output_lossy(output: String) -> String {
    if output.len() <= MAX_EXECUTION_OUTPUT_BYTES {
        return output;
    }
    let keep = MAX_EXECUTION_OUTPUT_BYTES.saturating_sub(96);
    let start = output.len().saturating_sub(keep);
    let trimmed = match output.get(start..) {
        Some(value) => value.to_string(),
        None => output
            .char_indices()
            .find(|(idx, _)| *idx >= start)
            .map(|(idx, _)| output[idx..].to_string())
            .unwrap_or_default(),
    };
    format!(
        "[OpsBatch: output truncated to last {} bytes]\n{}",
        keep, trimmed
    )
}

pub fn shell_quote(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok("''".to_string());
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err("路径包含非法控制字符".to_string());
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

pub fn reject_suspicious_local_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path is empty".to_string());
    }
    if path.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err("path contains control characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        let quoted = shell_quote("/tmp/a'b; touch /tmp/pwned").unwrap();
        assert_eq!("'/tmp/a'\\''b; touch /tmp/pwned'", quoted);
    }

    #[test]
    fn shell_quote_rejects_control_characters() {
        assert!(shell_quote("/tmp/a\nwhoami").is_err());
    }

    #[test]
    fn concurrency_is_never_zero_or_unbounded() {
        assert_eq!(1, clamp_execution_concurrency(0));
        assert_eq!(MAX_EXECUTION_CONCURRENCY, clamp_execution_concurrency(500));
    }

    #[test]
    fn rag_limit_is_clamped() {
        assert_eq!(DEFAULT_RAG_SEARCH_LIMIT, clamp_rag_limit(None));
        assert_eq!(1, clamp_rag_limit(Some(-5)));
        assert_eq!(MAX_RAG_SEARCH_LIMIT, clamp_rag_limit(Some(999)));
    }
}
