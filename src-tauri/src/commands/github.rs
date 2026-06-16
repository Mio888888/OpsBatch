use crate::commands::github_git::{
    build_auth_callbacks, run_system_git_clone, run_system_git_update, should_retry_with_system_git,
};
use crate::db::Database;
use crate::security::SECRET_PLACEHOLDER;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub id: String,
    pub url: String,
    pub branch: String,
    pub token: Option<String>,
    pub has_token: bool,
    pub last_pulled_at: Option<String>,
    pub update_on_startup: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResult {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub deleted: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibraryLanguage {
    ZhCn,
    EnUs,
}

impl LibraryLanguage {
    fn from_app_language(language: Option<&str>) -> Self {
        match language {
            Some("zh-CN") => Self::ZhCn,
            Some("en-US") => Self::EnUs,
            _ => Self::EnUs,
        }
    }

    fn from_language_mode(language_mode: Option<&str>) -> Self {
        match language_mode {
            Some("zh-CN") => Self::ZhCn,
            Some("en-US") => Self::EnUs,
            Some("system") | None => Self::from_system_locale(),
            Some(_) => Self::from_system_locale(),
        }
    }

    fn from_system_locale() -> Self {
        let locale = env::var("LANG")
            .or_else(|_| env::var("LC_ALL"))
            .or_else(|_| env::var("LC_MESSAGES"))
            .unwrap_or_default()
            .to_lowercase();
        if locale.starts_with("zh") {
            Self::ZhCn
        } else {
            Self::EnUs
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::ZhCn => "_cn",
            Self::EnUs => "_en",
        }
    }

    fn library_metadata_file(self) -> &'static str {
        match self {
            Self::ZhCn => "library_cn.json",
            Self::EnUs => "library_en.json",
        }
    }

    fn app_language(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
}

fn path_has_language_suffix(path: &Path, language: LibraryLanguage) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map_or(false, |stem| {
            let script_stem = stem.strip_suffix(".meta").unwrap_or(stem);
            script_stem.ends_with(language.suffix())
        })
}

fn validate_library_metadata(repo_path: &Path, language: LibraryLanguage, result: &mut PullResult) {
    let metadata_path = repo_path.join(language.library_metadata_file());
    if !metadata_path.exists() {
        result.errors.push(format!(
            "library metadata missing: {}",
            language.library_metadata_file()
        ));
        return;
    }

    match std::fs::read_to_string(&metadata_path) {
        Ok(raw) => {
            if let Err(e) = serde_json::from_str::<serde_json::Value>(&raw) {
                result.errors.push(format!(
                    "library metadata {}: {}",
                    language.library_metadata_file(),
                    e
                ));
            }
        }
        Err(e) => result.errors.push(format!(
            "read library metadata {}: {}",
            language.library_metadata_file(),
            e
        )),
    }
}

fn repo_sync_library_language(conn: &rusqlite::Connection) -> LibraryLanguage {
    let language_mode = conn
        .query_row(
            "SELECT value FROM general_settings WHERE key='languageMode'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    LibraryLanguage::from_language_mode(language_mode.as_deref())
}

#[tauri::command]
pub async fn list_repos(db: tauri::State<'_, Database>) -> Result<Vec<RepoInfo>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, url, branch, token, last_pulled_at, update_on_startup, enabled FROM github_repos ORDER BY url"
    ).map_err(|e| e.to_string())?;
    let repos = stmt
        .query_map([], |row| {
            let token: Option<String> = row.get(3)?;
            let has_token = token.as_deref().is_some_and(|value| !value.is_empty());
            Ok(RepoInfo {
                id: row.get(0)?,
                url: row.get(1)?,
                branch: row.get(2)?,
                token: if has_token {
                    Some(SECRET_PLACEHOLDER.to_string())
                } else {
                    None
                },
                has_token,
                last_pulled_at: row.get(4)?,
                update_on_startup: row.get::<_, i32>(5)? == 1,
                enabled: row.get::<_, i32>(6)? == 1,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(repos)
}

#[tauri::command]
pub async fn add_repo(
    db: tauri::State<'_, Database>,
    url: String,
    branch: String,
    token: Option<String>,
    update_on_startup: bool,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let stored_token = match token.filter(|value| !value.is_empty()) {
        Some(value) => {
            crate::keychain::store_github_token(&id, &value)?;
            Some(SECRET_PLACEHOLDER.to_string())
        }
        None => None,
    };
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO github_repos (id, url, branch, token, update_on_startup, enabled) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![id, url, branch, stored_token, update_on_startup as i32],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn delete_repo(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    // Also remove all synced items from this repo
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM commands WHERE source_repo_id=?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scripts WHERE source_repo_id=?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM quick_actions WHERE source_repo_id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM github_repos WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    let _ = crate::keychain::delete_github_token(&id);
    Ok(())
}

#[tauri::command]
pub async fn toggle_repo(
    db: tauri::State<'_, Database>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE github_repos SET enabled=?1 WHERE id=?2",
        params![enabled as i32, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_repo_update_on_startup(
    db: tauri::State<'_, Database>,
    id: String,
    update_on_startup: bool,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE github_repos SET update_on_startup=?1 WHERE id=?2",
        params![update_on_startup as i32, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn resolve_repo_token(repo_id: &str, stored: Option<String>) -> Result<Option<String>, String> {
    match stored.as_deref() {
        Some(SECRET_PLACEHOLDER) => crate::keychain::get_github_token(repo_id)
            .map(Some)
            .map_err(|e| match e {
                crate::keychain::SecretError::Missing => format!(
                    "GitHub 仓库 {} 的 token 未在本地加密存储中找到，请重新保存仓库 token。",
                    repo_id
                ),
                other => other.to_string(),
            }),
        Some(value) if !value.is_empty() => Ok(Some(value.to_string())),
        _ => Ok(None),
    }
}

fn log_startup_repo_update_summary(app: &AppHandle, results: &[StartupUpdateResult]) {
    if results.is_empty() {
        return;
    }
    let success_count = results.iter().filter(|item| item.pulled).count();
    let error_count = results.len().saturating_sub(success_count);
    let level = if error_count > 0 { "warn" } else { "info" };
    let message = format!(
        "Startup repository update finished: {} synced, {} failed",
        success_count, error_count
    );
    crate::commands::app_log::emit_log(app, level, "repo-sync", &message, "backend");
}

#[tauri::command]
pub fn pull_repo(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    repo_id: String,
    language: Option<String>,
) -> Result<PullResult, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (url, branch, token): (String, String, Option<String>) = conn
        .query_row(
            "SELECT url, branch, token FROM github_repos WHERE id=?1",
            params![repo_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let token = resolve_repo_token(&repo_id, token)?;

    let sync_language = LibraryLanguage::from_app_language(language.as_deref());

    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let repos_dir = app_data_dir.join("repos");
    std::fs::create_dir_all(&repos_dir).ok();

    let repo_hash = format!("{:x}", md5_hash(url.as_bytes()));
    let local_path = repos_dir.join(&repo_hash);

    let mut result = PullResult {
        added: Vec::new(),
        updated: Vec::new(),
        deleted: Vec::new(),
        errors: Vec::new(),
    };

    drop(conn);

    // Clone or pull
    if local_path.exists() {
        // Pull existing repo
        let repo =
            git2::Repository::open(&local_path).map_err(|e| format!("open repo failed: {}", e))?;
        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| format!("find remote failed: {}", e))?;

        let mut fetch_options = git2::FetchOptions::new();
        if let Some(ref t) = token {
            let callbacks = build_auth_callbacks(t)?;
            fetch_options.remote_callbacks(callbacks);
        }

        let used_system_git = match remote.fetch(&[&branch], Some(&mut fetch_options), None) {
            Ok(_) => false,
            Err(e) if should_retry_with_system_git(&e) => {
                run_system_git_update(&local_path, &branch, token.as_deref()).map_err(
                    |git_error| {
                        format!(
                            "fetch failed: {}; system git fallback failed: {}",
                            e, git_error
                        )
                    },
                )?;
                true
            }
            Err(e) => return Err(format!("fetch failed: {}", e)),
        };

        if !used_system_git {
            let fetch_head = repo
                .find_reference("FETCH_HEAD")
                .map_err(|e| format!("find FETCH_HEAD failed: {}", e))?;
            let fetch_commit = repo
                .reference_to_annotated_commit(&fetch_head)
                .map_err(|e| e.to_string())?;
            let analysis = repo
                .merge_analysis(&[&fetch_commit])
                .map_err(|e| e.to_string())?;

            if analysis.0.is_up_to_date() {
                // Repo is up to date, still continue to sync library items
            } else {
                if analysis.0.is_fast_forward() {
                    let refname = format!("refs/heads/{}", branch);
                    let mut reference = repo.find_reference(&refname).map_err(|e| e.to_string())?;
                    reference
                        .set_target(fetch_commit.id(), "Fast-forward")
                        .map_err(|e| e.to_string())?;
                    repo.set_head(&refname).map_err(|e| e.to_string())?;
                    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                        .map_err(|e| e.to_string())?;
                } else {
                    // Merge
                    repo.merge(&[&fetch_commit], None, None)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    } else {
        // Clone new repo
        let mut fetch_options = git2::FetchOptions::new();
        if let Some(ref t) = token {
            let callbacks = build_auth_callbacks(t)?;
            fetch_options.remote_callbacks(callbacks);
        }

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        builder.branch(&branch);

        match builder.clone(&url, &local_path) {
            Ok(_) => {}
            Err(e) if should_retry_with_system_git(&e) => {
                if local_path.exists() {
                    std::fs::remove_dir_all(&local_path).map_err(|remove_error| {
                        format!("cleanup failed clone: {}", remove_error)
                    })?;
                }
                run_system_git_clone(&url, &branch, &local_path, token.as_deref()).map_err(
                    |git_error| {
                        format!(
                            "clone failed: {}; system git fallback failed: {}",
                            e, git_error
                        )
                    },
                )?;
            }
            Err(e) => return Err(format!("clone failed: {}", e)),
        }
    }

    // Sync commands and scripts from the cloned repo + update last_pulled_at
    {
        let conn2 = db.pool.get().map_err(|e| e.to_string())?;
        sync_library_from_repo(&conn2, &repo_id, &local_path, sync_language, &mut result);
        conn2
            .execute(
                "UPDATE github_repos SET last_pulled_at=datetime('now','localtime') WHERE id=?1",
                params![repo_id],
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(result)
}

fn md5_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0;
    for &byte in data {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
    }
    hash
}

#[cfg(test)]
mod git_cli_tests {
    use super::*;

    #[test]
    fn sync_imports_url_only_script_json_as_remote_script() {
        let repo_id = "remote-json-repo";
        let temp_dir = std::env::temp_dir().join(format!(
            "opsbatch-remote-script-test-{}",
            uuid::Uuid::new_v4()
        ));
        let scripts_dir = temp_dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(temp_dir.join("library_cn.json"), "{}").unwrap();
        std::fs::write(
            scripts_dir.join("ecs_cn.json"),
            r#"{
              "name": "ECS/融合怪综合测评",
              "url": "https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh",
              "language": "shell",
              "category": "巡检",
              "tags": ["巡检", "基准测试"],
              "risk": "medium",
              "description": "远程 ECS 综合测评脚本",
              "parameters": [
                {"name": "-m", "description": "上游脚本运行模式", "required": false, "default": ""}
              ],
              "platform": ["linux"]
            }"#,
        )
        .unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE commands (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                platform TEXT DEFAULT 'linux',
                parameters TEXT DEFAULT '[]',
                url TEXT DEFAULT '',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0,
                source_repo_id TEXT
            );
            CREATE TABLE scripts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                language TEXT DEFAULT 'shell',
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                content TEXT DEFAULT '',
                parameters TEXT DEFAULT '[]',
                url TEXT DEFAULT '',
                platform TEXT DEFAULT 'linux',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0,
                source_repo_id TEXT
            );
            CREATE TABLE quick_actions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                parameters TEXT DEFAULT '[]',
                sort_order INTEGER DEFAULT 0,
                starred INTEGER DEFAULT 0,
                description TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                language TEXT DEFAULT 'shell',
                source_repo_id TEXT
            );
            ",
        )
        .unwrap();

        let mut result = PullResult {
            added: Vec::new(),
            updated: Vec::new(),
            deleted: Vec::new(),
            errors: Vec::new(),
        };

        sync_library_from_repo(
            &conn,
            repo_id,
            &temp_dir,
            LibraryLanguage::ZhCn,
            &mut result,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(vec!["script: ecs_cn.json"], result.added);

        let (name, content, url, parameters): (String, String, String, String) = conn
            .query_row(
                "SELECT name, content, url, parameters FROM scripts WHERE source_repo_id=?1",
                params![repo_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!("ECS/融合怪综合测评", name);
        assert_eq!("", content);
        assert_eq!(
            "https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh",
            url
        );
        assert!(parameters.contains("\"defaultValue\":\"\""));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn sync_imports_url_only_script_meta_json_as_remote_script() {
        let repo_id = "remote-meta-json-repo";
        let temp_dir = std::env::temp_dir().join(format!(
            "opsbatch-remote-meta-script-test-{}",
            uuid::Uuid::new_v4()
        ));
        let scripts_dir = temp_dir.join("scripts").join("shell");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(temp_dir.join("library_cn.json"), "{}").unwrap();
        std::fs::write(
            scripts_dir.join("ecs-benchmark_cn.meta.json"),
            r#"{
              "name": "ECS/融合怪综合测评",
              "url": "https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh",
              "language": "shell",
              "category": "巡检",
              "tags": ["巡检", "基准测试"],
              "risk": "medium",
              "description": "远程 ECS 综合测评脚本",
              "platform": ["linux"]
            }"#,
        )
        .unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_repo_sync_test_tables(&conn);

        let mut result = PullResult {
            added: Vec::new(),
            updated: Vec::new(),
            deleted: Vec::new(),
            errors: Vec::new(),
        };

        sync_library_from_repo(
            &conn,
            repo_id,
            &temp_dir,
            LibraryLanguage::ZhCn,
            &mut result,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            vec!["script: shell/ecs-benchmark_cn.meta.json"],
            result.added
        );

        let url: String = conn
            .query_row(
                "SELECT url FROM scripts WHERE source_repo_id=?1",
                params![repo_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(
            "https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh",
            url
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn create_repo_sync_test_tables(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "
            CREATE TABLE commands (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                platform TEXT DEFAULT 'linux',
                parameters TEXT DEFAULT '[]',
                url TEXT DEFAULT '',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0,
                source_repo_id TEXT
            );
            CREATE TABLE scripts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                language TEXT DEFAULT 'shell',
                category TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                risk TEXT DEFAULT 'low',
                description TEXT DEFAULT '',
                content TEXT DEFAULT '',
                parameters TEXT DEFAULT '[]',
                url TEXT DEFAULT '',
                platform TEXT DEFAULT 'linux',
                starred INTEGER DEFAULT 0,
                is_builtin INTEGER DEFAULT 0,
                source_repo_id TEXT
            );
            CREATE TABLE quick_actions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT DEFAULT '',
                parameters TEXT DEFAULT '[]',
                sort_order INTEGER DEFAULT 0,
                starred INTEGER DEFAULT 0,
                description TEXT DEFAULT '',
                tags TEXT DEFAULT '[]',
                language TEXT DEFAULT 'shell',
                source_repo_id TEXT
            );
            ",
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// YAML / JSON helpers for repo library parsing
// ---------------------------------------------------------------------------

/// Simple YAML parser for command files with predictable structure.
/// Handles: `key: value`, `key: |` (literal block), `key:` followed by `  - item`,
/// and `key:` followed by `  - name: val\n    desc: val` (sequence of objects).
fn parse_yaml_to_json(content: &str) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut map = Map::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            i += 1;
            continue;
        }

        // Only process top-level keys (no leading whitespace)
        if line.starts_with(' ') || line.starts_with('\t') {
            i += 1;
            continue;
        }

        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].to_string();
            let rest = line[colon_pos + 1..].trim();

            if rest == "|" {
                // Literal block scalar — collect indented lines
                i += 1;
                let mut block = Vec::new();
                while i < lines.len() {
                    let next = lines[i];
                    if next.trim().is_empty() {
                        // Empty line inside block
                        if i + 1 < lines.len()
                            && (lines[i + 1].starts_with("  ") || lines[i + 1].starts_with('\t'))
                        {
                            block.push(String::new());
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    if !next.starts_with("  ") && !next.starts_with('\t') {
                        break;
                    }
                    let stripped = if next.starts_with("  ") {
                        &next[2..]
                    } else {
                        &next[1..] // tab
                    };
                    block.push(stripped.to_string());
                    i += 1;
                }
                while block.last().map_or(false, |s| s.is_empty()) {
                    block.pop();
                }
                map.insert(key, Value::String(block.join("\n")));
                continue;
            } else if rest.is_empty() {
                // Sequence or empty value — try to parse as sequence of objects
                i += 1;

                // Peek ahead: are the items objects (e.g. "- name: foo") or simple strings?
                let mut items = Vec::new();
                while i < lines.len() {
                    let next = lines[i];
                    let trimmed = next.trim_start();
                    if trimmed.starts_with("- ") {
                        let after_dash = trimmed[2..].trim();

                        // Check if this "- " starts an object (contains ": ")
                        if let Some(obj_colon) = after_dash.find(": ") {
                            // Sequence of objects
                            let mut obj = Map::new();
                            let obj_key = &after_dash[..obj_colon];
                            let obj_val = after_dash[obj_colon + 2..].trim();
                            // Strip surrounding quotes
                            let obj_val_unquoted = obj_val
                                .strip_prefix('"')
                                .and_then(|v| v.strip_suffix('"'))
                                .unwrap_or(obj_val);
                            obj.insert(
                                obj_key.to_string(),
                                Value::String(obj_val_unquoted.to_string()),
                            );
                            i += 1;

                            // Collect continuation lines (same indentation or deeper)
                            while i < lines.len() {
                                let cont = lines[i];
                                let cont_trimmed = cont.trim_start();
                                // Stop if we hit another list item or a top-level key
                                if cont_trimmed.starts_with("- ")
                                    || (!cont.starts_with(' ')
                                        && !cont.starts_with('\t')
                                        && !cont.trim().is_empty())
                                {
                                    break;
                                }
                                if cont.trim().is_empty() {
                                    break;
                                }
                                if let Some(cpos) = cont_trimmed.find(": ") {
                                    let ck = cont_trimmed[..cpos].to_string();
                                    let cv = cont_trimmed[cpos + 2..].trim();
                                    let cv_unquoted = cv
                                        .strip_prefix('"')
                                        .and_then(|v| v.strip_suffix('"'))
                                        .unwrap_or(cv);
                                    obj.insert(ck, Value::String(cv_unquoted.to_string()));
                                }
                                i += 1;
                            }

                            items.push(Value::Object(obj));
                        } else {
                            // Simple string item
                            items.push(Value::String(after_dash.to_string()));
                            i += 1;
                        }
                    } else if trimmed == "-" {
                        items.push(Value::String(String::new()));
                        i += 1;
                    } else {
                        break;
                    }
                }
                map.insert(key, Value::Array(items));
                continue;
            } else {
                // Simple scalar
                map.insert(key, Value::String(rest.to_string()));
                i += 1;
                continue;
            }
        }

        i += 1;
    }

    Value::Object(map)
}

/// Convert platform array from repo format (e.g. `["linux", "macos"]`) to DB
/// single-value format (`"linux"` | `"windows"` | `"both"`).
fn platform_array_to_string(platforms: &[serde_json::Value]) -> String {
    let has_linux = platforms
        .iter()
        .any(|p| p.as_str().map_or(false, |s| s == "linux" || s == "macos"));
    let has_windows = platforms
        .iter()
        .any(|p| p.as_str().map_or(false, |s| s == "windows"));
    if has_linux && has_windows {
        "both".to_string()
    } else if has_windows {
        "windows".to_string()
    } else {
        "linux".to_string()
    }
}

/// Generate a deterministic ID for a repo-synced library item.
fn repo_item_id(repo_id: &str, relative_path: &str) -> String {
    format!(
        "repo-{:016x}-{:016x}",
        md5_hash(repo_id.as_bytes()),
        md5_hash(relative_path.as_bytes())
    )
}

/// Count existing library items from a specific repo.
fn count_repo_items(conn: &rusqlite::Connection, table: &str, repo_id: &str) -> usize {
    let sql = format!("SELECT COUNT(*) FROM {} WHERE source_repo_id = ?", table);
    conn.query_row(&sql, params![repo_id], |row| row.get::<_, i32>(0))
        .unwrap_or(0) as usize
}

// ---------------------------------------------------------------------------
// Import commands
// ---------------------------------------------------------------------------

fn import_commands_recursive(
    conn: &rusqlite::Connection,
    repo_id: &str,
    dir: &Path,
    base_dir: &Path,
    language: LibraryLanguage,
    result: &mut PullResult,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            result
                .errors
                .push(format!("read dir {}: {}", dir.display(), err));
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            import_commands_recursive(conn, repo_id, &path, base_dir, language, result);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        if !path_has_language_suffix(&path, language) {
            continue;
        }

        let relative = path
            .strip_prefix(base_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let id = repo_item_id(repo_id, &relative);

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                result
                    .errors
                    .push(format!("read command {}: {}", relative, e));
                continue;
            }
        };

        let yaml = parse_yaml_to_json(&content);
        let name = yaml
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let command = yaml
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let url = yaml
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let category = yaml
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags = yaml
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
            })
            .unwrap_or_else(|| "[]".to_string());
        let risk = yaml
            .get("risk")
            .and_then(|v| v.as_str())
            .unwrap_or("low")
            .to_string();
        let description = yaml
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let platform_arr: Vec<serde_json::Value> = yaml
            .get("platform")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let platform = if platform_arr.is_empty() {
            "linux".to_string()
        } else {
            platform_array_to_string(&platform_arr)
        };

        // Parse parameters from YAML (repo format: {name, description, required, default})
        let parameters = yaml
            .get("parameters")
            .and_then(|v| v.as_array())
            .map(|arr| {
                let transformed: Vec<serde_json::Value> = arr
                    .iter()
                    .map(|p| {
                        let mut obj = serde_json::Map::new();
                        obj.insert(
                            "name".to_string(),
                            p.get("name")
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new())),
                        );
                        obj.insert(
                            "description".to_string(),
                            p.get("description")
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new())),
                        );
                        obj.insert(
                            "required".to_string(),
                            p.get("required")
                                .and_then(|v| v.as_bool())
                                .map(serde_json::Value::Bool)
                                .unwrap_or(serde_json::Value::Bool(false)),
                        );
                        let default_val = p
                            .get("default")
                            .cloned()
                            .unwrap_or(serde_json::Value::String(String::new()));
                        obj.insert("defaultValue".to_string(), default_val);
                        serde_json::Value::Object(obj)
                    })
                    .collect();
                serde_json::to_string(&transformed).unwrap_or_else(|_| "[]".to_string())
            })
            .unwrap_or_else(|| "[]".to_string());

        match conn.execute(
            "INSERT INTO commands (id, name, command, category, tags, risk, description, platform, parameters, url, starred, is_builtin, source_repo_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0, ?11)",
            params![id, name, command, category, tags, risk, description, platform, parameters, url, repo_id],
        ) {
            Ok(_) => result.added.push(format!("command: {}", relative)),
            Err(e) => result
                .errors
                .push(format!("command {}: {}", relative, e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Import scripts
// ---------------------------------------------------------------------------

fn script_parameters_from_meta(meta: &serde_json::Value) -> String {
    meta.get("parameters")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let transformed: Vec<serde_json::Value> = arr
                .iter()
                .map(|p| {
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "name".to_string(),
                        p.get("name")
                            .cloned()
                            .unwrap_or(serde_json::Value::String(String::new())),
                    );
                    obj.insert(
                        "description".to_string(),
                        p.get("description")
                            .cloned()
                            .unwrap_or(serde_json::Value::String(String::new())),
                    );
                    obj.insert(
                        "required".to_string(),
                        p.get("required")
                            .and_then(|v| v.as_bool())
                            .map(serde_json::Value::Bool)
                            .unwrap_or(serde_json::Value::Bool(false)),
                    );
                    let default_val = p
                        .get("default")
                        .cloned()
                        .unwrap_or(serde_json::Value::String(String::new()));
                    obj.insert("defaultValue".to_string(), default_val);
                    serde_json::Value::Object(obj)
                })
                .collect();
            serde_json::to_string(&transformed).unwrap_or_else(|_| "[]".to_string())
        })
        .unwrap_or_else(|| "[]".to_string())
}

fn script_platform_from_meta(meta: &serde_json::Value, default_platform: &str) -> String {
    let platform_arr: Vec<serde_json::Value> = meta
        .get("platform")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if platform_arr.is_empty() {
        default_platform.to_string()
    } else {
        platform_array_to_string(&platform_arr)
    }
}

fn import_scripts_recursive(
    conn: &rusqlite::Connection,
    repo_id: &str,
    dir: &Path,
    base_dir: &Path,
    language_filter: LibraryLanguage,
    result: &mut PullResult,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            result
                .errors
                .push(format!("read dir {}: {}", dir.display(), err));
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            import_scripts_recursive(conn, repo_id, &path, base_dir, language_filter, result);
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "json" {
            if !path_has_language_suffix(&path, language_filter) {
                continue;
            }

            let relative = path
                .strip_prefix(base_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let raw = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    result
                        .errors
                        .push(format!("read script {}: {}", relative, e));
                    continue;
                }
            };
            let meta: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    result
                        .errors
                        .push(format!("parse script {}: {}", relative, e));
                    continue;
                }
            };
            let url = match meta.get("url").and_then(|v| v.as_str()) {
                Some(v) if !v.trim().is_empty() => v.to_string(),
                _ => continue,
            };

            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let id = repo_item_id(repo_id, &relative);
            let name = meta
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&stem)
                .to_string();
            let language = meta
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("shell")
                .to_string();
            let category = meta
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tags = meta
                .get("tags")
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "[]".to_string());
            let risk = meta
                .get("risk")
                .and_then(|v| v.as_str())
                .unwrap_or("low")
                .to_string();
            let description = meta
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let parameters = script_parameters_from_meta(&meta);
            let platform = script_platform_from_meta(&meta, "linux");

            match conn.execute(
                "INSERT INTO scripts (id, name, language, category, tags, risk, description, content, parameters, url, platform, starred, is_builtin, source_repo_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', ?8, ?9, ?10, 0, 0, ?11)",
                params![id, name, language, category, tags, risk, description, parameters, url, platform, repo_id],
            ) {
                Ok(_) => result.added.push(format!("script: {}", relative)),
                Err(e) => result.errors.push(format!("script {}: {}", relative, e)),
            }
            continue;
        }
        if !["sh", "py", "ps1"].contains(&ext) {
            continue;
        }
        if !path_has_language_suffix(&path, language_filter) {
            continue;
        }

        let relative = path
            .strip_prefix(base_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let id = repo_item_id(repo_id, &relative);

        // Read script content
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                result
                    .errors
                    .push(format!("read script {}: {}", relative, e));
                continue;
            }
        };

        // Try to read .meta.json
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let meta_file_name = format!("{}.meta.json", stem);
        let meta_path = path.parent().unwrap_or(dir).join(&meta_file_name);

        let default_lang = match ext {
            "sh" => "shell",
            "py" => "python",
            "ps1" => "powershell",
            _ => "shell",
        };

        let mut name = stem.clone();
        let mut language = default_lang.to_string();
        let mut category = String::new();
        let mut tags = "[]".to_string();
        let mut risk = "low".to_string();
        let mut description = String::new();
        let mut parameters = "[]".to_string();
        let mut url = String::new();
        let mut platform = "linux".to_string();

        if meta_path.exists() {
            if let Ok(meta_content) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&meta_content) {
                    if let Some(v) = meta.get("name").and_then(|v| v.as_str()) {
                        name = v.to_string();
                    }
                    if let Some(v) = meta.get("language").and_then(|v| v.as_str()) {
                        language = v.to_string();
                    }
                    if let Some(v) = meta.get("category").and_then(|v| v.as_str()) {
                        category = v.to_string();
                    }
                    if let Some(arr) = meta.get("tags").and_then(|v| v.as_array()) {
                        if let Ok(s) = serde_json::to_string(arr) {
                            tags = s;
                        }
                    }
                    if let Some(v) = meta.get("risk").and_then(|v| v.as_str()) {
                        risk = v.to_string();
                    }
                    if let Some(v) = meta.get("description").and_then(|v| v.as_str()) {
                        description = v.to_string();
                    }
                    if let Some(v) = meta.get("url").and_then(|v| v.as_str()) {
                        url = v.to_string();
                    }
                    // Transform parameters from repo format to frontend format:
                    // repo: {name, description, required, default}
                    // frontend: {name, description, required, defaultValue}
                    parameters = script_parameters_from_meta(&meta);
                    platform = script_platform_from_meta(&meta, &platform);
                }
            }
        }

        match conn.execute(
            "INSERT INTO scripts (id, name, language, category, tags, risk, description, content, parameters, url, platform, starred, is_builtin, source_repo_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0, ?12)",
            params![id, name, language, category, tags, risk, description, content, parameters, url, platform, repo_id],
        ) {
            Ok(_) => result.added.push(format!("script: {}", relative)),
            Err(e) => result.errors.push(format!("script {}: {}", relative, e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Import quick actions
// ---------------------------------------------------------------------------

fn import_quick_actions(
    conn: &rusqlite::Connection,
    repo_id: &str,
    qa_dir: &Path,
    repo_path: &Path,
    language: LibraryLanguage,
    result: &mut PullResult,
) {
    let entries = match std::fs::read_dir(qa_dir) {
        Ok(e) => e,
        Err(err) => {
            result
                .errors
                .push(format!("read dir {}: {}", qa_dir.display(), err));
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "json" {
            continue;
        }
        if !path_has_language_suffix(&path, language) {
            continue;
        }

        let relative = path
            .strip_prefix(repo_path)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let id = repo_item_id(repo_id, &relative);

        let raw = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                result
                    .errors
                    .push(format!("read quick-action {}: {}", relative, e));
                continue;
            }
        };

        let qa: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                result
                    .errors
                    .push(format!("parse quick-action {}: {}", relative, e));
                continue;
            }
        };

        let name = qa
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let category = qa
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags = qa
            .get("tags")
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "[]".to_string());
        let description = qa
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Resolve steps into a single command string
        let command = resolve_quick_action_steps(&qa, repo_path, language);

        if command.is_empty() {
            result
                .errors
                .push(format!("quick-action {}: no commands resolved", relative));
            continue;
        }

        match conn.execute(
            "INSERT INTO quick_actions (id, name, command, category, parameters, sort_order, starred, description, tags, language, source_repo_id) VALUES (?1, ?2, ?3, ?4, '[]', 0, 0, ?5, ?6, 'shell', ?7)",
            params![id, name, command, category, description, tags, repo_id],
        ) {
            Ok(_) => result
                .added
                .push(format!("quick-action: {}", relative)),
            Err(e) => result
                .errors
                .push(format!("quick-action {}: {}", relative, e)),
        }
    }
}

/// Resolve a quick-action's `steps` into a single command string by reading
/// referenced command YAML and script files from the repo.
fn resolve_quick_action_steps(
    qa: &serde_json::Value,
    repo_path: &Path,
    language: LibraryLanguage,
) -> String {
    let steps = match qa.get("steps").and_then(|v| v.as_array()) {
        Some(s) => s,
        None => return String::new(),
    };

    let mut commands = Vec::new();
    for step in steps {
        let step_type = step
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("command");
        let ref_path = match step.get("ref").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };

        let full_path = repo_path.join(ref_path);
        if !path_has_language_suffix(&full_path, language) {
            continue;
        }
        match step_type {
            "command" => {
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    let yaml = parse_yaml_to_json(&content);
                    // URL-based command: generate curl download-and-execute
                    if let Some(url) = yaml.get("url").and_then(|v| v.as_str()) {
                        if !url.is_empty() {
                            if let Ok(quoted_url) = crate::security::shell_quote(url) {
                                commands.push(format!("curl -sSL {} | bash", quoted_url));
                            }
                        }
                    } else if let Some(cmd) = yaml.get("command").and_then(|v| v.as_str()) {
                        // Local command template
                        commands.push(cmd.to_string());
                    }
                }
            }
            "script" => {
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    commands.push(content);
                }
            }
            _ => {}
        }
    }

    commands.join("\n")
}

// ---------------------------------------------------------------------------
// Main sync function
// ---------------------------------------------------------------------------

fn sync_library_from_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
    repo_path: &PathBuf,
    language: LibraryLanguage,
    result: &mut PullResult,
) {
    validate_library_metadata(repo_path, language, result);

    // Count existing items from this repo for accurate diff reporting
    let old_cmds = count_repo_items(conn, "commands", repo_id);
    let old_scripts = count_repo_items(conn, "scripts", repo_id);
    let old_qa = count_repo_items(conn, "quick_actions", repo_id);
    let old_total = old_cmds + old_scripts + old_qa;

    // Delete all previously synced items from this repo
    conn.execute(
        "DELETE FROM commands WHERE source_repo_id = ?1",
        params![repo_id],
    )
    .unwrap_or(0);
    conn.execute(
        "DELETE FROM scripts WHERE source_repo_id = ?1",
        params![repo_id],
    )
    .unwrap_or(0);
    conn.execute(
        "DELETE FROM quick_actions WHERE source_repo_id = ?1",
        params![repo_id],
    )
    .unwrap_or(0);

    // Import commands from YAML files
    let commands_dir = repo_path.join("commands");
    if commands_dir.exists() {
        import_commands_recursive(
            conn,
            repo_id,
            &commands_dir,
            &commands_dir,
            language,
            result,
        );
    }

    // Import scripts (with .meta.json metadata)
    let scripts_dir = repo_path.join("scripts");
    if scripts_dir.exists() {
        import_scripts_recursive(conn, repo_id, &scripts_dir, &scripts_dir, language, result);
    }

    // Import quick actions
    let qa_dir = repo_path.join("quick-actions");
    if qa_dir.exists() {
        import_quick_actions(conn, repo_id, &qa_dir, repo_path, language, result);
    }

    // Adjust result classification: on re-sync, report items as "updated" instead of "added"
    if old_total > 0 {
        let new_items = std::mem::take(&mut result.added);
        let new_total = new_items.len();

        // Classify by type
        for item in &new_items {
            result.updated.push(item.clone());
        }

        // Report removed items
        let removed = old_total as isize - new_total as isize;
        if removed > 0 {
            result
                .deleted
                .push(format!("{} items removed from repo", removed));
        }
    }
}

// ---------------------------------------------------------------------------
// Startup updates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupUpdateResult {
    pub repo_id: String,
    pub url: String,
    pub pulled: bool,
    pub message: String,
}

/// Pull all enabled repos that are configured to update when OpsBatch starts.
pub fn update_startup_repos(
    app: AppHandle,
    db: tauri::State<'_, Database>,
) -> Result<Vec<StartupUpdateResult>, String> {
    let repos: Vec<(String, String)> = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, url FROM github_repos WHERE enabled=1 AND update_on_startup=1")
            .map_err(|e| e.to_string())?;

        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    let mut results = Vec::new();
    let language = {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        repo_sync_library_language(&conn)
    };

    for (repo_id, url) in &repos {
        match pull_repo(
            app.clone(),
            db.clone(),
            repo_id.clone(),
            Some(language.app_language().to_string()),
        ) {
            Ok(_pull_result) => {
                results.push(StartupUpdateResult {
                    repo_id: repo_id.clone(),
                    url: url.clone(),
                    pulled: true,
                    message: "同步成功".to_string(),
                });
            }
            Err(e) => results.push(StartupUpdateResult {
                repo_id: repo_id.clone(),
                url: url.clone(),
                pulled: false,
                message: format!("同步失败: {}", e),
            }),
        }
    }

    Ok(results)
}

#[tauri::command]
pub fn run_startup_repo_updates(
    app: AppHandle,
    db: tauri::State<'_, Database>,
) -> Result<Vec<StartupUpdateResult>, String> {
    let results = update_startup_repos(app.clone(), db)?;
    log_startup_repo_update_summary(&app, &results);
    Ok(results)
}
