use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const RELEASES_PAGE_URL: &str = "https://github.com/Mio888888/OpsBatch/releases/latest";
const RELEASES_ATOM_URL: &str = "https://github.com/Mio888888/OpsBatch/releases.atom";
const RELEASES_DOWNLOAD_BASE: &str = "https://github.com/Mio888888/OpsBatch/releases/download";

/// Where releases are looked up and downloaded from. GitHub by default; `OPSBATCH_UPDATE_BASE=http://host:port`
/// points all three URLs at a mirror/mock with the same layout (`/releases/latest`, `/releases.atom`,
/// `/releases/download/<tag>/<asset>`).
#[derive(Debug, Clone)]
pub struct UpdateSource {
    pub releases_page_url: String,
    pub releases_atom_url: String,
    pub download_base: String,
    /// minisign public key (base64 of the key file, as in the Tauri updater config); when set, every installer must
    /// carry a valid `<asset>.sig` next to it.
    pub pubkey: Option<String>,
}

/// The updater key: injected at build time like the original's `__TAURI_UPDATER_PUBKEY__` placeholder, or given at
/// run time through `OPSBATCH_UPDATER_PUBKEY` (tests, mirrors with their own key).
fn configured_pubkey() -> Option<String> {
    let value = std::env::var("OPSBATCH_UPDATER_PUBKEY").ok().or_else(|| option_env!("OPSBATCH_UPDATER_PUBKEY").map(str::to_string))?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("base64 无效".into()),
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// `minisign` public key / signature as the Tauri tooling writes them: base64 of the usual two-line text files.
fn decode_minisign_text(b64: &str) -> Result<String, String> {
    String::from_utf8(base64_decode(b64)?).map_err(|_| "minisign 文本无效".to_string())
}

impl UpdateSource {
    pub fn github() -> Self {
        Self { releases_page_url: RELEASES_PAGE_URL.into(), releases_atom_url: RELEASES_ATOM_URL.into(), download_base: RELEASES_DOWNLOAD_BASE.into(), pubkey: configured_pubkey() }
    }

    pub fn with_base(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self { releases_page_url: format!("{base}/releases/latest"), releases_atom_url: format!("{base}/releases.atom"), download_base: format!("{base}/releases/download"), pubkey: configured_pubkey() }
    }

    pub fn from_env() -> Self {
        match std::env::var("OPSBATCH_UPDATE_BASE") {
            Ok(base) if !base.trim().is_empty() => Self::with_base(&base),
            _ => Self::github(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_title: Option<String>,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    pub release_url: String,
}

#[derive(Debug, Clone)]
struct ReleaseNotes {
    title: String,
    body: String,
    published_at: Option<String>,
}
/// `current_version` is the running app's version (the app crate passes `env!("CARGO_PKG_VERSION")`).
pub async fn check_app_update(current_version: &str) -> Result<AppUpdateInfo, String> {
    check_app_update_from(&UpdateSource::from_env(), current_version).await
}

pub async fn check_app_update_from(source: &UpdateSource, current_version: &str) -> Result<AppUpdateInfo, String> {
    let current_version = current_version.to_string();
    let response = reqwest::Client::new()
        .get(&source.releases_page_url)
        .header(reqwest::header::USER_AGENT, "OpsBatch update checker")
        .send()
        .await
        .map_err(|e| format!("检查更新失败: {}", e))?
        .error_for_status()
        .map_err(|e| format!("检查更新失败: {}", e))?;
    let release_url = response.url().to_string();
    let latest_version = parse_release_tag_from_url(&release_url)
        .ok_or_else(|| format!("解析更新信息失败: {}", release_url))?;
    let release_notes = fetch_release_notes(source, &latest_version).await.ok();

    let has_update = is_remote_version_newer(&latest_version, &current_version);
    Ok(AppUpdateInfo {
        has_update,
        current_version,
        latest_version: Some(latest_version),
        release_title: release_notes.as_ref().map(|notes| notes.title.clone()),
        release_notes: release_notes.as_ref().map(|notes| notes.body.clone()),
        published_at: release_notes.and_then(|notes| notes.published_at),
        release_url,
    })
}

async fn fetch_release_notes(source: &UpdateSource, tag: &str) -> Result<ReleaseNotes, String> {
    let feed = reqwest::Client::new()
        .get(&source.releases_atom_url)
        .header(reqwest::header::USER_AGENT, "OpsBatch update checker")
        .send()
        .await
        .map_err(|e| format!("获取更新内容失败: {}", e))?
        .error_for_status()
        .map_err(|e| format!("获取更新内容失败: {}", e))?
        .text()
        .await
        .map_err(|e| format!("读取更新内容失败: {}", e))?;

    parse_release_notes_from_feed(&feed, tag)
        .ok_or_else(|| format!("未找到版本 {} 的更新内容", tag))
}

/// Name of the installer a release publishes for this platform (`version` may carry a leading `v`).
/// The release workflow in `.github/workflows/release.yml` uses the same names.
pub fn installer_asset_name(version: &str, os: &str, arch: &str) -> Option<String> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let arch = match arch {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        _ => return None,
    };
    match os {
        "macos" => Some(format!("OpsBatch-{version}-macos-{arch}.dmg")),
        "windows" => Some(format!("OpsBatch-{version}-windows-{arch}-setup.exe")),
        "linux" => Some(format!("OpsBatch-{version}-linux-{arch}.AppImage")),
        _ => None,
    }
}

/// Hash of `name` in a `sha256sum`-style listing (`<hex>  <file>` or `<hex> *<file>`).
pub fn parse_sha256sums(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let mut parts = line.trim().splitn(2, char::is_whitespace);
        let (hash, file) = (parts.next()?, parts.next()?.trim().trim_start_matches('*'));
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download the installer of `tag` for `os`/`arch` into `dest_dir`, reporting `(downloaded, total)` while it
/// streams, and verify it against the release's `SHA256SUMS`. A missing listing, a missing entry or a mismatch
/// is an error and the partial file is removed — an unverified installer is never handed to the caller.
pub async fn download_update(
    source: &UpdateSource,
    tag: &str,
    os: &str,
    arch: &str,
    dest_dir: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let name = installer_asset_name(tag, os, arch).ok_or_else(|| format!("当前平台暂无可安装的更新包 ({os}/{arch})"))?;
    let client = reqwest::Client::new();
    let get = |url: String| {
        client.get(url).header(reqwest::header::USER_AGENT, "OpsBatch updater").send()
    };
    let base = source.download_base.trim_end_matches('/');
    let sums = get(format!("{base}/{tag}/SHA256SUMS"))
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("获取校验文件失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取校验文件失败: {e}"))?;
    let expected = parse_sha256sums(&sums, &name).ok_or_else(|| format!("校验文件中没有 {name}"))?;
    // with an updater key configured the installer's minisign signature is mandatory (the Tauri updater's rule)
    let signature = match &source.pubkey {
        Some(key) => {
            let public = minisign_verify::PublicKey::decode(&decode_minisign_text(key)?).map_err(|e| format!("更新公钥无效: {e}"))?;
            let sig_b64 = get(format!("{base}/{tag}/{name}.sig"))
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("获取签名文件失败: {e}"))?
                .text()
                .await
                .map_err(|e| format!("读取签名文件失败: {e}"))?;
            let sig = minisign_verify::Signature::decode(&decode_minisign_text(&sig_b64)?).map_err(|e| format!("签名文件无效: {e}"))?;
            Some((public, sig))
        }
        None => None,
    };

    std::fs::create_dir_all(dest_dir).map_err(|e| format!("创建下载目录失败: {e}"))?;
    let part = dest_dir.join(format!("{name}.part"));
    let target = dest_dir.join(&name);
    let result = async {
        let mut response = get(format!("{base}/{tag}/{name}")).await.and_then(|r| r.error_for_status()).map_err(|e| format!("下载失败: {e}"))?;
        let total = response.content_length();
        let mut file = std::fs::File::create(&part).map_err(|e| format!("写入下载文件失败: {e}"))?;
        let mut hasher = Sha256::new();
        let mut verifier = match &signature {
            Some((public, sig)) => Some(public.verify_stream(sig).map_err(|e| format!("签名校验失败: {e}"))?),
            None => None,
        };
        let mut downloaded = 0u64;
        on_progress(0, total);
        while let Some(chunk) = response.chunk().await.map_err(|e| format!("下载失败: {e}"))? {
            std::io::Write::write_all(&mut file, &chunk).map_err(|e| format!("写入下载文件失败: {e}"))?;
            hasher.update(&chunk);
            if let Some(v) = verifier.as_mut() {
                v.update(&chunk);
            }
            downloaded += chunk.len() as u64;
            on_progress(downloaded, total);
        }
        drop(file);
        if total.is_some_and(|t| t != downloaded) {
            return Err(format!("下载不完整: {downloaded}/{}", total.unwrap_or(0)));
        }
        if let Some(v) = verifier.as_mut() {
            v.finalize().map_err(|e| format!("安装包签名校验失败: {e}"))?;
        }
        let actual = hex(&hasher.finalize());
        if actual != expected {
            return Err(format!("安装包校验失败 (SHA-256 不匹配): {name}"));
        }
        std::fs::rename(&part, &target).map_err(|e| format!("保存安装包失败: {e}"))?;
        Ok(target.clone())
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Hand a verified installer to the OS: macOS opens the disk image, Windows starts the installer, Linux reveals
/// the AppImage (marked executable) because replacing a running AppImage is the user's call.
pub fn open_installer(path: &Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(path).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").args(["/C", "start", ""]).arg(path).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        Command::new("xdg-open").arg(path.parent().unwrap_or(path)).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("打开安装包失败: {s}")),
        Err(e) => Err(format!("打开安装包失败: {e}")),
    }
}

fn is_remote_version_newer(remote: &str, current: &str) -> bool {
    let remote_segments = parse_version_segments(remote);
    let current_segments = parse_version_segments(current);
    let max_len = remote_segments.len().max(current_segments.len()).max(3);

    for index in 0..max_len {
        let remote_part = *remote_segments.get(index).unwrap_or(&0);
        let current_part = *current_segments.get(index).unwrap_or(&0);
        if remote_part > current_part {
            return true;
        }
        if remote_part < current_part {
            return false;
        }
    }

    false
}

fn parse_version_segments(value: &str) -> Vec<u64> {
    let stable_version = value
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .split(['-', '+'])
        .next()
        .unwrap_or_default();

    stable_version
        .split('.')
        .filter_map(|segment| segment.parse::<u64>().ok())
        .collect()
}

fn parse_release_tag_from_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    let mut segments = url.path_segments()?;
    while let Some(segment) = segments.next() {
        if segment == "tag" {
            return segments.next().map(ToString::to_string);
        }
    }
    None
}

fn parse_release_notes_from_feed(feed: &str, tag: &str) -> Option<ReleaseNotes> {
    for entry in capture_all(feed, r"(?s)<entry\b[^>]*>(.*?)</entry>") {
        let link_href = capture_first(&entry, r#"<link\b[^>]*href="([^"]+)""#).unwrap_or_default();
        if !link_href.ends_with(&format!("/releases/tag/{}", tag)) {
            continue;
        }

        let title = capture_first(&entry, r"(?s)<title[^>]*>(.*?)</title>")
            .map(|value| decode_xml_entities(value.trim()))
            .unwrap_or_else(|| tag.to_string());
        let published_at = capture_first(&entry, r"(?s)<updated[^>]*>(.*?)</updated>")
            .map(|value| value.trim().to_string());
        let body = capture_first(&entry, r"(?s)<content[^>]*>(.*?)</content>")
            .map(|value| html_fragment_to_text(&decode_xml_entities(value.trim())))
            .unwrap_or_default();

        return Some(ReleaseNotes {
            title,
            body,
            published_at,
        });
    }

    None
}

fn capture_all(value: &str, pattern: &str) -> Vec<String> {
    regex::Regex::new(pattern)
        .ok()
        .map(|regex| {
            regex
                .captures_iter(value)
                .filter_map(|captures| captures.get(1).map(|match_| match_.as_str().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn capture_first(value: &str, pattern: &str) -> Option<String> {
    regex::Regex::new(pattern)
        .ok()?
        .captures(value)?
        .get(1)
        .map(|match_| match_.as_str().to_string())
}

fn html_fragment_to_text(value: &str) -> String {
    let with_line_breaks = regex::Regex::new(r"(?i)<\s*(br|/p|/li)\b[^>]*>")
        .map(|regex| regex.replace_all(value, "\n").to_string())
        .unwrap_or_else(|_| value.to_string());
    let without_tags = regex::Regex::new(r"(?s)<[^>]+>")
        .map(|regex| regex.replace_all(&with_line_breaks, "").to_string())
        .unwrap_or(with_line_breaks);

    without_tags
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_remote_version_newer_than_current() {
        assert!(is_remote_version_newer("v0.1.3", "0.1.2"));
        assert!(is_remote_version_newer("0.2.0", "0.1.9"));
        assert!(is_remote_version_newer("v1.0.0", "0.9.9"));
    }

    #[test]
    fn ignores_equal_or_older_remote_versions() {
        assert!(!is_remote_version_newer("v0.1.2", "0.1.2"));
        assert!(!is_remote_version_newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn ignores_prerelease_suffix_when_comparing_numeric_segments() {
        assert!(is_remote_version_newer("v0.1.3-beta.1", "0.1.2"));
        assert!(!is_remote_version_newer("v0.1.2-beta.1", "0.1.2"));
    }

    #[test]
    fn parses_latest_release_redirect_url_tag() {
        let tag =
            parse_release_tag_from_url("https://github.com/Mio888888/OpsBatch/releases/tag/v0.1.3");
        assert_eq!(Some("v0.1.3".to_string()), tag);
    }

    #[test]
    fn parses_release_notes_from_atom_feed() {
        let feed = r#"
            <feed>
              <entry>
                <title>v0.1.3</title>
                <link href="https://github.com/Mio888888/OpsBatch/releases/tag/v0.1.3" />
                <updated>2026-06-12T10:00:00Z</updated>
                <content type="html">&lt;ul&gt;&lt;li&gt;新增更新弹窗&lt;/li&gt;&lt;li&gt;修复检查更新频率限制&lt;/li&gt;&lt;/ul&gt;</content>
              </entry>
            </feed>
        "#;

        let notes = parse_release_notes_from_feed(feed, "v0.1.3").expect("release notes");

        assert_eq!("v0.1.3", notes.title);
        assert_eq!(Some("2026-06-12T10:00:00Z".to_string()), notes.published_at);
        assert!(notes.body.contains("新增更新弹窗"));
        assert!(notes.body.contains("修复检查更新频率限制"));
    }

    #[test]
    fn installer_names_follow_the_release_convention() {
        assert_eq!(installer_asset_name("v0.2.0", "macos", "aarch64").as_deref(), Some("OpsBatch-0.2.0-macos-aarch64.dmg"));
        assert_eq!(installer_asset_name("0.2.0", "windows", "amd64").as_deref(), Some("OpsBatch-0.2.0-windows-x86_64-setup.exe"));
        assert_eq!(installer_asset_name("0.2.0", "linux", "x86_64").as_deref(), Some("OpsBatch-0.2.0-linux-x86_64.AppImage"));
        assert!(installer_asset_name("0.2.0", "freebsd", "x86_64").is_none());
        assert!(installer_asset_name("0.2.0", "macos", "riscv64").is_none());
    }

    #[test]
    fn checksum_listing_is_parsed_strictly() {
        let h = "a".repeat(64);
        let listing = format!("{h}  OpsBatch-1.0.0-macos-aarch64.dmg\n{}  *other.exe\nbad  short.bin\n", "B".repeat(64));
        assert_eq!(parse_sha256sums(&listing, "OpsBatch-1.0.0-macos-aarch64.dmg"), Some(h));
        assert_eq!(parse_sha256sums(&listing, "other.exe"), Some("b".repeat(64)));
        assert_eq!(parse_sha256sums(&listing, "short.bin"), None);
        assert_eq!(parse_sha256sums(&listing, "missing"), None);
    }

    #[test]
    fn source_can_be_redirected_to_a_mirror() {
        let s = UpdateSource::with_base("http://127.0.0.1:9/");
        assert_eq!(s.releases_page_url, "http://127.0.0.1:9/releases/latest");
        assert_eq!(s.download_base, "http://127.0.0.1:9/releases/download");
    }
}
