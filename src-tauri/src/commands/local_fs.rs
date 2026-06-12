use std::fs;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::path::{Path, PathBuf};

use dashmap::DashMap;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::commands::sftp::FileEntry;
use crate::security::reject_suspicious_local_path;

pub struct LocalFsManager {
    roots: DashMap<String, PathBuf>,
}

impl LocalFsManager {
    pub fn new() -> Self {
        Self {
            roots: DashMap::new(),
        }
    }

    pub fn authorize_root(&self, path: impl Into<PathBuf>) -> Result<String, String> {
        let path = path.into();
        let canonical = canonicalize_existing_or_parent(&path)?;
        let token = uuid::Uuid::new_v4().to_string();
        self.roots.insert(token.clone(), canonical);
        Ok(token)
    }

    pub fn is_authorized(&self, path: &str) -> Result<bool, String> {
        reject_suspicious_local_path(path)?;
        let candidate = canonicalize_existing_or_parent(Path::new(path))?;
        Ok(self
            .roots
            .iter()
            .any(|root| candidate.starts_with(root.value())))
    }

    fn ensure_authorized(&self, path: &str) -> Result<(), String> {
        reject_suspicious_local_path(path)?;
        let candidate = canonicalize_existing_or_parent(Path::new(path))?;
        if self
            .roots
            .iter()
            .any(|root| candidate.starts_with(root.value()))
        {
            return Ok(());
        }
        Err("local path is outside authorized roots".to_string())
    }
}

#[tauri::command]
pub async fn local_authorize_directory(
    app: AppHandle,
    manager: tauri::State<'_, LocalFsManager>,
) -> Result<Option<String>, String> {
    let selected = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok());

    let Some(path) = selected else {
        return Ok(None);
    };

    let canonical = canonicalize_existing_or_parent(&path)?;
    manager.authorize_root(&canonical)?;
    Ok(Some(canonical.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn local_is_authorized(
    manager: tauri::State<'_, LocalFsManager>,
    path: String,
) -> Result<bool, String> {
    manager.is_authorized(&path)
}

fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|e| format!("canonicalize: {}", e));
    }
    let parent = path.parent().ok_or("path has no parent")?;
    parent
        .canonicalize()
        .map_err(|e| format!("canonicalize parent: {}", e))
}

#[tauri::command]
pub async fn local_list_dir(
    manager: tauri::State<'_, LocalFsManager>,
    path: String,
) -> Result<Vec<FileEntry>, String> {
    manager.ensure_authorized(&path)?;
    local_list_dir_blocking(path)
}

fn local_list_dir_blocking(path: String) -> Result<Vec<FileEntry>, String> {
    let dir_path = if path.is_empty() { home_dir() } else { path };
    reject_suspicious_local_path(&dir_path)?;

    let dir = Path::new(&dir_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("directory not found: {}", dir_path));
    }

    let mut entries: Vec<FileEntry> = Vec::new();
    let read_dir = fs::read_dir(dir).map_err(|e| format!("read dir: {}", e))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| format!("dir entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "." {
            continue;
        }

        let file_path = entry.path();
        let symlink_meta = file_path.symlink_metadata().ok();
        let is_symlink = symlink_meta
            .as_ref()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let metadata = if is_symlink {
            entry.metadata().unwrap_or_else(|_| symlink_meta.unwrap())
        } else {
            entry.metadata().map_err(|e| format!("metadata: {}", e))?
        };

        let modified = metadata.modified().ok().and_then(|t| {
            let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            chrono::DateTime::from_timestamp(secs, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        });

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode() & 0o777)
        };
        #[cfg(not(unix))]
        let permissions: Option<u32> = None;

        entries.push(FileEntry {
            name,
            path: file_path.to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            is_symlink,
            size: if metadata.is_dir() { 0 } else { metadata.len() },
            modified,
            permissions,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[tauri::command]
pub async fn local_read_file(
    manager: tauri::State<'_, LocalFsManager>,
    path: String,
    max_size: Option<u64>,
) -> Result<String, String> {
    manager.ensure_authorized(&path)?;
    let file = Path::new(&path);
    if !file.exists() {
        return Err(format!("file not found: {}", path));
    }

    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let limit = max_size.unwrap_or(10 * 1024 * 1024);
    if file_size > limit {
        return Err(format!(
            "文件过大 ({} bytes)，预览限制 {} bytes",
            file_size, limit
        ));
    }

    fs::read(file).map(|bytes| BASE64.encode(&bytes)).map_err(|e| format!("read failed: {}", e))
}

#[tauri::command]
pub fn local_mkdir(manager: tauri::State<'_, LocalFsManager>, path: String) -> Result<(), String> {
    manager.ensure_authorized(&path)?;
    fs::create_dir_all(&path).map_err(|e| format!("mkdir failed: {}", e))
}

#[tauri::command]
pub fn local_rename(
    manager: tauri::State<'_, LocalFsManager>,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    manager.ensure_authorized(&old_path)?;
    manager.ensure_authorized(&new_path)?;
    fs::rename(&old_path, &new_path).map_err(|e| format!("rename failed: {}", e))
}

#[tauri::command]
pub fn local_remove(manager: tauri::State<'_, LocalFsManager>, path: String) -> Result<(), String> {
    manager.ensure_authorized(&path)?;
    let p = Path::new(&path);
    if p.is_dir() {
        fs::remove_dir(p).map_err(|e| format!("rmdir failed: {}", e))
    } else {
        fs::remove_file(p).map_err(|e| format!("remove failed: {}", e))
    }
}

#[tauri::command]
pub fn local_home_dir(manager: tauri::State<'_, LocalFsManager>) -> Result<String, String> {
    let home = home_dir();
    if !manager.is_authorized(&home)? {
        return Err("home directory is not authorized".to_string());
    }
    Ok(home)
}

fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/".to_string())
}
