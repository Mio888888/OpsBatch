use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use dashmap::DashMap;
use russh_sftp::client::fs::DirEntry;
use russh_sftp::protocol::FileAttributes;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::Database;
use crate::security::{shell_quote, MAX_SFTP_TREE_NODES};
use crate::ssh;

const SFTP_TRANSFER_CHUNK_SIZE: usize = 128 * 1024;

struct SftpSessionEntry {
    sftp: Mutex<russh_sftp::client::SftpSession>,
    lease: ssh::SharedSshChannelLease,
}

pub struct SftpManager {
    sessions: DashMap<String, SftpSessionEntry>,
}

impl SftpManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }
}

async fn run_on_blocking_thread<T, F>(operation: &'static str, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(task());
    });
    rx.await
        .map_err(|e| format!("{} thread failed: {}", operation, e))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<String>,
    pub permissions: Option<u32>,
}

fn load_host_config(
    db: &Database,
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
) -> Result<ssh::SshConfig, String> {
    if let Some(config) = pool.cached_config(host_id) {
        return Ok(config);
    }

    let conn = db.pool.get().map_err(|e| e.to_string())?;
    let (ip, port, auth_type, username, password, private_key, proxy_settings): (
        String,
        i32,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT ip, port, auth_type, username, password, private_key, COALESCE(proxy_settings, '{}') FROM hosts WHERE id=?1",
            rusqlite::params![host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(|e| format!("host not found: {}", e))?;
    drop(conn);

    let password = crate::commands::hosts::resolve_host_password(host_id, password)?;
    let private_key = crate::commands::hosts::resolve_host_private_key(host_id, private_key)?;

    Ok(ssh::SshConfig {
        host: ip,
        port: port as u16,
        username,
        auth_type,
        password,
        private_key,
        proxy: crate::commands::hosts::resolve_host_proxy_settings(host_id, proxy_settings)?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpCapabilities {
    pub can_write: bool,
    pub home_dir: String,
}

#[tauri::command]
pub async fn sftp_open(app: tauri::AppHandle, host_id: String) -> Result<SftpCapabilities, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result: Result<SftpCapabilities, String> = (|| {
            let db = app.state::<Database>();
            let manager = app.state::<SftpManager>();
            let pool = app.state::<ssh::SshConnectionRegistry>();

            if let Some(entry) = manager.sessions.get(&host_id) {
                let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
                let home = entry
                    .lease
                    .block_on(sftp.canonicalize("."))
                    .unwrap_or_else(|_| "/".to_string());
                return Ok(SftpCapabilities {
                    can_write: true,
                    home_dir: home,
                });
            }

            let config = load_host_config(&db, &pool, &host_id)?;
            let pooled_sftp = pool.open_sftp_session(&host_id, &config, 10)?;
            let ssh::PooledSftpSession { sftp, lease } = pooled_sftp;

            let home = lease
                .block_on(sftp.canonicalize("."))
                .unwrap_or_else(|_| "/".to_string());

            manager.sessions.insert(
                host_id,
                SftpSessionEntry {
                    sftp: Mutex::new(sftp),
                    lease,
                },
            );

            Ok(SftpCapabilities {
                can_write: true,
                home_dir: home,
            })
        })();
        let _ = tx.send(result);
    });

    rx.await
        .map_err(|e| format!("sftp open thread failed: {}", e))?
}

#[tauri::command]
pub fn sftp_close(manager: tauri::State<'_, SftpManager>, host_id: String) -> Result<(), String> {
    manager.sessions.remove(&host_id);
    Ok(())
}

#[tauri::command]
pub async fn sftp_warmup(app: tauri::AppHandle, host_id: String) -> Result<(), String> {
    let db = app.state::<Database>();
    let pool = app.state::<ssh::SshConnectionRegistry>();
    let config = load_host_config(&db, &pool, &host_id)?;
    let manager = app.state::<SftpManager>();

    if manager.sessions.contains_key(&host_id) {
        return Ok(());
    }

    // SSH uses its own tokio Runtime internally via block_on().
    // Run on a plain thread to avoid calling block_on from within
    // the Tauri Tokio runtime context (which would deadlock).
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let pool = app.state::<ssh::SshConnectionRegistry>();
        let manager = app.state::<SftpManager>();
        let result: Result<(), String> = (|| {
            let pooled_sftp = pool.open_sftp_session(&host_id, &config, 10)?;
            let ssh::PooledSftpSession { sftp, lease } = pooled_sftp;
            manager.sessions.insert(
                host_id,
                SftpSessionEntry {
                    sftp: Mutex::new(sftp),
                    lease,
                },
            );
            Ok(())
        })();
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| format!("sftp warmup thread failed: {}", e))?
}

fn format_remote_metadata(dir_entry: &DirEntry) -> FileEntry {
    let m = dir_entry.metadata();
    let name = dir_entry.file_name();
    let path = dir_entry.path();
    let is_dir = m.is_dir();
    let is_symlink = m.is_symlink();
    let size = m.len();
    let modified = m.modified().ok().and_then(|t| {
        let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
    });
    let permissions = {
        let p = m.permissions();
        let mut mode: u32 = 0;
        if p.owner_read {
            mode |= 0o400;
        }
        if p.owner_write {
            mode |= 0o200;
        }
        if p.owner_exec {
            mode |= 0o100;
        }
        if p.group_read {
            mode |= 0o040;
        }
        if p.group_write {
            mode |= 0o020;
        }
        if p.group_exec {
            mode |= 0o010;
        }
        if p.other_read {
            mode |= 0o004;
        }
        if p.other_write {
            mode |= 0o002;
        }
        if p.other_exec {
            mode |= 0o001;
        }
        Some(mode)
    };

    FileEntry {
        name,
        path: if path.is_empty() {
            "/".to_string()
        } else {
            path
        },
        is_dir,
        is_symlink,
        size,
        modified,
        permissions,
    }
}

fn format_remote_stat(name: String, path: String, m: &FileAttributes) -> FileEntry {
    let is_dir = m.is_dir();
    let is_symlink = m.is_symlink();
    let size = m.len();
    let modified = m.modified().ok().and_then(|t| {
        let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
    });
    let permissions = {
        let p = m.permissions();
        let mut mode: u32 = 0;
        if p.owner_read {
            mode |= 0o400;
        }
        if p.owner_write {
            mode |= 0o200;
        }
        if p.owner_exec {
            mode |= 0o100;
        }
        if p.group_read {
            mode |= 0o040;
        }
        if p.group_write {
            mode |= 0o020;
        }
        if p.group_exec {
            mode |= 0o010;
        }
        if p.other_read {
            mode |= 0o004;
        }
        if p.other_write {
            mode |= 0o002;
        }
        if p.other_exec {
            mode |= 0o001;
        }
        Some(mode)
    };

    FileEntry {
        name,
        path,
        is_dir,
        is_symlink,
        size,
        modified,
        permissions,
    }
}

#[tauri::command]
pub async fn sftp_list_dir(
    app: tauri::AppHandle,
    host_id: String,
    path: String,
) -> Result<Vec<FileEntry>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let manager = app.state::<SftpManager>();
        let result = list_remote_dir_blocking(&manager, &host_id, &path);
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| format!("sftp list thread failed: {}", e))?
}

fn list_remote_dir_blocking(
    manager: &SftpManager,
    host_id: &str,
    path: &str,
) -> Result<Vec<FileEntry>, String> {
    let entry = manager
        .sessions
        .get(host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;

    let read_dir = entry
        .lease
        .block_on(sftp.read_dir(path))
        .map_err(|e| format!("read_dir failed: {}", e))?;

    let items: Vec<DirEntry> = read_dir.collect();

    let mut result: Vec<FileEntry> = items
        .iter()
        .filter_map(|item| {
            let name = item.file_name();
            if name == "." || name == ".." {
                return None;
            }
            Some(format_remote_metadata(item))
        })
        .collect();

    result.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(result)
}

#[tauri::command]
pub fn sftp_stat(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    path: String,
) -> Result<FileEntry, String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    let metadata = entry
        .lease
        .block_on(sftp.metadata(&path))
        .map_err(|e| format!("stat failed: {}", e))?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    Ok(format_remote_stat(name, path, &metadata))
}

#[tauri::command]
pub async fn sftp_read_file(
    app: AppHandle,
    host_id: String,
    path: String,
    max_size: Option<u64>,
) -> Result<String, String> {
    run_on_blocking_thread("sftp read file", move || {
        let db = app.state::<Database>();
        let manager = app.state::<SftpManager>();
        let pool = app.state::<ssh::SshConnectionRegistry>();

        ensure_sftp_session(&db, &manager, &pool, &host_id)?;
        let entry = manager
            .sessions
            .get(&host_id)
            .ok_or("sftp session not found")?;
        let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;

        let metadata = entry
            .lease
            .block_on(sftp.metadata(&path))
            .map_err(|e| format!("stat failed: {}", e))?;
        let file_size = metadata.len();
        let limit = max_size.unwrap_or(10 * 1024 * 1024);
        if file_size > limit {
            return Err(format!(
                "文件过大 ({} bytes)，预览限制 {} bytes",
                file_size, limit
            ));
        }

        entry
            .lease
            .block_on(sftp.read(&path))
            .map(|bytes| BASE64.encode(&bytes))
            .map_err(|e| format!("read failed: {}", e))
    })
    .await
}

#[tauri::command]
pub fn sftp_mkdir(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    path: String,
) -> Result<(), String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    entry
        .lease
        .block_on(sftp.create_dir(&path))
        .map_err(|e| format!("mkdir failed: {}", e))
}

#[tauri::command]
pub fn sftp_rename(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    entry
        .lease
        .block_on(sftp.rename(&old_path, &new_path))
        .map_err(|e| format!("rename failed: {}", e))
}

#[tauri::command]
pub fn sftp_remove(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    path: String,
) -> Result<(), String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    entry
        .lease
        .block_on(sftp.remove_file(&path))
        .map_err(|e| format!("remove failed: {}", e))
}

#[tauri::command]
pub fn sftp_rmdir(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    path: String,
) -> Result<(), String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    entry
        .lease
        .block_on(sftp.remove_dir(&path))
        .map_err(|e| format!("rmdir failed: {}", e))
}

#[tauri::command]
pub async fn sftp_upload(
    app: AppHandle,
    host_id: String,
    local_path: String,
    remote_path: String,
    transfer_id: String,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app_for_thread = app.clone();
    std::thread::spawn(move || {
        let manager = app_for_thread.state::<SftpManager>();
        let result = upload_file_blocking(
            app_for_thread.clone(),
            &manager,
            &host_id,
            &local_path,
            &remote_path,
            &transfer_id,
        );
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| format!("upload thread failed: {}", e))?
}

fn upload_file_blocking(
    app: AppHandle,
    manager: &SftpManager,
    host_id: &str,
    local_path: &str,
    remote_path: &str,
    transfer_id: &str,
) -> Result<(), String> {
    let local = Path::new(local_path);
    if !local.exists() {
        return Err(format!("local file not found: {}", local_path));
    }

    let file_size = local.metadata().map(|m| m.len()).unwrap_or(0);

    let entry = manager
        .sessions
        .get(host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;

    let _ = app.emit(
        &format!("sftp-transfer:{}:start", transfer_id),
        serde_json::json!({ "fileSize": file_size }),
    );

    let start = Instant::now();
    let result: Result<(), String> = entry.lease.block_on(async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // 用 tokio async 读本地文件,避免在 async 块内同步阻塞 executor
        let mut local_file = tokio::fs::File::open(local)
            .await
            .map_err(|e| format!("open local: {}", e))?;
        let mut file = sftp
            .create(remote_path)
            .await
            .map_err(|e| format!("{}", e))?;
        let mut buffer = vec![0u8; SFTP_TRANSFER_CHUNK_SIZE];

        loop {
            let read_bytes = local_file
                .read(&mut buffer)
                .await
                .map_err(|e| format!("read local: {}", e))?;
            if read_bytes == 0 {
                break;
            }
            file.write_all(&buffer[..read_bytes])
                .await
                .map_err(|e| format!("{}", e))?;
        }

        file.flush().await.map_err(|e| format!("{}", e))?;
        file.shutdown().await.map_err(|e| format!("{}", e))?;
        Ok(())
    });

    match result {
        Ok(()) => {
            let _ = app.emit(
                &format!("sftp-transfer:{}:done", transfer_id),
                serde_json::json!({
                    "success": true,
                    "fileSize": file_size,
                    "durationMs": start.elapsed().as_millis() as u64,
                }),
            );
            Ok(())
        }
        Err(e) => {
            let _ = app.emit(
                &format!("sftp-transfer:{}:done", transfer_id),
                serde_json::json!({ "success": false, "error": e.to_string() }),
            );
            Err(format!("upload failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn sftp_download(
    app: AppHandle,
    host_id: String,
    remote_path: String,
    local_path: String,
    transfer_id: String,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app_for_thread = app.clone();
    std::thread::spawn(move || {
        let manager = app_for_thread.state::<SftpManager>();
        let result = download_file_blocking(
            app_for_thread.clone(),
            &manager,
            &host_id,
            &remote_path,
            &local_path,
            &transfer_id,
        );
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|e| format!("download thread failed: {}", e))?
}

fn download_file_blocking(
    app: AppHandle,
    manager: &SftpManager,
    host_id: &str,
    remote_path: &str,
    local_path: &str,
    transfer_id: &str,
) -> Result<(), String> {
    let entry = manager
        .sessions
        .get(host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;

    let metadata = entry
        .lease
        .block_on(sftp.metadata(remote_path))
        .map_err(|e| format!("stat failed: {}", e))?;
    let file_size = metadata.len();

    let local_target = PathBuf::from(local_path);
    if let Some(parent) = local_target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create local parent: {}", e))?;
    }

    let _ = app.emit(
        &format!("sftp-transfer:{}:start", transfer_id),
        serde_json::json!({ "fileSize": file_size }),
    );

    let start = Instant::now();
    let result: Result<(), String> = entry.lease.block_on(async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut remote_file = sftp.open(remote_path).await.map_err(|e| format!("{}", e))?;
        // 用 tokio async 写本地文件,避免在 async 块内同步阻塞 executor
        let mut local_file = tokio::fs::File::create(&local_target)
            .await
            .map_err(|e| format!("create local: {}", e))?;
        let mut buffer = vec![0u8; SFTP_TRANSFER_CHUNK_SIZE];

        loop {
            let read_bytes = remote_file
                .read(&mut buffer)
                .await
                .map_err(|e| format!("{}", e))?;
            if read_bytes == 0 {
                break;
            }
            local_file
                .write_all(&buffer[..read_bytes])
                .await
                .map_err(|e| format!("write local: {}", e))?;
        }

        local_file
            .flush()
            .await
            .map_err(|e| format!("flush local: {}", e))?;
        Ok(())
    });

    match result {
        Ok(()) => {
            let _ = app.emit(
                &format!("sftp-transfer:{}:done", transfer_id),
                serde_json::json!({
                    "success": true,
                    "fileSize": file_size,
                    "durationMs": start.elapsed().as_millis() as u64,
                }),
            );
            Ok(())
        }
        Err(e) => {
            let _ = app.emit(
                &format!("sftp-transfer:{}:done", transfer_id),
                serde_json::json!({ "success": false, "error": e.to_string() }),
            );
            Err(format!("download failed: {}", e))
        }
    }
}

#[tauri::command]
pub fn sftp_home_dir(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
) -> Result<String, String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    entry
        .lease
        .block_on(sftp.canonicalize("."))
        .map_err(|e| format!("home dir failed: {}", e))
}

#[tauri::command]
pub fn sftp_exists(
    manager: tauri::State<'_, SftpManager>,
    host_id: String,
    path: String,
) -> Result<bool, String> {
    let entry = manager
        .sessions
        .get(&host_id)
        .ok_or("sftp session not found")?;
    let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
    match entry.lease.block_on(sftp.try_exists(&path)) {
        Ok(exists) => Ok(exists),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub fn sftp_extract_archive(
    db: tauri::State<'_, Database>,
    pool: tauri::State<'_, crate::ssh::SshConnectionRegistry>,
    host_id: String,
    archive_path: String,
    target_dir: String,
) -> Result<String, String> {
    let ext = Path::new(&archive_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let archive_basename = Path::new(&archive_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("extracted");

    let base = if ext == "gz" || ext == "bz2" || ext == "xz" {
        Path::new(archive_basename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(archive_basename)
            .to_string()
    } else {
        archive_basename.to_string()
    };

    let target = if target_dir.is_empty() {
        let parent = Path::new(&archive_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("/tmp");
        format!("{}/{}", parent, base)
    } else {
        target_dir
    };

    let quoted_target = shell_quote(&target)?;
    let quoted_archive = shell_quote(&archive_path)?;
    let command = match ext.as_str() {
        "gz" | "tgz" => format!(
            "mkdir -p {} && tar xzf {} -C {}",
            quoted_target, quoted_archive, quoted_target
        ),
        "bz2" | "tbz2" => format!(
            "mkdir -p {} && tar xjf {} -C {}",
            quoted_target, quoted_archive, quoted_target
        ),
        "xz" | "txz" => format!(
            "mkdir -p {} && tar xJf {} -C {}",
            quoted_target, quoted_archive, quoted_target
        ),
        "tar" => format!(
            "mkdir -p {} && tar xf {} -C {}",
            quoted_target, quoted_archive, quoted_target
        ),
        "zip" => format!(
            "mkdir -p {} && unzip -o {} -d {}",
            quoted_target, quoted_archive, quoted_target
        ),
        "7z" => format!(
            "mkdir -p {} && 7z x {} -o{} -y",
            quoted_target, quoted_archive, quoted_target
        ),
        _ => return Err(format!("不支持的压缩格式: .{}", ext)),
    };

    let config = load_host_config(&db, &pool, &host_id)?;
    let output = pool.execute(&host_id, &config, &command, 60)?;
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<TreeNode>,
}

#[tauri::command]
pub async fn sftp_read_file_tree(
    app: AppHandle,
    host_id: String,
    path: String,
    max_depth: Option<u32>,
) -> Result<TreeNode, String> {
    run_on_blocking_thread("sftp read file tree", move || {
        let depth = max_depth.unwrap_or(3);
        let manager = app.state::<SftpManager>();
        let entry = manager
            .sessions
            .get(&host_id)
            .ok_or("sftp session not found")?;
        let sftp = entry.sftp.lock().map_err(|e| e.to_string())?;
        let name = Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let mut visited = 0usize;
        read_tree_node(&sftp, &entry.lease, &path, &name, depth, 0, &mut visited)
    })
    .await
}

fn read_tree_node(
    sftp: &russh_sftp::client::SftpSession,
    connection: &ssh::SharedSshChannelLease,
    path: &str,
    name: &str,
    max_depth: u32,
    current_depth: u32,
    visited: &mut usize,
) -> Result<TreeNode, String> {
    *visited += 1;
    if *visited > MAX_SFTP_TREE_NODES {
        return Err("目录树节点过多，请缩小范围后重试".to_string());
    }
    let metadata = connection
        .block_on(sftp.metadata(path))
        .map_err(|e| format!("stat {} failed: {}", path, e))?;
    let is_dir = metadata.is_dir();

    if !is_dir || current_depth >= max_depth {
        return Ok(TreeNode {
            name: name.to_string(),
            path: path.to_string(),
            is_dir,
            children: vec![],
        });
    }

    let items: Vec<DirEntry> = connection
        .block_on(sftp.read_dir(path))
        .map_err(|e| format!("read_dir {} failed: {}", path, e))?
        .collect();

    let mut entries: Vec<FileEntry> = items
        .iter()
        .filter_map(|item| {
            let item_name = item.file_name();
            if item_name == "." || item_name == ".." {
                return None;
            }
            Some(format_remote_metadata(item))
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let child_nodes: Vec<TreeNode> = entries
        .iter()
        .filter_map(|child| {
            read_tree_node(
                sftp,
                connection,
                &child.path,
                &child.name,
                max_depth,
                current_depth + 1,
                visited,
            )
            .ok()
        })
        .collect();

    Ok(TreeNode {
        name: name.to_string(),
        path: path.to_string(),
        is_dir: true,
        children: child_nodes,
    })
}

fn ensure_sftp_session(
    db: &Database,
    manager: &SftpManager,
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
) -> Result<(), String> {
    if let Some(entry) = manager.sessions.get(host_id) {
        if !entry.lease.is_closed() {
            eprintln!("[SFTP] Session for {} exists, reusing", host_id);
            return Ok(());
        }
        eprintln!(
            "[SFTP] Session for {} exists but connection is closed, removing stale session",
            host_id
        );
    } else {
        eprintln!("[SFTP] No session for {}, creating new one", host_id);
    }
    drop(manager.sessions.remove(host_id));
    let config = load_host_config(db, pool, host_id)?;
    eprintln!(
        "[SFTP] Opening SFTP session for {}@{}:{}",
        config.username, config.host, config.port
    );
    let pooled_sftp = pool.open_sftp_session(host_id, &config, 10)?;
    let ssh::PooledSftpSession { sftp, lease } = pooled_sftp;
    manager.sessions.insert(
        host_id.to_string(),
        SftpSessionEntry {
            sftp: Mutex::new(sftp),
            lease,
        },
    );
    eprintln!("[SFTP] Session for {} created successfully", host_id);
    Ok(())
}

#[tauri::command]
pub fn sftp_write_file(
    db: tauri::State<'_, Database>,
    manager: tauri::State<'_, SftpManager>,
    pool: tauri::State<'_, ssh::SshConnectionRegistry>,
    host_id: String,
    path: String,
    content: String,
) -> Result<(), String> {
    let data = content.into_bytes();
    eprintln!(
        "[SFTP] write_file host={} path={} size={}bytes",
        host_id,
        path,
        data.len()
    );

    let write_result = attempt_sftp_write(&db, &manager, &pool, &host_id, &path, &data);
    if write_result.is_ok() {
        eprintln!("[SFTP] write_file OK host={} path={}", host_id, path);
        return Ok(());
    }

    let first_error = write_result.unwrap_err();
    eprintln!("[SFTP] Write failed, reconnecting: {}", first_error);
    drop(manager.sessions.remove(&host_id));

    let retry_result = attempt_sftp_write(&db, &manager, &pool, &host_id, &path, &data);
    if retry_result.is_ok() {
        eprintln!("[SFTP] write_file retry OK host={} path={}", host_id, path);
    } else {
        eprintln!(
            "[SFTP] write_file retry also failed: {:?}",
            retry_result.as_ref().err()
        );
    }
    retry_result.map_err(|e| format!("{} (重试后仍失败: {})", first_error, e))
}

fn attempt_sftp_write(
    db: &Database,
    manager: &SftpManager,
    pool: &ssh::SshConnectionRegistry,
    host_id: &str,
    path: &str,
    data: &[u8],
) -> Result<(), String> {
    ensure_sftp_session(db, manager, pool, host_id)?;
    let entry = manager.sessions.get(host_id).ok_or_else(|| {
        eprintln!("[SFTP] session not found after ensure for {}", host_id);
        "sftp session not found".to_string()
    })?;
    let sftp = entry.sftp.lock().map_err(|e| {
        eprintln!("[SFTP] mutex lock failed for {}: {}", host_id, e);
        e.to_string()
    })?;
    eprintln!(
        "[SFTP] Starting write {}bytes to {} on host {}",
        data.len(),
        path,
        host_id
    );
    let write_result = entry.lease.block_on(async {
        use tokio::io::AsyncWriteExt;

        let mut file = sftp
            .create(path)
            .await
            .map_err(|e| format!("create: {}", e))?;
        file.write_all(data)
            .await
            .map_err(|e| format!("write_all: {}", e))?;
        file.flush().await.map_err(|e| format!("flush: {}", e))?;
        file.shutdown().await.map_err(|e| format!("close: {}", e))?;
        Ok::<(), String>(())
    });
    write_result.map_err(|e| {
        eprintln!("[SFTP] write failed: {}", e);
        format!("write failed: {}", e)
    })
}
