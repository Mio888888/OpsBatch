//! File classification and display formatting for the SFTP panel (port of the helpers in SftpPanel.tsx).

use ops_ssh::sftp::FileEntry;

/// Text after the last dot (the whole name when there is no dot, so `Makefile` -> `makefile`), lower-cased.
fn ext_of(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_lowercase()
}

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "json", "js", "ts", "tsx", "jsx", "py", "rs", "go", "java", "c", "h", "cpp",
    "hpp", "html", "css", "scss", "less", "yaml", "yml", "toml", "ini", "cfg", "conf", "sh",
    "bash", "zsh", "fish", "sql", "xml", "csv", "log", "env",
];
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "svg", "webp", "ico", "bmp"];
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "aac", "ogg"];
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "ogg"];
const FONT_EXTS: &[&str] = &["ttf", "otf", "woff", "woff2"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Text,
    Image,
    Audio,
    Video,
    Font,
    Pdf,
    Binary,
}

/// What the preview dialog shows for a file name.
pub fn preview_kind(name: &str) -> PreviewKind {
    let ext = ext_of(name);
    let ext = ext.as_str();
    if TEXT_EXTS.contains(&ext) {
        PreviewKind::Text
    } else if IMAGE_EXTS.contains(&ext) {
        PreviewKind::Image
    } else if AUDIO_EXTS.contains(&ext) {
        PreviewKind::Audio
    } else if VIDEO_EXTS.contains(&ext) {
        PreviewKind::Video
    } else if FONT_EXTS.contains(&ext) {
        PreviewKind::Font
    } else if ext == "pdf" {
        PreviewKind::Pdf
    } else {
        PreviewKind::Binary
    }
}

impl PreviewKind {
    /// Kinds the app cannot show itself (no video decoder / PDF renderer / font viewer): offered as "open with the
    /// system app" instead of a hex dump.
    pub fn opens_externally(self) -> bool {
        matches!(
            self,
            PreviewKind::Video | PreviewKind::Pdf | PreviewKind::Font
        )
    }

    /// How much of the file the preview reads.
    pub fn read_limit(self) -> u64 {
        match self {
            PreviewKind::Audio => 64 * 1024 * 1024,
            _ => 2 * 1024 * 1024,
        }
    }
}

/// Entries that get a "Preview" menu item: the preview set plus a few extension-less text names.
pub fn is_previewable(entry: &FileEntry) -> bool {
    if entry.is_dir {
        return false;
    }
    let ext = ext_of(&entry.name);
    let extra = ["gitignore", "dockerfile", "makefile"];
    preview_kind(&entry.name) != PreviewKind::Binary || extra.contains(&ext.as_str())
}

pub fn is_archive(entry: &FileEntry) -> bool {
    if entry.is_dir {
        return false;
    }
    let n = entry.name.to_lowercase();
    [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tbz2", ".txz", ".tar", ".zip", ".7z", ".rar",
    ]
    .iter()
    .any(|s| n.ends_with(s))
}

pub fn file_icon(entry: &FileEntry) -> &'static str {
    if entry.is_dir {
        return "📁";
    }
    match ext_of(&entry.name).as_str() {
        "md" => "📝",
        "json" => "📋",
        "js" | "ts" | "tsx" | "jsx" => "📜",
        "py" => "🐍",
        "rs" => "🦀",
        "go" => "🔵",
        "java" => "☕",
        "c" | "h" | "cpp" => "🔧",
        "html" => "🌐",
        "css" | "scss" => "🎨",
        "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "ico" => "🖼️",
        "mp3" | "wav" | "flac" | "aac" | "ogg" => "🎵",
        "mp4" | "avi" | "mkv" | "mov" | "webm" => "🎬",
        "zip" | "gz" | "tar" | "bz2" | "xz" | "7z" | "rar" => "📦",
        "pdf" => "📕",
        "ttf" | "otf" | "woff" | "woff2" => "🔤",
        _ => "📄",
    }
}

pub fn format_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if bytes == 0 {
        "-".into()
    } else if b >= K * K * K {
        format!("{:.1}G", b / K / K / K)
    } else if b >= K * K {
        format!("{:.1}M", b / K / K)
    } else if b >= K {
        format!("{:.1}K", b / K)
    } else {
        format!("{bytes}B")
    }
}

/// `bytes_per_ms` as the backend reports it (size / duration).
pub fn format_speed(bytes_per_ms: f64) -> String {
    if bytes_per_ms == 0.0 {
        return String::new();
    }
    let bps = bytes_per_ms * 1000.0;
    if bps >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bps / 1024.0 / 1024.0)
    } else if bps >= 1024.0 {
        format!("{:.0} KB/s", bps / 1024.0)
    } else {
        format!("{bps:.0} B/s")
    }
}

pub fn format_duration(ms: Option<u64>) -> String {
    match ms {
        None => String::new(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) => format!("{:.1}s", ms as f64 / 1000.0),
    }
}

pub fn hex_dump(bytes: &[u8], max_lines: usize) -> String {
    let mut lines = Vec::new();
    for (i, chunk) in bytes.chunks(16).take(max_lines).enumerate() {
        let hex = chunk
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let ascii: String = chunk
            .iter()
            .map(|b| {
                if (32..=126).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        lines.push(format!("{:08x}  {:<47}  |{}|", i * 16, hex, ascii));
    }
    lines.join("\n")
}

pub fn mime_for_image(name: &str) -> &'static str {
    match ext_of(name).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: format!("/x/{name}"),
            is_dir: dir,
            is_symlink: false,
            size: 0,
            modified: None,
            permissions: None,
        }
    }

    #[test]
    fn classification_matches_the_original_sets() {
        assert_eq!(preview_kind("a.JSON"), PreviewKind::Text);
        assert_eq!(preview_kind("a.png"), PreviewKind::Image);
        assert_eq!(preview_kind("a.mp3"), PreviewKind::Audio);
        assert_eq!(preview_kind("a.mp4"), PreviewKind::Video);
        assert_eq!(preview_kind("a.woff2"), PreviewKind::Font);
        assert_eq!(preview_kind("a.pdf"), PreviewKind::Pdf);
        assert_eq!(preview_kind("a.bin"), PreviewKind::Binary);
        assert!(
            PreviewKind::Video.opens_externally()
                && PreviewKind::Pdf.opens_externally()
                && PreviewKind::Font.opens_externally()
        );
        assert!(!PreviewKind::Audio.opens_externally() && !PreviewKind::Text.opens_externally());
        assert!(PreviewKind::Audio.read_limit() > PreviewKind::Text.read_limit());
        assert_eq!(preview_kind("noext"), PreviewKind::Binary);
        assert!(is_previewable(&entry("Makefile", false)));
        assert!(is_previewable(&entry(".gitignore", false)));
        assert!(is_previewable(&entry("a.rs", false)));
        assert!(!is_previewable(&entry("a.rs", true)));
        assert!(!is_previewable(&entry("a.bin", false)));
    }

    #[test]
    fn archives_by_suffix() {
        for n in [
            "a.tar.gz", "a.TGZ", "a.zip", "a.7z", "a.rar", "a.tar.xz", "a.tar",
        ] {
            assert!(is_archive(&entry(n, false)), "{n}");
        }
        assert!(!is_archive(&entry("a.gz", false)));
        assert!(!is_archive(&entry("dir.zip", true)));
    }

    #[test]
    fn size_speed_duration_formats() {
        assert_eq!(format_size(0), "-");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0M");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0G");
        assert_eq!(format_speed(0.0), "");
        assert_eq!(format_speed(2.0), "2 KB/s");
        assert_eq!(format_speed(2048.0), "2.0 MB/s");
        assert_eq!(format_speed(0.1), "100 B/s");
        assert_eq!(format_duration(None), "");
        assert_eq!(format_duration(Some(250)), "250ms");
        assert_eq!(format_duration(Some(1500)), "1.5s");
    }

    #[test]
    fn hex_dump_layout() {
        let d = hex_dump(b"Hello, world!\x00\x01\x02ABCDEFG", 16);
        let lines: Vec<&str> = d.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with(
            "00000000  48 65 6c 6c 6f 2c 20 77 6f 72 6c 64 21 00 01 02  |Hello, world!...|"
        ));
        assert!(lines[1].starts_with("00000010  41 42 43 44 45 46 47"));
        assert_eq!(
            hex_dump(&[0u8; 400], 16).lines().count(),
            16,
            "capped at 16 lines"
        );
    }
}
