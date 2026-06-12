use serde::Serialize;

const RELEASES_PAGE_URL: &str = "https://github.com/Mio888888/OpsBatch/releases/latest";
const RELEASES_ATOM_URL: &str = "https://github.com/Mio888888/OpsBatch/releases.atom";

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

#[tauri::command]
pub async fn check_app_update(app: tauri::AppHandle) -> Result<AppUpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    let response = reqwest::Client::new()
        .get(RELEASES_PAGE_URL)
        .header(reqwest::header::USER_AGENT, "OpsBatch update checker")
        .send()
        .await
        .map_err(|e| format!("检查更新失败: {}", e))?
        .error_for_status()
        .map_err(|e| format!("检查更新失败: {}", e))?;
    let release_url = response.url().to_string();
    let latest_version = parse_release_tag_from_url(&release_url)
        .ok_or_else(|| format!("解析更新信息失败: {}", release_url))?;
    let release_notes = fetch_release_notes(&latest_version).await.ok();

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

async fn fetch_release_notes(tag: &str) -> Result<ReleaseNotes, String> {
    let feed = reqwest::Client::new()
        .get(RELEASES_ATOM_URL)
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
}
