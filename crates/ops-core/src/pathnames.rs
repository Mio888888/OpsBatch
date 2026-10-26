//! Path helpers that treat both `/` and `\` as separators (local Windows paths and remote POSIX paths share them).

pub fn basename_from_path(path: &str, fallback: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    if last.is_empty() { fallback.to_string() } else { last.to_string() }
}

pub fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }
    let sep = if base.contains('\\') && !base.contains('/') { '\\' } else { '/' };
    let trimmed = base.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return format!("{sep}{name}");
    }
    format!("{trimmed}{sep}{name}")
}

fn is_drive_root(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    (b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')).then(|| &s[..2])
}

fn is_drive_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || is_drive_root(s).is_some()
}

pub fn dirname_from_path(path: &str) -> String {
    if let Some(drive) = is_drive_root(path) {
        return format!("{drive}\\");
    }
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "/".into();
    }
    let sep = if trimmed.contains('\\') && !trimmed.contains('/') { '\\' } else { '/' };
    let Some(index) = trimmed.rfind(['/', '\\']) else { return "/".into() };
    if index == 0 {
        return "/".into();
    }
    if is_drive_prefix(&trimmed[..=index]) {
        return format!("{}{sep}", &trimmed[..index]);
    }
    trimmed[..index].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_handles_posix_windows_and_trailing_separators() {
        assert_eq!(basename_from_path("/Users/me/Desktop/report.csv", "file"), "report.csv");
        assert_eq!(basename_from_path(r"C:\Users\me\Desktop\report.csv", "file"), "report.csv");
        assert_eq!(basename_from_path("/Users/me/Desktop/report.csv/", "file"), "report.csv");
        assert_eq!(basename_from_path("/", "root"), "root");
        assert_eq!(basename_from_path("", "file"), "file");
    }

    #[test]
    fn join_keeps_the_separator_style_of_the_base() {
        assert_eq!(join_path(r"C:\Users\me\Downloads\", "report.csv"), r"C:\Users\me\Downloads\report.csv");
        assert_eq!(join_path("/Users/me/Downloads/", "report.csv"), "/Users/me/Downloads/report.csv");
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("", "a"), "a");
    }

    #[test]
    fn dirname_returns_parents_including_windows_roots() {
        assert_eq!(dirname_from_path(r"C:\Users\me\Downloads"), r"C:\Users\me");
        assert_eq!(dirname_from_path(r"C:\Users\"), r"C:\");
        assert_eq!(dirname_from_path(r"C:\"), r"C:\");
        assert_eq!(dirname_from_path("/var/log/syslog"), "/var/log");
        assert_eq!(dirname_from_path("/var"), "/");
        assert_eq!(dirname_from_path("/"), "/");
        assert_eq!(dirname_from_path("relative"), "/");
    }
}
