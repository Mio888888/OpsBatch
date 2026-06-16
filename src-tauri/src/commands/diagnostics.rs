use std::fs::OpenOptions;
use std::io::Write;

use tauri::{AppHandle, Manager};

fn redact_diagnostic_message(message: &str) -> String {
    message
        .replace(['\r', '\n'], " ")
        .chars()
        .take(2000)
        .collect()
}

fn diagnostics_log_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    Ok(app_data_dir.join("opsbatch-diagnostics.log"))
}

pub fn append_diagnostic_log(app: &AppHandle, source: &str, message: &str) {
    let Ok(path) = diagnostics_log_path(app) else {
        return;
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let message = redact_diagnostic_message(message);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{timestamp} [{source}] {message}");
    }
}

#[tauri::command]
pub fn write_diagnostic_log(app: AppHandle, source: String, message: String) {
    append_diagnostic_log(&app, &source, &message);
}
