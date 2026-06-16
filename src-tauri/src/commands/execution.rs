use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::Database;
use crate::security::{clamp_execution_concurrency, truncate_output_lossy};
use crate::ssh;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionHistory {
    pub id: String,
    pub command: String,
    pub host_ids: String,
    pub host_count: i32,
    pub success_count: i32,
    pub fail_count: i32,
    pub started_at: String,
    pub completed_at: String,
    pub duration: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryDetail {
    pub host_id: String,
    pub host_name: String,
    pub status: String,
    pub output: String,
    pub exit_code: i32,
    pub duration: i64,
}

fn is_cancelled(app: &AppHandle, task_id: &str) -> bool {
    let db = app.state::<Database>();
    let Ok(conn) = db.pool.get() else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM execution_cancellations WHERE task_id=?1 LIMIT 1",
        params![task_id],
        |_| Ok(()),
    )
    .is_ok()
}

#[tauri::command]
pub async fn list_execution_history(
    db: tauri::State<'_, Database>,
) -> Result<Vec<ExecutionHistory>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, command, host_ids, host_count, success_count, fail_count, started_at, completed_at, duration FROM execution_history ORDER BY started_at DESC LIMIT 100"
    ).map_err(|e| e.to_string())?;
    let history = stmt
        .query_map([], |row| {
            Ok(ExecutionHistory {
                id: row.get(0)?,
                command: row.get(1)?,
                host_ids: row.get(2)?,
                host_count: row.get(3)?,
                success_count: row.get(4)?,
                fail_count: row.get(5)?,
                started_at: row.get(6)?,
                completed_at: row.get(7)?,
                duration: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(history)
}

#[tauri::command]
pub async fn get_execution_detail(
    db: tauri::State<'_, Database>,
    history_id: String,
) -> Result<Vec<HistoryDetail>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT host_id, host_name, status, output, exit_code, duration FROM execution_details WHERE history_id=?1 ORDER BY host_name"
    ).map_err(|e| e.to_string())?;
    let details = stmt
        .query_map(params![history_id], |row| {
            Ok(HistoryDetail {
                host_id: row.get(0)?,
                host_name: row.get(1)?,
                status: row.get(2)?,
                output: row.get(3)?,
                exit_code: row.get(4)?,
                duration: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(details)
}

#[tauri::command]
pub fn execute_command(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    task_id: String,
    host_ids: Vec<String>,
    command: String,
    concurrency: u32,
    timeout: u64,
    quick_action_id: Option<String>,
) -> Result<String, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;

    let mut configs: Vec<(String, String, ssh::SshConfig)> = Vec::new();
    for hid in &host_ids {
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
                proxy: crate::commands::hosts::resolve_host_proxy_settings(hid, proxy_settings)?,
            },
        ));
    }

    drop(conn);

    let task_id_arc: Arc<str> = Arc::from(task_id.as_str());
    let cmd_arc: Arc<str> = Arc::from(command.as_str());
    let host_count = host_ids.len() as i32;
    let host_ids_str = serde_json::to_string(&host_ids).unwrap_or_default();
    let qa_id_for_thread = quick_action_id.clone();

    // Save execution history (with optional quick_action_id)
    {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO execution_history (id, command, host_ids, host_count, success_count, fail_count, started_at, duration, quick_action_id) VALUES (?1, ?2, ?3, ?4, 0, 0, datetime('now','localtime'), 0, ?5)",
            params![task_id, command, host_ids_str, host_count, qa_id_for_thread],
        ).map_err(|e| e.to_string())?;
    }

    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut success_count = 0u32;
        let mut fail_count = 0u32;

        crate::commands::app_log::emit_log(
            &app,
            "info",
            "execution",
            &format!("Execute command [{}], {} host(s)", cmd_arc, host_count),
            "backend",
        );

        let max_concurrent = clamp_execution_concurrency(concurrency) as usize;
        let mut handles: VecDeque<
            std::thread::JoinHandle<(String, String, u32, String, i32, u64)>,
        > = VecDeque::new();
        let mut pending_db_writes: Vec<(String, String, u32, String, i32, u64)> = Vec::new();

        for (hid, host_name, config) in configs.into_iter() {
            if is_cancelled(&app, &task_id_arc) {
                fail_count += 1;
                let _ = app.emit(
                    &format!("exec:{}:output", task_id_arc),
                    serde_json::json!({
                        "hostId": hid,
                        "hostName": host_name,
                        "status": "failed",
                        "output": "[已取消]",
                        "exitCode": -1,
                        "duration": 0,
                    }),
                );
                continue;
            }

            let app_spawn = app.clone();
            let task_id = Arc::clone(&task_id_arc);
            let command = Arc::clone(&cmd_arc);

            if handles.len() >= max_concurrent {
                let old_handle = handles.pop_front().unwrap();
                if let Ok((host_id, hn, result_flag, output, exit_code, dur)) = old_handle.join() {
                    if result_flag == 1 {
                        success_count += 1;
                    } else {
                        fail_count += 1;
                    }
                    pending_db_writes.push((
                        host_id.clone(),
                        hn.clone(),
                        result_flag,
                        output.clone(),
                        exit_code,
                        dur,
                    ));
                    let _ = crate::commands::asciinema::write_asciinema_recording(
                        app.clone(),
                        task_id_arc.to_string(),
                        host_id,
                        hn,
                        cmd_arc.to_string(),
                        output,
                    );
                }
            }

            let handle = std::thread::spawn(move || {
                let _ = app_spawn.emit(&format!("exec:{}:start", task_id), &hid);
                let pool = app_spawn.state::<ssh::SshConnectionRegistry>();

                match pool.execute(&hid, &config, &command, timeout) {
                    Ok(output) => {
                        let output = truncate_output_lossy(output);
                        let _ = app_spawn.emit(
                            &format!("exec:{}:output", task_id),
                            serde_json::json!({
                                "hostId": hid,
                                "hostName": host_name,
                                "status": "success",
                                "output": output,
                                "exitCode": 0,
                                "duration": 0,
                            }),
                        );
                        (hid, host_name, 1u32, output, 0, 0u64)
                    }
                    Err(e) => {
                        let e = truncate_output_lossy(e);
                        let _ = app_spawn.emit(
                            &format!("exec:{}:output", task_id),
                            serde_json::json!({
                                "hostId": hid,
                                "hostName": host_name,
                                "status": "failed",
                                "output": e,
                                "exitCode": -1,
                                "duration": 0,
                            }),
                        );
                        (hid, host_name, 0u32, e, -1, 0u64)
                    }
                }
            });
            handles.push_back(handle);
        }

        for h in handles {
            if let Ok((host_id, hn, result_flag, output, exit_code, dur)) = h.join() {
                if result_flag == 1 {
                    success_count += 1;
                } else {
                    fail_count += 1;
                }
                pending_db_writes.push((
                    host_id.clone(),
                    hn.clone(),
                    result_flag,
                    output.clone(),
                    exit_code,
                    dur,
                ));
                let _ = crate::commands::asciinema::write_asciinema_recording(
                    app.clone(),
                    task_id_arc.to_string(),
                    host_id,
                    hn,
                    cmd_arc.to_string(),
                    output,
                );
            }
        }

        // Single DB lock acquisition for all detail inserts
        {
            let db = app.state::<Database>();
            // 单次事务批量写入,避免逐条 INSERT 触发独立 fsync
            if let Ok(mut conn) = db.pool.get() {
                if let Ok(tx) = conn.transaction() {
                    for (host_id, hn, result_flag, output, exit_code, dur) in pending_db_writes {
                        if let Err(e) = tx.execute(
                           "INSERT INTO execution_details (history_id, host_id, host_name, status, output, exit_code, duration) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                           rusqlite::params![&*task_id_arc, host_id, hn, if result_flag == 1 { "success" } else { "failed" }, output, exit_code, dur as i64],
                       ) {
                           eprintln!("[Exec] DB insert failed host={}: {}", host_id, e);
                       }
                    }
                    if let Err(e) = tx.commit() {
                        eprintln!("[Exec] DB transaction commit failed: {}", e);
                    }
                }
            };
        }

        let duration_ms = start.elapsed().as_millis() as i32;

        // Update history record
        {
            let db = app.state::<Database>();
            if let Ok(conn) = db.pool.get() {
                conn.execute(
                    "UPDATE execution_history SET success_count=?1, fail_count=?2, completed_at=datetime('now','localtime'), duration=?3 WHERE id=?4",
                    rusqlite::params![success_count, fail_count, duration_ms, &*task_id_arc],
                ).ok();
            };
        }

        // Update quick_action last_run_at / last_status if triggered from a quick action
        if let Some(ref qa_id) = quick_action_id {
            let qa_status = if fail_count == 0 {
                "success"
            } else if success_count == 0 {
                "failed"
            } else {
                "partial"
            };
            let db = app.state::<Database>();
            if let Ok(conn) = db.pool.get() {
                conn.execute(
                    "UPDATE quick_actions SET last_run_at=datetime('now','localtime'), last_status=?1 WHERE id=?2",
                    rusqlite::params![qa_status, qa_id],
                ).ok();
            };
        }

        let _ = app.emit(
            &format!("exec:{}:done", task_id_arc),
            serde_json::json!({
                "successCount": success_count,
                "failCount": fail_count,
                "duration": duration_ms,
            }),
        );

        let (level, summary) = if fail_count == 0 {
            (
                "success",
                format!(
                    "Command [{}]: all succeeded ({} host(s), {}ms)",
                    cmd_arc, success_count, duration_ms
                ),
            )
        } else if success_count == 0 {
            (
                "error",
                format!(
                    "Command [{}]: all failed ({} host(s), {}ms)",
                    cmd_arc, fail_count, duration_ms
                ),
            )
        } else {
            (
                "warn",
                format!(
                    "Command [{}]: {} succeeded, {} failed ({}ms)",
                    cmd_arc, success_count, fail_count, duration_ms
                ),
            )
        };
        crate::commands::app_log::emit_log(&app, level, "execution", &summary, "backend");
    });

    Ok(task_id)
}

#[tauri::command]
pub fn get_task_output(_task_id: String) -> Result<Vec<serde_json::Value>, String> {
    Ok(vec![])
}

#[tauri::command]
pub async fn cancel_execution(
    db: tauri::State<'_, Database>,
    task_id: String,
) -> Result<(), String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO execution_cancellations (task_id) VALUES (?1) ON CONFLICT(task_id) DO UPDATE SET cancelled_at=datetime('now','localtime')",
        params![task_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}
