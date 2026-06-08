use serde::{Deserialize, Serialize};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// Asciinema v2 format header
#[derive(Serialize, Deserialize)]
struct AsciinemaHeader {
    version: u32,
    width: u32,
    height: u32,
    timestamp: u64,
    env: AsciinemaEnv,
}

#[derive(Serialize, Deserialize)]
struct AsciinemaEnv {
    term: String,
    shell: String,
}

/// Write asciinema recording for an execution session
#[tauri::command]
pub fn write_asciinema_recording(
    app: AppHandle,
    history_id: String,
    host_id: String,
    _host_name: String,
    command: String,
    output: String,
) -> Result<String, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let recordings_dir = app_data_dir.join("recordings");
    fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    let filename = format!("{}_{}.cast", history_id, host_id);
    let filepath = recordings_dir.join(&filename);

    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Header
    let header = AsciinemaHeader {
        version: 2,
        width: 200,
        height: 50,
        timestamp: start_time,
        env: AsciinemaEnv {
            term: "xterm-256color".into(),
            shell: "/bin/bash".into(),
        },
    };

    let mut lines: Vec<String> = Vec::new();
    lines.push(serde_json::to_string(&header).map_err(|e| e.to_string())?);

    // Command line as first output event [time, "o", "content"]
    let cmd_line = format!("{}\r\n", command);
    lines.push(format!(
        "[0.000000, \"o\", {}]",
        serde_json::to_string(&cmd_line).map_err(|e| e.to_string())?
    ));

    // Output with incremental timing (simulate streaming at ~10ms per line)
    let output_lines: Vec<&str> = output.split('\n').collect();
    let mut elapsed = 0.05; // small delay after command prompt
    for line in &output_lines {
        let line_with_newline = format!("{}\r\n", line);
        lines.push(format!(
            "[{:.6}, \"o\", {}]",
            elapsed,
            serde_json::to_string(&line_with_newline).map_err(|e| e.to_string())?
        ));
        elapsed += 0.01 + (line.len() as f64 * 0.0001);
    }

    let content = lines.join("\n");
    fs::write(&filepath, &content).map_err(|e| e.to_string())?;

    Ok(filepath.to_string_lossy().to_string())
}

/// Read asciinema recording for replay
#[derive(Serialize)]
struct AsciinemaEvent {
    time: f64,
    event_type: String,
    content: String,
}

#[derive(Serialize)]
pub struct AsciinemaRecording {
    header: AsciinemaHeader,
    events: Vec<AsciinemaEvent>,
    filepath: String,
}

#[tauri::command]
pub fn read_asciinema_recording(
    app: AppHandle,
    history_id: String,
    host_id: String,
) -> Result<Option<AsciinemaRecording>, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let filepath = app_data_dir
        .join("recordings")
        .join(format!("{}_{}.cast", history_id, host_id));

    if !filepath.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let mut lines = content.lines();

    // Parse header
    let header_line = lines.next().ok_or("Empty recording file")?;
    let header: AsciinemaHeader = serde_json::from_str(header_line).map_err(|e| e.to_string())?;

    // Parse events
    let mut events = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        // Parse [time, "type", "content"]
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(arr_items) = arr.as_array() {
                if arr_items.len() >= 3 {
                    events.push(AsciinemaEvent {
                        time: arr_items[0].as_f64().unwrap_or(0.0),
                        event_type: arr_items[1].as_str().unwrap_or("o").into(),
                        content: serde_json::from_str::<String>(
                            arr_items[2].as_str().unwrap_or(""),
                        )
                        .unwrap_or_default(),
                    });
                }
            }
        }
    }

    Ok(Some(AsciinemaRecording {
        header,
        events,
        filepath: filepath.to_string_lossy().to_string(),
    }))
}

/// List available recordings for a history entry
#[derive(Serialize)]
pub struct RecordingMeta {
    host_id: String,
    host_name: String,
    filepath: String,
    duration: f64,
}

#[tauri::command]
pub fn list_recordings(app: AppHandle, history_id: String) -> Result<Vec<RecordingMeta>, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let recordings_dir = app_data_dir.join("recordings");
    if !recordings_dir.exists() {
        return Ok(Vec::new());
    }

    let prefix = format!("{}_", history_id);
    let mut recordings = Vec::new();

    for entry in fs::read_dir(&recordings_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with(&prefix) && fname.ends_with(".cast") {
            let content = fs::read_to_string(entry.path()).unwrap_or_default();
            let last_line = content.lines().last().unwrap_or("[]");
            let duration = serde_json::from_str::<serde_json::Value>(last_line)
                .ok()
                .and_then(|v| v.as_array()?.first()?.as_f64())
                .unwrap_or(0.0);

            let host_id = fname
                .trim_start_matches(&prefix)
                .trim_end_matches(".cast")
                .to_string();

            recordings.push(RecordingMeta {
                host_id,
                host_name: String::new(),
                filepath: entry.path().to_string_lossy().to_string(),
                duration,
            });
        }
    }

    Ok(recordings)
}
