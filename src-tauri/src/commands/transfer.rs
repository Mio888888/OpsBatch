use rusqlite::params;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::Database;
use crate::security::shell_quote;
use crate::ssh;

/// hosts 表查询的连接字段：(ip, port, auth_type, name, username, password, private_key, proxy_settings)
type HostConnectionRow = (
    String,
    i32,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Deserialize)]
pub struct TransferRequest {
    pub task_id: Option<String>,
    pub direction: String,
    pub host_ids: Vec<String>,
    pub local_path: String,
    pub remote_path: String,
    pub remote_paths: Option<HashMap<String, String>>,
    pub timeout: Option<u64>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TransferProgressRecord {
    host_id: String,
    host_name: String,
    success: bool,
    file_size: i64,
    duration: i64,
    error: Option<String>,
}

fn resolve_firstdir(
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
    config: &ssh::SshConfig,
    parent_path: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let quoted_parent = shell_quote(parent_path)?;
    let cmd = format!("ls -1d {}/ 2>/dev/null | head -1", quoted_parent);
    let output = pool.execute(host_id, config, &cmd, timeout_secs)?;
    let name = output.trim().trim_end_matches('/');
    let name = name.split('/').next_back().unwrap_or("");
    if name.is_empty() {
        Err(format!("no directories found under {}", parent_path))
    } else {
        Ok(name.to_string())
    }
}

fn resolve_remote_path_template(
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
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
        match resolve_firstdir(pool, host_id, config, parent, timeout_secs) {
            Ok(dir_name) => result = result.replace(full_match, &dir_name),
            Err(e) => return Err(e),
        }
    }

    Ok(result)
}

fn resolve_upload_target_path(local_path: &str, remote_path: &str) -> Result<String, String> {
    let normalized = remote_path.trim();
    if normalized.is_empty() {
        return Err("remote path is empty".to_string());
    }

    let local_name = Path::new(local_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid local file name: {}", local_path))?;

    if normalized.ends_with('/') {
        return Ok(format!("{}{}", normalized, local_name));
    }

    Ok(normalized.to_string())
}

fn upload_file_via_pool(
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
    config: &ssh::SshConfig,
    local_path: &str,
    remote_path: &str,
    timeout_secs: u64,
) -> Result<ssh::TransferResult, String> {
    let start = std::time::Instant::now();

    let local = Path::new(local_path);
    if !local.exists() {
        return Err(format!("local file not found: {}", local_path));
    }

    let upload_target = resolve_upload_target_path(local_path, remote_path)?;
    let file_size = local.metadata().map(|m| m.len()).unwrap_or(0);
    let data = fs::read(local).map_err(|e| format!("read local file failed: {}", e))?;

    let sftp_session = pool.open_sftp_session(host_id, config, timeout_secs)?;

    // 确保远程父目录存在，否则 SFTP write 会返回 Failure
    if let Some(parent) = Path::new(&upload_target).parent() {
        if !parent.as_os_str().is_empty() {
            let parent_str = parent.to_string_lossy();
            let quoted = shell_quote(&parent_str)?;
            let mkdir_cmd = format!("mkdir -p {}", quoted);
            if let Err(e) = pool.execute(host_id, config, &mkdir_cmd, timeout_secs) {
                // 非致命：父目录可能已存在，忽略错误继续尝试写入
                eprintln!("[transfer] mkdir -p {} failed for {}: {}", parent_str, host_id, e);
            }
        }
    }

    sftp_session.lease.block_on(async move {
        use tokio::io::AsyncWriteExt;

        let mut file = sftp_session
            .sftp
            .create(&upload_target)
            .await
            .map_err(|e| format!("create file failed: {}", e))?;
        file.write_all(&data)
            .await
            .map_err(|e| format!("write failed: {}", e))?;
        file.flush()
            .await
            .map_err(|e| format!("flush failed: {}", e))?;
        file.shutdown()
            .await
            .map_err(|e| format!("close file failed: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ssh::TransferResult {
            host: config.host.clone(),
            success: true,
            error: None,
            file_size,
            duration_ms,
        })
    })
}

fn download_file_via_pool(
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
    config: &ssh::SshConfig,
    remote_path: &str,
    local_dir: &str,
    timeout_secs: u64,
) -> Result<ssh::TransferResult, String> {
    let start = std::time::Instant::now();

    let sftp_session = pool.open_sftp_session(host_id, config, timeout_secs)?;
    sftp_session.lease.block_on(async {
        let metadata = sftp_session
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| format!("stat remote file failed: {}", e))?;
        let file_size = metadata.len();

        let data = sftp_session
            .sftp
            .read(remote_path)
            .await
            .map_err(|e| format!("read failed: {}", e))?;

        let dir = PathBuf::from(local_dir);
        fs::create_dir_all(&dir).ok();
        let file_name = Path::new(remote_path)
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("downloaded"));
        let local_file_path = dir.join(file_name);
        fs::write(&local_file_path, &data)
            .map_err(|e| format!("write local file failed: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ssh::TransferResult {
            host: config.host.clone(),
            success: true,
            error: None,
            file_size,
            duration_ms,
        })
    })
}

#[tauri::command]
pub fn file_transfer(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    request: TransferRequest,
) -> Result<String, String> {
    let task_id = request
        .task_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let conn = db.pool.get().map_err(|e| e.to_string())?;

    let mut configs: Vec<(String, String, ssh::SshConfig)> = Vec::new();
    for hid in &request.host_ids {
        let (ip, port, auth_type, host_name, username, password, private_key, proxy_settings): HostConnectionRow = conn
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
                proxy: crate::commands::hosts::resolve_host_proxy_settings(hid, proxy_settings)?,
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
        let pool = app.state::<ssh::SshConnectionRegistry>();
        let mut records: Vec<TransferProgressRecord> = Vec::new();

        for (hid, host_name, config) in configs {
            let resolved_remote = match remote_paths.as_ref().and_then(|m| m.get(&hid)) {
                Some(p) => p.clone(),
                None => {
                    match resolve_remote_path_template(
                        &pool,
                        &hid,
                        &remote_path,
                        &host_name,
                        &config,
                        timeout,
                    ) {
                        Ok(p) => p,
                        Err(e) => {
                            let record = TransferProgressRecord {
                                host_id: hid.clone(),
                                host_name: host_name.clone(),
                                success: false,
                                file_size: 0,
                                duration: 0,
                                error: Some(format!("路径解析失败: {}", e)),
                            };
                            records.push(record.clone());
                            let _ = app.emit(
                                &format!("transfer:{}:progress", task_id_clone),
                                &record,
                            );
                            continue;
                        }
                    }
                }
            };

            let result = if direction == "upload" {
                upload_file_via_pool(&pool, &hid, &config, &local_path, &resolved_remote, timeout)
            } else {
                let local_dir = format!("{}/{}", local_path, host_name);
                download_file_via_pool(&pool, &hid, &config, &resolved_remote, &local_dir, timeout)
            };

            let (success, file_size, duration, error) = match result {
                Ok(r) => (r.success, r.file_size as i64, r.duration_ms as i64, None),
                Err(e) => (false, 0, 0, Some(e)),
            };

            let record = TransferProgressRecord {
                host_id: hid.clone(),
                host_name: host_name.clone(),
                success,
                file_size,
                duration,
                error,
            };
            records.push(record.clone());
            let _ = app.emit(&format!("transfer:{}:progress", task_id_clone), &record);
        }

        let _ = app.emit(
            &format!("transfer:{}:done", task_id_clone),
            serde_json::json!({
                "direction": direction,
                "results": records,
            }),
        );
    });

    Ok(task_id)
}
