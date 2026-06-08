use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::Database;

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
pub fn emit_log(
    app: &AppHandle,
    level: &str,
    source: &str,
    message: &str,
    origin: &str,
) {
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
        if let Ok(conn) = db.conn.lock() {
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
pub fn get_log_history(
    app: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<AppLogEntry>, String> {
    let limit = limit.unwrap_or(500);
    let db = app.state::<Database>();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
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

/// Frontend can call this to emit a test log entry (used by global-log window on load).
#[tauri::command]
pub fn ping_log(app: AppHandle, message: String) {
    emit_log(&app, "info", "system", &message, "backend");
}
