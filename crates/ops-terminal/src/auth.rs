use regex::Regex;
use std::sync::OnceLock;

/// Does an SSH connect error mean "bad credentials" (so the UI offers "edit auth and retry")?
pub fn is_ssh_authentication_failure(message: Option<&str>) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)认证被拒绝|authentication (?:failed|failure|refused)|permission denied")
            .unwrap()
    });
    message.is_some_and(|m| re.is_match(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ssh_authentication_failures_in_chinese_and_english_messages() {
        assert!(is_ssh_authentication_failure(Some("连接失败: 认证被拒绝")));
        assert!(is_ssh_authentication_failure(Some(
            "Connection failed: Authentication failed"
        )));
        assert!(is_ssh_authentication_failure(Some(
            "Permission denied (publickey,password)"
        )));
    }

    #[test]
    fn does_not_treat_ordinary_connection_failures_as_authentication_failures() {
        assert!(!is_ssh_authentication_failure(Some(
            "连接失败: connection refused"
        )));
        assert!(!is_ssh_authentication_failure(Some("连接超时")));
        assert!(!is_ssh_authentication_failure(None));
    }
}
