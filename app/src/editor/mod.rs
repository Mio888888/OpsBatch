//! Remote file editor window (M5-09): file mode (one file) and directory mode (file tree + switching files).

pub mod window;

/// Editor grammar for a file name (same table as `guessLanguage` in EditorWindow.tsx; unknown -> shell).
pub fn guess_language(file_name: Option<&str>) -> &'static str {
    let Some(name) = file_name else { return "bash" };
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "py" => "python",
        "ps1" | "psm1" | "psd1" => "powershell",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "json" => "json",
        "html" => "html",
        "css" | "scss" => "css",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        "sql" => "sql",
        "xml" => "xml",
        "csv" => "text",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" => "cpp",
        _ => "bash",
    }
}

/// Last path component, ignoring a trailing slash.
pub fn base_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

/// Stable window label per (host, mode, path) so the same target reuses its window.
pub fn window_label(host_id: &str, dir: bool, path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (host_id, dir, path).hash(&mut h);
    format!("editor-{:x}", h.finish())
}

/// Decode file bytes for the editor (invalid UTF-8 becomes replacement characters, like `TextDecoder`).
pub fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_table() {
        assert_eq!(guess_language(Some("deploy.SH")), "bash");
        assert_eq!(guess_language(Some("a.tsx")), "typescript");
        assert_eq!(guess_language(Some("a.yml")), "yaml");
        assert_eq!(guess_language(Some("main.rs")), "rust");
        assert_eq!(guess_language(Some("data.csv")), "text");
        assert_eq!(guess_language(Some("Makefile")), "bash");
        assert_eq!(guess_language(None), "bash");
    }

    #[test]
    fn names_and_labels() {
        assert_eq!(base_name("/etc/nginx/"), "nginx");
        assert_eq!(base_name("/etc/hosts"), "hosts");
        assert_eq!(base_name("/"), "/");
        assert_eq!(window_label("h1", false, "/a"), window_label("h1", false, "/a"));
        assert_ne!(window_label("h1", false, "/a"), window_label("h1", true, "/a"));
        assert_ne!(window_label("h1", false, "/a"), window_label("h2", false, "/a"));
    }
}
