use rusqlite::params;
use serde::Deserialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::security::shell_quote;
use crate::ssh;

#[derive(Deserialize)]
pub struct TransferRequest {
    pub direction: String,
    pub host_ids: Vec<String>,
    pub local_path: String,
    pub remote_path: String,
    pub remote_paths: Option<HashMap<String, String>>,
    pub timeout: Option<u64>,
}

fn resolve_firstdir(
    config: &ssh::SshConfig,
    parent_path: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let quoted_parent = shell_quote(parent_path)?;
    let cmd = format!("ls -1d {}/ 2>/dev/null | head -1", quoted_parent);
    let result = ssh::execute_command(config, &cmd, timeout_secs)?;
    let name = result.output.trim().trim_end_matches('/');
    let name = name.split('/').last().unwrap_or("");
    if name.is_empty() {
        Err(format!("no directories found under {}", parent_path))
    } else {
        Ok(name.to_string())
    }
}

fn resolve_remote_path_template(
    path_template: &str,
    host_name: &str,
    config: &ssh::SshConfig,
    timeout_secs: u64,
) -> Result<String, String> {
    let mut result = path_template.to_string();

    // {host} — hostname
    result = result.replace("{host}", host_name);

    // {firstdir:parent_path} — first directory under parent_path on remote host
    let re = regex::Regex::new(r"\{firstdir:([^}]+)\}").unwrap();
    for cap in re.captures_iter(path_template) {
        let full_match = cap.get(0).unwrap().as_str();
        let parent = cap.get(1).unwrap().as_str();
        match resolve_firstdir(config, parent, timeout_secs) {
            Ok(dir_name) => result = result.replace(full_match, &dir_name),
            Err(e) => return Err(e),
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn file_transfer(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    request: TransferRequest,
) -> Result<String, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let conn = db.pool.get().map_err(|e| e.to_string())?;

    let mut configs: Vec<(String, String, ssh::SshConfig)> = Vec::new();
    for hid in &request.host_ids {
        let (ip, port, auth_type, host_name, username, password, private_key, proxy_settings): (String, i32, String, String, String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT ip, port, auth_type, name, username, password, private_key, COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
                params![hid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
            )
            .map_err(|e| format!("host {} not found: {}", hid, e))?;

        let password = crate::commands::hosts::resolve_host_password(hid, password)?;
        let private_key = crate::commands::hosts::resolve_host_private_key(hid, private_key)?;

        configs.push((
            hid.clone(),
            host_name,
            ssh::SshConfig {
                host: ip,
                port: port as u16,
                username,
                auth_type,
                password,
                private_key,
                proxy: crate::commands::hosts::parse_host_proxy_settings(proxy_settings),
            },
        ));
    }
    drop(conn);

    let task_id_clone = task_id.clone();
    let direction = request.direction.clone();
    let local_path = request.local_path.clone();
    let remote_path = request.remote_path.clone();
    let remote_paths = request.remote_paths.clone();
    let timeout = request.timeout.unwrap_or(60);

    std::thread::spawn(move || {
        for (hid, host_name, config) in configs {
            let resolved_remote = match remote_paths.as_ref().and_then(|m| m.get(&hid)) {
                Some(p) => p.clone(),
                None => {
                    match resolve_remote_path_template(&remote_path, &host_name, &config, timeout) {
                        Ok(p) => p,
                        Err(e) => {
                            let record = serde_json::json!({
                                "hostId": hid,
                                "hostName": host_name,
                                "success": false,
                                "fileSize": 0,
                                "duration": 0,
                                "error": format!("路径解析失败: {}", e),
                            });
                            let _ =
                                app.emit(&format!("transfer:{}:progress", task_id_clone), record);
                            continue;
                        }
                    }
                }
            };

            let result = if direction == "upload" {
                ssh::upload_file(&config, &local_path, &resolved_remote, timeout)
            } else {
                let local_dir = format!("{}/{}", local_path, host_name);
                ssh::download_file(&config, &resolved_remote, &local_dir, timeout)
            };

            let (success, file_size, duration, error) = match result {
                Ok(r) => (r.success, r.file_size as i64, r.duration_ms as i64, None),
                Err(e) => (false, 0, 0, Some(e)),
            };

            let record = serde_json::json!({
                "hostId": hid,
                "hostName": host_name,
                "success": success,
                "fileSize": file_size,
                "duration": duration,
                "error": error,
            });

            let _ = app.emit(&format!("transfer:{}:progress", task_id_clone), record);
        }

        let _ = app.emit(
            &format!("transfer:{}:done", task_id_clone),
            serde_json::json!({
                "direction": direction,
            }),
        );
    });

    Ok(task_id)
}
