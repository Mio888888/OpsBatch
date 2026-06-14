use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::Database;

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

/// Emit a structured log entry to all frontend windows and persist to DB.
pub fn emit_log(app: &AppHandle, level: &str, source: &str, message: &str, origin: &str) {
    let timestamp = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let entry = AppLogEntry {
        timestamp: timestamp.clone(),
        level: level.to_string(),
        source: source.to_string(),
        message: message.to_string(),
        origin: origin.to_string(),
    };

    // Persist to DB (best-effort, don't block the caller)
    if let Some(db) = app.try_state::<Database>() {
        let level = level.to_string();
        let source = source.to_string();
        let message = message.to_string();
        let origin = origin.to_string();
        if let Ok(conn) = db.pool.get() {
            let _ = conn.execute(
                "INSERT INTO app_logs (timestamp, level, source, message, origin) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![timestamp, level, source, message, origin],
            );
        }
    }

    // Broadcast to all frontend windows
    let _ = app.emit("global-log", &entry);
}

/// Return recent log entries from DB (for loading history).
#[tauri::command]
pub fn get_log_history(app: AppHandle, limit: Option<u32>) -> Result<Vec<AppLogEntry>, String> {
    let limit = limit.unwrap_or(500);
    let db = app.state::<Database>();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT timestamp, level, source, message, origin FROM app_logs ORDER BY id DESC LIMIT ?1",
        )
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
    entries.reverse(); // Return in chronological order
    Ok(entries)
}

pub fn clear_app_logs(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute("DELETE FROM app_logs", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn clear_log_history(app: AppHandle) -> Result<(), String> {
    let db = app.state::<Database>();
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    clear_app_logs(&conn)
}

/// Frontend can call this to emit a test log entry (used by global-log window on load).
#[tauri::command]
pub fn ping_log(app: AppHandle, message: String) {
    emit_log(&app, "info", "system", &message, "backend");
}

#[tauri::command]
pub fn emit_frontend_log(app: AppHandle, level: String, source: String, message: String) {
    let level = normalize_level(&level);
    let source = normalize_source(&source);
    let message = truncate_frontend_log_message(&message);

    emit_log(&app, &level, &source, &message, "frontend");
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

#[cfg(test)]
mod tests {
    use super::*;

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
            .expect("count app logs");
        assert_eq!(0, count);
    }

    #[test]
    fn frontend_log_truncation_keeps_long_parser_diagnostics() {
        let message = format!("rdp.ai.parse\n{}\nend-marker", "x".repeat(6000));

        let truncated = truncate_frontend_log_message(&message);

        assert!(truncated.ends_with("end-marker"));
    }
}
