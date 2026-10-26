//! Port forwarding panel (M5-10).

pub mod panel;

pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        "0B".into()
    } else if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{}K", (bytes as f64 / 1024.0).round() as u64)
    } else {
        format!("{bytes}B")
    }
}

/// Whether the form can be submitted: a local address, plus a remote one unless it is a dynamic (SOCKS) forward.
pub fn form_is_complete(dynamic: bool, local: &str, remote: &str) -> bool {
    !local.trim().is_empty() && (dynamic || !remote.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_format_like_the_original() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(900), "900B");
        assert_eq!(format_bytes(2048), "2K");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0M");
    }

    #[test]
    fn dynamic_forwards_need_no_remote_address() {
        assert!(form_is_complete(true, "127.0.0.1:1080", ""));
        assert!(!form_is_complete(false, "127.0.0.1:8080", ""));
        assert!(form_is_complete(false, "127.0.0.1:8080", "10.0.0.1:80"));
        assert!(!form_is_complete(true, "  ", "x"));
    }
}
