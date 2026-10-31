//! Global log (`app_logs` table + live `global-log` events) and the plain-text diagnostics file.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::Core;

pub const GLOBAL_LOG_TOPIC: &str = "global-log";
const MAX_FRONTEND_LOG_MESSAGE_CHARS: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLogEntry {
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub message: String,
    pub origin: String,
}

/// Create the app_logs table.
pub fn init_app_logs_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            level TEXT NOT NULL,
            source TEXT NOT NULL,
            message TEXT NOT NULL,
            origin TEXT NOT NULL DEFAULT 'backend'
        );
        CREATE INDEX IF NOT EXISTS idx_app_logs_time ON app_logs(timestamp);",
    )
    .map_err(|e| e.to_string())?;

    // Prune old logs on startup — keep last 10000 rows
    conn.execute_batch(
        "DELETE FROM app_logs WHERE id NOT IN (
            SELECT id FROM app_logs ORDER BY id DESC LIMIT 10000
        );",
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Persist a log entry and broadcast it to every window.
pub fn emit_log(core: &Core, level: &str, source: &str, message: &str, origin: &str) {
    let timestamp = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let entry = AppLogEntry {
        timestamp: timestamp.clone(),
        level: level.to_string(),
        source: source.to_string(),
        message: message.to_string(),
        origin: origin.to_string(),
    };

    if let Ok(conn) = core.db.pool.get() {
        let _ = conn.execute(
            "INSERT INTO app_logs (timestamp, level, source, message, origin) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![timestamp, level, source, message, origin],
        );
    }
    core.emit(GLOBAL_LOG_TOPIC, &entry);
}

/// Recent entries in chronological order.
pub fn get_log_history(core: &Core, limit: Option<u32>) -> Result<Vec<AppLogEntry>, String> {
    let limit = limit.unwrap_or(500);
    let conn = core.db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT timestamp, level, source, message, origin FROM app_logs ORDER BY id DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![limit], |row| {
            Ok(AppLogEntry {
                timestamp: row.get(0)?,
                level: row.get(1)?,
                source: row.get(2)?,
                message: row.get(3)?,
                origin: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<AppLogEntry> = rows.filter_map(|r| r.ok()).collect();
    entries.reverse();
    Ok(entries)
}

pub fn clear_app_logs(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute("DELETE FROM app_logs", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn clear_log_history(core: &Core) -> Result<(), String> {
    let conn = core.db.pool.get().map_err(|e| e.to_string())?;
    clear_app_logs(&conn)
}

/// Log coming from the UI layer (was `emit_frontend_log`).
pub fn emit_frontend_log(core: &Core, level: &str, source: &str, message: &str) {
    let level = normalize_level(level);
    let source = normalize_source(source);
    let message = truncate_frontend_log_message(&sanitize_log_message(message));
    emit_log(core, &level, &source, &message, "frontend");
    // Warnings and errors from the UI layer are kept in the diagnostics file as well (it survives log clearing).
    if matches!(level.as_str(), "error" | "warn") {
        append_diagnostic_log(core.data_dir(), &format!("frontend.{source}"), &message);
    }
}

fn truncate_frontend_log_message(message: &str) -> String {
    message
        .chars()
        .take(MAX_FRONTEND_LOG_MESSAGE_CHARS)
        .collect::<String>()
}

fn normalize_level(level: &str) -> String {
    match level {
        "error" => "error",
        "warn" => "warn",
        "success" => "success",
        _ => "info",
    }
    .to_string()
}

fn normalize_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        "system".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

// ---- diagnostics file ----

fn redact_diagnostic_message(message: &str) -> String {
    sanitize_log_message(message)
        .replace(['\r', '\n'], " ")
        .chars()
        .take(2000)
        .collect()
}

const MAX_LOG_MESSAGE_CHARS: usize = 12000;

/// Mask bearer tokens, API keys, tokens and passwords before a message is stored or shown
/// (port of `sanitizeLogMessage` in `utils/globalLogger.ts`), then cap it at 12 000 characters.
pub fn sanitize_log_message(message: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<[(Regex, &str); 3]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            (Regex::new(r#"(?i)(Bearer\s+)[^\s"']+"#).unwrap(), "${1}***"),
            (Regex::new(r#"(?i)("(?:[^"]*)(?:api[_-]?key|apikey|token|password)[^"]*"\s*:\s*")[^"]*(")"#).unwrap(), "${1}***${2}"),
            (Regex::new(r#"(?i)((?:api[_-]?key|token|password)\s*[:=]\s*)[^\s,"']+"#).unwrap(), "${1}***"),
        ]
    });
    let mut out = message.to_string();
    for (re, replacement) in patterns {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out.chars().take(MAX_LOG_MESSAGE_CHARS).collect()
}

/// Append one line to `<data_dir>/opsbatch-diagnostics.log`.
pub fn append_diagnostic_log(data_dir: &Path, source: &str, message: &str) {
    if std::fs::create_dir_all(data_dir).is_err() {
        return;
    }
    let path = data_dir.join("opsbatch-diagnostics.log");
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let message = redact_diagnostic_message(message);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{timestamp} [{source}] {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_are_masked_in_log_messages() {
        assert_eq!(
            sanitize_log_message("Bearer abc123 token=secret password: hunter2 api_key = live-key"),
            "Bearer *** token=*** password: *** api_key = ***"
        );
        assert_eq!(
            sanitize_log_message(
                r#"{"vncPassword":"secret","proxyPassword":"proxy-secret","apiKey":"live-key"}"#
            ),
            r#"{"vncPassword":"***","proxyPassword":"***","apiKey":"***"}"#
        );
    }

    #[test]
    fn long_diagnostics_keep_their_tail_but_are_capped() {
        let long = format!("rdp.ai.parse\n{}\nend-marker", "x".repeat(6000));
        assert!(sanitize_log_message(&long).ends_with("end-marker"));
        assert_eq!(
            sanitize_log_message(&"y".repeat(20000)).chars().count(),
            12000
        );
    }

    #[test]
    fn frontend_logs_are_masked_persisted_and_written_to_the_diagnostics_file() {
        let core = Core::open_temp("frontend-log").unwrap();
        emit_frontend_log(&core, "error", "ui.test", "login failed password=hunter2");
        emit_frontend_log(&core, "info", "ui.test", "just info");
        let history = get_log_history(&core, Some(10)).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|e| e.origin == "frontend"));
        assert_eq!(history[0].message, "login failed password=***");
        let diag =
            std::fs::read_to_string(core.data_dir().join("opsbatch-diagnostics.log")).unwrap();
        assert!(
            diag.contains("[frontend.ui.test] login failed password=***")
                && !diag.contains("hunter2")
                && !diag.contains("just info")
        );
    }

    #[test]
    fn clear_app_logs_deletes_persisted_rows() {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        init_app_logs_table(&conn).expect("init app log table");
        conn.execute(
            "INSERT INTO app_logs (timestamp, level, source, message, origin) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["10:00:00.000", "info", "system", "hello", "backend"],
        )
        .expect("insert app log");

        clear_app_logs(&conn).expect("clear app logs");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM app_logs", [], |row| row.get(0))
            .expect("count");
        assert_eq!(0, count);
    }

    #[test]
    fn frontend_log_truncation_keeps_long_parser_diagnostics() {
        let message = format!("rdp.ai.parse\n{}\nend-marker", "x".repeat(6000));
        assert!(truncate_frontend_log_message(&message).ends_with("end-marker"));
    }

    #[test]
    fn emit_log_persists_and_broadcasts() {
        let core = Core::open_temp("log").unwrap();
        let rx = core.events.subscribe(GLOBAL_LOG_TOPIC);
        emit_log(&core, "info", "system", "hello", "backend");
        let ev = rx.try_recv().expect("event");
        let entry: AppLogEntry = ev.parse().unwrap();
        assert_eq!(entry.message, "hello");
        let history = get_log_history(&core, None).unwrap();
        assert!(history.iter().any(|e| e.message == "hello"));
        emit_frontend_log(&core, "bogus", "  ", "from ui");
        let last = get_log_history(&core, Some(1)).unwrap().pop().unwrap();
        assert_eq!(
            (
                last.level.as_str(),
                last.source.as_str(),
                last.origin.as_str()
            ),
            ("info", "system", "frontend")
        );
    }

    #[test]
    fn diagnostics_are_single_line() {
        let dir = std::env::temp_dir().join(format!("opsbatch-diag-{}", uuid::Uuid::new_v4()));
        append_diagnostic_log(&dir, "t", "a\nb\r\nc");
        let text = std::fs::read_to_string(dir.join("opsbatch-diagnostics.log")).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("[t] a b  c"));
    }
}
