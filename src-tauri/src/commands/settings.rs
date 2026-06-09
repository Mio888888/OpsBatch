use crate::db::Database;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DangerRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub enabled: bool,
    pub is_builtin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontFamilyInfo {
    pub family: String,
    pub styles: Vec<String>,
}

#[tauri::command]
pub async fn list_danger_rules(db: tauri::State<'_, Database>) -> Result<Vec<DangerRule>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, pattern, enabled, is_builtin FROM danger_rules ORDER BY is_builtin DESC, name"
    ).map_err(|e| e.to_string())?;
    let rules = stmt
        .query_map([], |row| {
            Ok(DangerRule {
                id: row.get(0)?,
                name: row.get(1)?,
                pattern: row.get(2)?,
                enabled: row.get::<_, i32>(3)? == 1,
                is_builtin: row.get::<_, i32>(4)? == 1,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let builtin = vec![
        ("rm -rf /", "rm\\s+-rf\\s+/"),
        ("rm -rf ~", "rm\\s+-rf\\s+~"),
        ("fork bomb", ":\\(\\)\\{\\s*:\\|:\\&\\s*\\}\\s*;"),
        ("dd overwrite", "dd\\s+if="),
        ("mkfs", "mkfs\\."),
        ("/dev/sda redirect", ">\\s*/dev/sda"),
        ("chmod 777 /", "chmod\\s+-R\\s+777\\s+/"),
        ("chown /", "chown\\s+-R\\s+\\w+\\s+/"),
        ("shutdown", "shutdown\\s+"),
        ("reboot", "reboot\\b"),
        ("init 0/6", "init\\s+[06]\\b"),
        ("drop database", "(?:DROP|drop)\\s+(?:DATABASE|database|SCHEMA|schema)"),
        ("truncate table", "(?:TRUNCATE|truncate)\\s+(?:TABLE|table)?\\s*\\w"),
        ("systemctl stop critical", "systemctl\\s+(?:stop|disable|mask)\\s+(?:sshd|nginx|docker|firewalld|NetworkManager|systemd)\\b"),
        ("iptables flush", "iptables\\s+-F"),
        ("rm -rf var/log", "rm\\s+-rf\\s+/var/log"),
        ("wipefs", "wipefs\\s+-a"),
        ("> /etc/passwd", ">\\s*/etc/passwd"),
        ("mv /* /dev/null", "mv\\s+/\\S*\\s+/dev/null"),
    ];

    let existing_names: Vec<String> = rules
        .iter()
        .filter(|r| r.is_builtin)
        .map(|r| r.name.clone())
        .collect();

    let missing: Vec<_> = builtin
        .into_iter()
        .filter(|(name, _)| !existing_names.iter().any(|n| n == *name))
        .collect();

    if !missing.is_empty() {
        for (name, pattern) in &missing {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO danger_rules (id, name, pattern, enabled, is_builtin) VALUES (?1, ?2, ?3, 1, 1)",
                params![id, name, pattern],
            ).ok();
        }
        let mut stmt2 = conn.prepare(
            "SELECT id, name, pattern, enabled, is_builtin FROM danger_rules ORDER BY is_builtin DESC, name"
        ).map_err(|e| e.to_string())?;
        return stmt2
            .query_map([], |row| {
                Ok(DangerRule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    pattern: row.get(2)?,
                    enabled: row.get::<_, i32>(3)? == 1,
                    is_builtin: row.get::<_, i32>(4)? == 1,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
    }

    Ok(rules)
}

#[tauri::command]
pub async fn add_danger_rule(
    db: tauri::State<'_, Database>,
    name: String,
    pattern: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    // Validate regex
    regex::Regex::new(&pattern).map_err(|e| format!("无效的正则表达式: {}", e))?;
    conn.execute(
        "INSERT INTO danger_rules (id, name, pattern, enabled, is_builtin) VALUES (?1, ?2, ?3, 1, 0)",
        params![id, name, pattern],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn delete_danger_rule(db: tauri::State<'_, Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM danger_rules WHERE id=?1 AND is_builtin=0",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_danger_rule(
    db: tauri::State<'_, Database>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE danger_rules SET enabled=?1 WHERE id=?2",
        params![enabled as i32, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== General Settings =====

#[tauri::command]
pub async fn get_general_settings(
    db: tauri::State<'_, Database>,
) -> Result<HashMap<String, String>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM general_settings")
        .map_err(|e| e.to_string())?;
    let settings = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub async fn save_general_settings(
    db: tauri::State<'_, Database>,
    settings: HashMap<String, String>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    for (key, value) in &settings {
        conn.execute(
            "INSERT INTO general_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, value],
        ).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_system_font_families() -> Result<Vec<FontFamilyInfo>, String> {
    let mut families: HashMap<String, BTreeSet<String>> = HashMap::new();
    collect_platform_font_families(&mut families);
    for dir in font_search_dirs() {
        collect_font_files(&dir, &mut families);
    }
    Ok(font_family_infos(families))
}

fn collect_platform_font_families(families: &mut HashMap<String, BTreeSet<String>>) {
    #[cfg(target_os = "macos")]
    collect_macos_font_families(families);
}

#[cfg(target_os = "macos")]
fn collect_macos_font_families(families: &mut HashMap<String, BTreeSet<String>>) {
    let Ok(output) = Command::new("/usr/bin/atsutil")
        .args(["fonts", "-list"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines().skip(1) {
        if let Some((family, style)) = parse_macos_font_name(line.trim()) {
            add_font_style(families, family, style);
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_font_name(name: &str) -> Option<(String, String)> {
    let normalized = name.replace('-', " ");
    let mut parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let mut style_parts = Vec::new();
    while parts.last().is_some_and(|part| is_font_style_token(part)) {
        if let Some(part) = parts.pop() {
            style_parts.push(part);
        }
    }
    style_parts.reverse();

    let family = parts.join(" ");
    if family.is_empty() {
        None
    } else {
        let style = if style_parts.is_empty() {
            "Regular".to_string()
        } else {
            style_parts.join(" ")
        };
        Some((family, style))
    }
}

#[cfg(target_os = "macos")]
fn is_font_style_token(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "regular"
            | "roman"
            | "bold"
            | "bolditalic"
            | "boldoblique"
            | "italic"
            | "oblique"
            | "medium"
            | "semibold"
            | "semibolditalic"
            | "semilight"
            | "semilightitalic"
            | "demibold"
            | "demibolditalic"
            | "extrabold"
            | "extrabolditalic"
            | "light"
            | "lightitalic"
            | "extralight"
            | "extralightitalic"
            | "ultralight"
            | "ultralightitalic"
            | "thin"
            | "black"
            | "heavy"
            | "book"
            | "condensed"
            | "expanded"
            | "narrow"
    )
}

fn font_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Fonts"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local_app_data).join("Microsoft/Windows/Fonts"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            dirs.push(home.join(".fonts"));
            dirs.push(home.join(".local/share/fonts"));
        }
    }

    dirs
}

fn collect_font_files(dir: &Path, families: &mut HashMap<String, BTreeSet<String>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, families);
            continue;
        }
        if is_font_file(&path) {
            if let Some((family, style)) = font_from_path(&path) {
                add_font_style(families, family, style);
            }
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "ttf" | "ttc" | "otf" | "otc")
    )
}

fn font_from_path(path: &Path) -> Option<(String, String)> {
    let stem = path.file_stem()?.to_string_lossy();
    let mut name = stem
        .replace(['_', '-'], " ")
        .replace("Nerd Font Complete", "Nerd Font")
        .replace("Nerd Font Mono", "Nerd Font Mono");

    let mut style = "Regular".to_string();
    for suffix in FONT_STYLE_SUFFIXES {
        if let Some(stripped) = name.strip_suffix(*suffix) {
            name = stripped.to_string();
            style = suffix.trim().to_string();
            break;
        }
    }

    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some((trimmed.to_string(), style))
    }
}

const FONT_STYLE_SUFFIXES: &[&str] = &[
    " Bold Italic",
    " BoldItalic",
    " Bold Oblique",
    " BoldOblique",
    " Extra Light Italic",
    " ExtraLightItalic",
    " Extra Light",
    " ExtraLight",
    " Ultra Light Italic",
    " UltraLightItalic",
    " Ultra Light",
    " UltraLight",
    " Semi Bold Italic",
    " SemiBoldItalic",
    " Semi Bold",
    " SemiBold",
    " Semibold",
    " Semi Light Italic",
    " SemiLightItalic",
    " Semi Light",
    " SemiLight",
    " Light Italic",
    " LightItalic",
    " Demi Bold Italic",
    " DemiBoldItalic",
    " Demi Bold",
    " DemiBold",
    " Extra Bold Italic",
    " ExtraBoldItalic",
    " Extra Bold",
    " ExtraBold",
    " Mono Regular",
    " Regular",
    " Bold",
    " Italic",
    " Medium",
    " Light",
    " Thin",
    " Black",
    " Heavy",
    " Condensed",
    " Expanded",
];

fn add_font_style(families: &mut HashMap<String, BTreeSet<String>>, family: String, style: String) {
    let normalized_family = strip_font_style_suffix(&family);
    let family = normalized_family.trim();
    if family.is_empty() {
        return;
    }
    let style = normalize_font_style(&style);
    families
        .entry(family.to_string())
        .or_default()
        .insert(style);
}

fn strip_font_style_suffix(family: &str) -> String {
    let original = family
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut normalized = original.replace(['-', '_'], " ");

    loop {
        let lower = normalized.to_ascii_lowercase();
        let Some(marker) = FONT_STYLE_SUFFIXES
            .iter()
            .map(|suffix| format!(" {}", suffix.trim().to_ascii_lowercase()))
            .find(|suffix| lower.ends_with(suffix))
        else {
            break;
        };
        if normalized.len() <= marker.len() {
            break;
        }
        normalized = normalized[..normalized.len() - marker.len()]
            .trim()
            .to_string();
    }

    if normalized.trim().is_empty() {
        original
    } else {
        normalized.trim().to_string()
    }
}

fn normalize_font_style(style: &str) -> String {
    let style = style.trim();
    if style.is_empty() {
        "Regular".to_string()
    } else {
        style.to_string()
    }
}

fn font_family_infos(families: HashMap<String, BTreeSet<String>>) -> Vec<FontFamilyInfo> {
    let mut infos: Vec<FontFamilyInfo> = families
        .into_iter()
        .filter_map(|(family, styles)| {
            let mut styles: Vec<String> = styles.into_iter().collect();
            if styles.is_empty() {
                styles.push("Regular".to_string());
            }
            styles.sort_by(|a, b| {
                font_style_rank(a)
                    .cmp(&font_style_rank(b))
                    .then_with(|| a.cmp(b))
            });
            Some(FontFamilyInfo { family, styles })
        })
        .collect();
    infos.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    infos
}

fn font_style_rank(style: &str) -> u8 {
    match style.to_ascii_lowercase().as_str() {
        "regular" | "roman" => 0,
        "medium" => 1,
        "semibold" | "semi bold" | "semilight" | "semi light" | "demibold" | "demi bold" => 2,
        "bold" => 3,
        "italic" | "oblique" | "lightitalic" | "light italic" => 4,
        "bold italic" | "bolditalic" | "semibolditalic" | "semi bold italic"
        | "semilightitalic" | "semi light italic" => 5,
        "light" => 6,
        "thin" => 7,
        _ => 8,
    }
}

#[tauri::command]
pub async fn export_database_backup(
    app: tauri::AppHandle,
    destination: String,
) -> Result<(), String> {
    let source = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("opsbatch.db");
    std::fs::copy(&source, &destination).map_err(|e| e.to_string())?;
    Ok(())
}
