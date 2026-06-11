use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

use crate::db::Database;
use crate::security::SECRET_PLACEHOLDER;
use crate::ssh::{self, SshConnectionRegistry};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: i32,
    pub auth_type: String,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub os: String,
    pub tags: String,
    pub group_id: Option<String>,
    pub remark: String,
    pub status: String,
    pub jump_chain: String,
    pub rdp_settings: String,
    pub proxy_settings: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateHost {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: i32,
    pub auth_type: String,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub os: String,
    pub tags: String,
    pub group_id: Option<String>,
    pub remark: Option<String>,
    pub jump_chain: Option<String>,
    pub rdp_settings: Option<String>,
    pub proxy_settings: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMonitorProcess {
    pub memory: String,
    pub cpu: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMonitorNetwork {
    pub interface: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMonitorFilesystem {
    pub path: String,
    pub used: String,
    pub available: String,
    pub total: String,
    pub percent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMonitorSnapshot {
    pub timestamp: u64,
    pub uptime: Option<String>,
    pub load_average: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_used_mb: Option<u64>,
    pub memory_total_mb: Option<u64>,
    pub swap_used_mb: Option<u64>,
    pub swap_total_mb: Option<u64>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub processes: Vec<HostMonitorProcess>,
    pub network: Option<HostMonitorNetwork>,
    pub networks: Vec<HostMonitorNetwork>,
    pub ping_ms: Option<f64>,
    pub filesystems: Vec<HostMonitorFilesystem>,
}

fn parse_first_f64(value: &str) -> Option<f64> {
    value.split_whitespace().next()?.parse::<f64>().ok()
}

struct HostConnectionInfo {
    config: ssh::SshConfig,
    os: String,
}

pub(crate) fn parse_host_proxy_settings(value: Option<String>) -> Option<ssh::ProxySettings> {
    value
        .and_then(|raw| serde_json::from_str::<ssh::ProxySettings>(&raw).ok())
        .filter(|settings| {
            settings.enabled && !settings.host.trim().is_empty() && settings.port > 0
        })
}

fn get_host_connection_info(
    db: &Database,
    pool: &crate::ssh::SshConnectionRegistry,
    id: &str,
) -> Result<HostConnectionInfo, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let (ip, port, auth_type, username, password, private_key, os, proxy_settings): (
        String,
        i32,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT ip, port, auth_type, username, password, private_key, os, COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|e| format!("host not found: {}", e))?;

    if let Some(config) = pool.cached_config(id) {
        return Ok(HostConnectionInfo { config, os });
    }

    let password = resolve_host_password(id, password)?;
    let private_key = resolve_host_private_key(id, private_key)?;

    Ok(HostConnectionInfo {
        config: ssh::SshConfig {
            host: ip,
            port: port as u16,
            username,
            auth_type,
            password,
            private_key,
            proxy: parse_host_proxy_settings(proxy_settings),
        },
        os,
    })
}

pub(crate) fn resolve_host_password(
    host_id: &str,
    stored: Option<String>,
) -> Result<Option<String>, String> {
    match stored.as_deref() {
        Some(SECRET_PLACEHOLDER) => crate::keychain::get_host_password(host_id)
            .map(Some)
            .map_err(|e| missing_host_secret_message("密码", host_id, e)),
        _ => Ok(stored),
    }
}

pub(crate) fn resolve_host_private_key(
    host_id: &str,
    stored: Option<String>,
) -> Result<Option<String>, String> {
    match stored.as_deref() {
        Some(SECRET_PLACEHOLDER) => crate::keychain::get_host_private_key(host_id)
            .map(Some)
            .map_err(|e| missing_host_secret_message("私钥", host_id, e)),
        _ => Ok(stored),
    }
}

fn missing_host_secret_message(
    label: &str,
    host_id: &str,
    error: crate::keychain::SecretError,
) -> String {
    match error {
        crate::keychain::SecretError::Missing => format!(
            "主机 {} 的{}未在系统钥匙串中找到，请重新编辑主机并保存凭据。",
            host_id, label
        ),
        other => other.to_string(),
    }
}

fn store_host_secrets(
    host_id: &str,
    password: Option<String>,
    private_key: Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    let stored_password = match password.filter(|value| !value.is_empty()) {
        Some(value) => {
            crate::keychain::store_host_password(host_id, &value)?;
            Some(SECRET_PLACEHOLDER.to_string())
        }
        None => None,
    };
    let stored_private_key = match private_key.filter(|value| !value.is_empty()) {
        Some(value) => {
            crate::keychain::store_host_private_key(host_id, &value)?;
            Some(SECRET_PLACEHOLDER.to_string())
        }
        None => None,
    };
    Ok((stored_password, stored_private_key))
}

fn mask_secret_for_frontend(value: Option<String>) -> Option<String> {
    value
        .filter(|secret| !secret.is_empty())
        .map(|_| SECRET_PLACEHOLDER.to_string())
}

fn secret_state(value: Option<&str>) -> String {
    match value {
        None => "none".to_string(),
        Some("") => "empty".to_string(),
        Some(SECRET_PLACEHOLDER) => "placeholder".to_string(),
        Some(value) => format!("provided(len={})", value.len()),
    }
}

fn stored_host_secret_for_update(
    app: &tauri::AppHandle,
    host_id: &str,
    label: &str,
    current_stored: Option<String>,
    incoming: Option<String>,
    store: fn(&str, &str) -> Result<(), String>,
    read: fn(&str) -> Result<String, crate::keychain::SecretError>,
) -> Result<Option<String>, String> {
    match incoming.as_deref() {
        None | Some("") | Some(SECRET_PLACEHOLDER) => {
            if current_stored.as_deref() == Some(SECRET_PLACEHOLDER) {
                match read(host_id) {
                    Ok(_) => {}
                    Err(error) => {
                        crate::commands::app_log::emit_log(
                            app,
                            "error",
                            "host-secret",
                            &format!(
                                "keychain entry missing/unreadable host={} field={} error={}",
                                host_id, label, error
                            ),
                            "backend",
                        );
                        return Err(missing_host_secret_message(label, host_id, error));
                    }
                }
            }
            Ok(current_stored)
        }
        Some(value) => {
            crate::commands::app_log::emit_log(
                app,
                "info",
                "host-secret",
                &format!(
                    "storing new keychain secret host={} field={} len={}",
                    host_id,
                    label,
                    value.len()
                ),
                "backend",
            );
            store(host_id, value).map_err(|error| {
                crate::commands::app_log::emit_log(
                    app,
                    "error",
                    "host-secret",
                    &format!(
                        "keychain store failed host={} field={} error={}",
                        host_id, label, error
                    ),
                    "backend",
                );
                error
            })?;
            crate::commands::app_log::emit_log(
                app,
                "info",
                "host-secret",
                &format!("keychain store succeeded host={} field={}", host_id, label),
                "backend",
            );
            Ok(Some(SECRET_PLACEHOLDER.to_string()))
        }
    }
}

fn is_windows_host(os: &str) -> bool {
    os.trim().eq_ignore_ascii_case("windows")
}

fn host_status_probe_port(default_port: i32, rdp_settings: &str) -> i32 {
    let vnc_settings = crate::commands::vnc::parse_vnc_settings(rdp_settings);
    if vnc_settings.protocol.as_deref() == Some("vnc") {
        return crate::commands::vnc::vnc_port_from_settings(rdp_settings, default_port) as i32;
    }
    default_port
}

fn section_lines(output: &str, name: &str) -> Vec<String> {
    let marker = format!("__{}__", name);
    let mut collecting = false;
    let mut lines = Vec::new();

    for line in output.lines() {
        if line.starts_with("__") && line.ends_with("__") {
            collecting = line == marker;
            continue;
        }
        if collecting {
            lines.push(line.trim().to_string());
        }
    }

    lines
}

fn section_first(output: &str, name: &str) -> Option<String> {
    section_lines(output, name)
        .into_iter()
        .find(|line| !line.is_empty())
}

fn parse_processes(lines: Vec<String>) -> Vec<HostMonitorProcess> {
    lines
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let rss_kb = parts.next()?.parse::<u64>().ok()?;
            let cpu = parts.next()?.to_string();
            let command = parts.collect::<Vec<_>>().join(" ");
            if command.is_empty() {
                return None;
            }
            let memory = if rss_kb >= 1024 {
                format!("{:.1}M", rss_kb as f64 / 1024.0)
            } else {
                format!("{}K", rss_kb)
            };
            Some(HostMonitorProcess {
                memory,
                cpu,
                command,
            })
        })
        .collect()
}

fn parse_filesystems(lines: Vec<String>) -> Vec<HostMonitorFilesystem> {
    lines
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.split('|');
            Some(HostMonitorFilesystem {
                path: parts.next()?.to_string(),
                used: parts.next()?.to_string(),
                available: parts.next()?.to_string(),
                total: parts.next()?.to_string(),
                percent: parts.next()?.to_string(),
            })
        })
        .collect()
}

fn parse_networks(lines: Vec<String>) -> Vec<HostMonitorNetwork> {
    lines
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some(HostMonitorNetwork {
                interface: parts.next()?.to_string(),
                rx_bytes: parts.next()?.parse::<u64>().ok()?,
                tx_bytes: parts.next()?.parse::<u64>().ok()?,
            })
        })
        .collect()
}

fn parse_memory(output: &str, field: &str) -> Option<u64> {
    section_lines(output, "MEMORY")
        .into_iter()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            if parts.next()? == field {
                return parts.next()?.parse::<u64>().ok();
            }
            None
        })
}

fn measure_ping_ms(host: &str) -> Option<f64> {
    #[cfg(target_os = "windows")]
    use std::os::windows::process::CommandExt;

    let mut command = std::process::Command::new("ping");

    #[cfg(target_os = "windows")]
    command
        .args(["-n", "1", "-w", "3000", host])
        .creation_flags(CREATE_NO_WINDOW);

    #[cfg(target_os = "macos")]
    command.args(["-c", "1", "-W", "3000", host]);

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    command.args(["-c", "1", "-W", "3", host]);

    let output = command.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    stdout.lines().find_map(|line| {
        if let Some(value) = parse_ping_value_after(line, "time=") {
            return Some(value);
        }
        parse_ping_value_after(line, "时间=")
    })
}

fn parse_ping_value_after(line: &str, marker: &str) -> Option<f64> {
    let start = line.find(marker)? + marker.len();
    let rest = line[start..].trim_start();
    let value = rest
        .trim_start_matches('<')
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .next()?;

    value.parse::<f64>().ok()
}

fn empty_monitor_snapshot(
    timestamp: u64,
    os: Option<String>,
    ping_ms: Option<f64>,
) -> HostMonitorSnapshot {
    HostMonitorSnapshot {
        timestamp,
        uptime: None,
        load_average: None,
        cpu_percent: None,
        memory_used_mb: None,
        memory_total_mb: None,
        swap_used_mb: None,
        swap_total_mb: None,
        os,
        kernel: None,
        processes: Vec::new(),
        network: None,
        networks: Vec::new(),
        ping_ms,
        filesystems: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfoResult {
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub memory_total_gb: f64,
    pub disk_usage: f64,
    pub disk_total_gb: f64,
    pub os_name: String,
    pub kernel: String,
    pub uptime: String,
}

#[tauri::command]
pub async fn get_host_system_info(
    app: tauri::AppHandle,
    id: String,
) -> Result<SystemInfoResult, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let db = app.state::<Database>();
        let pool = app.state::<SshConnectionRegistry>();
        let result = inner_get_host_monitor_snapshot(&db, &pool, &id);
        let _ = tx.send(result);
    });
    let snapshot = rx
        .await
        .map_err(|e| format!("system info task failed: {}", e))??;
    let memory_usage = match (snapshot.memory_used_mb, snapshot.memory_total_mb) {
        (Some(used), Some(total)) if total > 0 => used as f64 * 100.0 / total as f64,
        _ => 0.0,
    };
    let memory_total_gb = snapshot
        .memory_total_mb
        .map(|value| value as f64 / 1024.0)
        .unwrap_or(0.0);
    let (disk_usage, disk_total_gb) = snapshot
        .filesystems
        .first()
        .map(|filesystem| {
            let usage = filesystem
                .percent
                .trim_end_matches('%')
                .parse::<f64>()
                .unwrap_or(0.0);
            (usage, 0.0)
        })
        .unwrap_or((0.0, 0.0));

    Ok(SystemInfoResult {
        cpu_usage: snapshot.cpu_percent.unwrap_or(0.0),
        memory_usage,
        memory_total_gb,
        disk_usage,
        disk_total_gb,
        os_name: snapshot.os.unwrap_or_default(),
        kernel: snapshot.kernel.unwrap_or_default(),
        uptime: snapshot.uptime.unwrap_or_default(),
    })
}

fn inner_get_host_monitor_snapshot(
    db: &Database,
    pool: &SshConnectionRegistry,
    id: &str,
) -> Result<HostMonitorSnapshot, String> {
    let HostConnectionInfo { config, os } = get_host_connection_info(db, pool, id)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;

    let ping_ms = measure_ping_ms(&config.host);
    if is_windows_host(&os) {
        return Ok(empty_monitor_snapshot(
            timestamp,
            Some("Windows".to_string()),
            ping_ms,
        ));
    }

    let command = r#"
printf '__UPTIME__\n'; uptime -p 2>/dev/null || awk '{print int($1/86400) " 天"}' /proc/uptime 2>/dev/null
printf '__LOAD__\n'; awk '{print $1", "$2", "$3}' /proc/loadavg 2>/dev/null
printf '__CPU__\n'; (head -n1 /proc/stat; sleep 0.2; head -n1 /proc/stat) 2>/dev/null | awk 'NR==1 {u=$2+$4; t=$2+$4+$5} NR==2 {u2=$2+$4; t2=$2+$4+$5; if (t2>t) printf "%.1f\n", (u2-u)*100/(t2-t);}'
printf '__MEMORY__\n'; free -m 2>/dev/null | awk '/^Mem:/ {print "mem_used "$3; print "mem_total "$2} /^Swap:/ {print "swap_used "$3; print "swap_total "$2}'
printf '__OS__\n'; . /etc/os-release 2>/dev/null; printf '%s\n' "${PRETTY_NAME:-$(uname -s)}"
printf '__KERNEL__\n'; uname -r 2>/dev/null
printf '__NETWORK__\n'; awk '$1 ~ /:/ {iface=$1; gsub(":", "", iface); if (iface != "lo") print iface, $2, $10}' /proc/net/dev 2>/dev/null
printf '__PROCESSES__\n'; ps -eo rss=,pcpu=,comm= --sort=-rss 2>/dev/null | head -n 6
printf '__FILESYSTEMS__\n'; df -hP 2>/dev/null | awk 'NR>1 {print $6 "|" $3 "|" $4 "|" $2 "|" $5}' | head -n 8
"#;
    let output = pool.execute(id, &config, command, 10).unwrap_or_default();
    let networks = parse_networks(section_lines(&output, "NETWORK"));
    let network = networks.first().cloned();

    Ok(HostMonitorSnapshot {
        timestamp,
        uptime: section_first(&output, "UPTIME"),
        load_average: section_first(&output, "LOAD"),
        cpu_percent: section_first(&output, "CPU").and_then(|value| parse_first_f64(&value)),
        memory_used_mb: parse_memory(&output, "mem_used"),
        memory_total_mb: parse_memory(&output, "mem_total"),
        swap_used_mb: parse_memory(&output, "swap_used"),
        swap_total_mb: parse_memory(&output, "swap_total"),
        os: section_first(&output, "OS"),
        kernel: section_first(&output, "KERNEL"),
        processes: parse_processes(section_lines(&output, "PROCESSES")),
        network,
        networks,
        ping_ms,
        filesystems: parse_filesystems(section_lines(&output, "FILESYSTEMS")),
    })
}

#[tauri::command]
pub async fn get_host_monitor_snapshot(
    app: tauri::AppHandle,
    id: String,
) -> Result<HostMonitorSnapshot, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let db = app.state::<Database>();
        let pool = app.state::<SshConnectionRegistry>();
        let result = inner_get_host_monitor_snapshot(&db, &pool, &id);
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| format!("monitor task failed: {}", e))?
}
#[tauri::command]
pub async fn list_hosts(db: tauri::State<'_, Database>) -> Result<Vec<Host>, String> {
    let conn = db.conn.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, name, ip, port, auth_type, username, password, private_key, os, tags, group_id, remark, status, jump_chain, COALESCE(rdp_settings, '{}'), COALESCE(proxy_settings, '{}'), created_at, updated_at FROM hosts ORDER BY name"
        ).map_err(|e| e.to_string())?;

        let hosts = stmt
            .query_map([], |row| {
                Ok(Host {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    ip: row.get(2)?,
                    port: row.get(3)?,
                    auth_type: row.get(4)?,
                    username: row.get(5)?,
                    password: mask_secret_for_frontend(row.get(6)?),
                    private_key: mask_secret_for_frontend(row.get(7)?),
                    os: row.get(8)?,
                    tags: row.get(9)?,
                    group_id: row.get(10)?,
                    remark: row.get(11)?,
                    status: row.get(12)?,
                    jump_chain: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "[]".to_string()),
                    rdp_settings: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "{}".to_string()),
                    proxy_settings: row.get::<_, Option<String>>(15)?.unwrap_or_else(|| "{}".to_string()),
                    created_at: row.get(16)?,
                    updated_at: row.get(17)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(hosts)
    }).await.map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
pub struct NewHost {
    pub name: String,
    pub ip: String,
    pub port: Option<i32>,
    pub auth_type: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub os: Option<String>,
    pub tags: Option<String>,
    pub group_id: Option<String>,
    pub remark: Option<String>,
    pub jump_chain: Option<String>,
    pub rdp_settings: Option<String>,
    pub proxy_settings: Option<String>,
}

#[tauri::command]
pub async fn add_host(db: tauri::State<'_, Database>, host: NewHost) -> Result<String, String> {
    let conn = db.conn.clone();
    tokio::task::spawn_blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        let (password, private_key) = store_host_secrets(&id, host.password, host.private_key)?;
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO hosts (id, name, ip, port, auth_type, username, password, private_key, os, tags, group_id, remark, jump_chain, rdp_settings, proxy_settings) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                host.name,
                host.ip,
                host.port.unwrap_or(22),
                host.auth_type.unwrap_or_else(|| "password".into()),
                host.username.unwrap_or_else(|| "root".into()),
                password,
                private_key,
                host.os.unwrap_or_else(|| "linux".into()),
                host.tags.unwrap_or_else(|| "[]".into()),
                host.group_id,
                host.remark.unwrap_or_default(),
                host.jump_chain.unwrap_or_else(|| "[]".into()),
                host.rdp_settings.unwrap_or_else(|| "{}".into()),
                host.proxy_settings.unwrap_or_else(|| "{}".into()),
            ],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_host(
    app: tauri::AppHandle,
    db: tauri::State<'_, Database>,
    host: UpdateHost,
) -> Result<(), String> {
    let conn = db.conn.clone();
    tokio::task::spawn_blocking(move || {
        crate::commands::app_log::emit_log(
            &app,
            "info",
            "host-secret",
            &format!(
                "update_host start host={} auth_type={} password={} private_key={}",
                host.id,
                host.auth_type,
                secret_state(host.password.as_deref()),
                secret_state(host.private_key.as_deref())
            ),
            "backend",
        );
        let (current_password, current_private_key): (Option<String>, Option<String>) = {
            let conn = conn.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT password, private_key FROM hosts WHERE id=?1",
                params![host.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| format!("host {} not found: {}", host.id, e))?
        };

        let password = stored_host_secret_for_update(
            &app,
            &host.id,
            "密码",
            current_password,
            host.password,
            crate::keychain::store_host_password,
            crate::keychain::get_host_password,
        )?;

        let private_key = stored_host_secret_for_update(
            &app,
            &host.id,
            "私钥",
            current_private_key,
            host.private_key,
            crate::keychain::store_host_private_key,
            crate::keychain::get_host_private_key,
        )?;

        {
            let conn = conn.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE hosts SET name=?1, ip=?2, port=?3, auth_type=?4, username=?5, password=?6, private_key=?7, os=?8, tags=?9, group_id=?10, remark=?11, jump_chain=?12, rdp_settings=?13, proxy_settings=?14, updated_at=datetime('now','localtime') WHERE id=?15",
                params![host.name, host.ip, host.port, host.auth_type, host.username, password, private_key, host.os, host.tags, host.group_id, host.remark, host.jump_chain.as_deref().unwrap_or("[]"), host.rdp_settings.as_deref().unwrap_or("{}"), host.proxy_settings.as_deref().unwrap_or("{}"), host.id],
            ).map_err(|e| e.to_string())?;
        }
        crate::commands::app_log::emit_log(
            &app,
            "info",
            "host-secret",
            &format!("update_host db update succeeded host={}", host.id),
            "backend",
        );
        app.state::<crate::ssh::SshConnectionRegistry>()
            .forget_config(&host.id);
        app.state::<crate::ssh::SshConnectionRegistry>()
            .remove_connection(&host.id);
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_host(
    app: tauri::AppHandle,
    db: tauri::State<'_, Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.conn.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM hosts WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        let _ = crate::keychain::delete_host_password(&id);
        let _ = crate::keychain::delete_host_private_key(&id);
        app.state::<crate::ssh::SshConnectionRegistry>()
            .forget_config(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn check_host_status(db: tauri::State<'_, Database>, id: String) -> Result<String, String> {
    let (ip, port, rdp_settings) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let result: (String, i32, String) = conn
            .query_row(
                "SELECT ip, port, COALESCE(rdp_settings, '{}') FROM hosts WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        result
    };

    let probe_port = host_status_probe_port(port, &rdp_settings);
    let addr = format!("{}:{}", ip, probe_port);
    let status = match addr.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(socket_addr) = addrs.next() {
                if TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5)).is_ok() {
                    "online"
                } else {
                    "offline"
                }
            } else {
                "offline"
            }
        }
        Err(_) => "offline",
    };

    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE hosts SET status=?1 WHERE id=?2",
            params![status, id],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(status.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_status_probe_port_uses_vnc_default_port() {
        assert_eq!(5900, host_status_probe_port(22, r#"{"protocol":"vnc"}"#));
    }

    #[test]
    fn host_status_probe_port_uses_vnc_custom_port() {
        assert_eq!(
            5902,
            host_status_probe_port(22, r#"{"protocol":"vnc","vncPort":5902}"#)
        );
    }

    #[test]
    fn host_status_probe_port_keeps_regular_host_port() {
        assert_eq!(3389, host_status_probe_port(3389, r#"{"protocol":"rdp"}"#));
    }
}
