//! SFTP browser state (port of `stores/sftp.ts`): remote + local listings, transfers, bookmarks, preview.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use ops_core::pathnames::{basename_from_path, dirname_from_path, join_path};
use ops_ssh::sftp::FileEntry;
use serde::{Deserialize, Serialize};

/// Run blocking backend code (the sync SFTP/forward services `block_on` internally) off the UI and async threads.
pub fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> impl Future<Output = Result<T, String>> {
    let fut = ops_core::rt::run(async move { tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())? });
    async move { fut.await.and_then(|r| r) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Done,
    Error,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub id: String,
    pub direction: Direction,
    pub file_name: String,
    pub local_path: String,
    pub remote_path: String,
    pub file_size: u64,
    pub status: TransferStatus,
    pub progress: f32,
    /// bytes per millisecond (size / duration), like the original
    pub speed: f64,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub path: String,
    pub side: Side,
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub side: Side,
    pub entry: FileEntry,
    pub data: Option<Vec<u8>>,
    pub loading: bool,
    pub error: Option<String>,
    /// Video / PDF / font: nothing is loaded, the user opens it with the system app.
    pub external: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Connecting,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpEvent {
    TransferFinished { success: bool },
}

pub struct SftpStore {
    pub host_id: Option<String>,
    pub session: SessionState,
    pub remote_path: String,
    pub local_path: String,
    pub remote_entries: Vec<FileEntry>,
    pub local_entries: Vec<FileEntry>,
    pub remote_loading: bool,
    pub local_loading: bool,
    pub remote_error: Option<String>,
    pub local_error: Option<String>,
    pub selected_remote: Option<String>,
    pub selected_local: Option<String>,
    pub transfers: Vec<Transfer>,
    transfer_index: HashMap<String, usize>,
    pub bookmarks: Vec<Bookmark>,
    pub preview: Option<Preview>,
    /// How "open with the system app" reaches the OS (replaced in headless tests).
    system_opener: std::rc::Rc<dyn Fn(&std::path::Path, &mut App)>,
    remote_seq: u64,
    local_seq: u64,
    open_seq: u64,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<SftpEvent> for SftpStore {}

pub struct SftpHandle(pub Entity<SftpStore>);
impl Global for SftpHandle {}

pub fn sftp_store(cx: &App) -> Entity<SftpStore> {
    cx.global::<SftpHandle>().0.clone()
}

pub fn init(cx: &mut App) {
    let store = cx.new(SftpStore::new);
    cx.set_global(SftpHandle(store));
}

const BOOKMARKS_KEY: &str = "sftpBookmarks";

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

impl SftpStore {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ctx = crate::services::ctx(cx);
        let mut tasks = Vec::new();

        let events = ctx.events.subscribe("sftp-transfer:*");
        tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(ev) = events.recv_async().await {
                let mut parts = ev.topic.splitn(3, ':');
                let (_, Some(id), Some(kind)) = (parts.next(), parts.next(), parts.next()) else { continue };
                let (id, kind, payload) = (id.to_string(), kind.to_string(), ev.payload.clone());
                if this.update(cx, |this, cx| this.on_transfer_event(&id, &kind, &payload, cx)).is_err() {
                    break;
                }
            }
        }));

        let c2 = ctx.clone();
        tasks.push(cx.spawn(async move |this, cx| {
            let settings = ops_core::rt::run(async move { ops_core::settings::get_general_settings(&c2.db).await }).await.and_then(|r| r);
            if let Ok(map) = settings {
                if let Some(list) = map.get(BOOKMARKS_KEY).and_then(|raw| serde_json::from_str::<Vec<Bookmark>>(raw).ok()) {
                    let _ = this.update(cx, |this, cx| {
                        if this.bookmarks.is_empty() {
                            this.bookmarks = list;
                            cx.notify();
                        }
                    });
                }
            }
        }));

        Self {
            host_id: None,
            session: SessionState::Idle,
            remote_path: "/".into(),
            local_path: String::new(),
            remote_entries: Vec::new(),
            local_entries: Vec::new(),
            remote_loading: false,
            local_loading: false,
            remote_error: None,
            local_error: None,
            selected_remote: None,
            selected_local: None,
            transfers: Vec::new(),
            transfer_index: HashMap::new(),
            bookmarks: Vec::new(),
            preview: None,
            system_opener: std::rc::Rc::new(|path, cx| cx.open_with_system(path)),
            remote_seq: 0,
            local_seq: 0,
            open_seq: 0,
            _tasks: tasks,
        }
    }

    // ---- session ----

    /// Open the SFTP channel of `host_id` and list its home directory. Switching hosts resets the remote side.
    pub fn open_session(&mut self, host_id: &str, cx: &mut Context<Self>) {
        if self.host_id.as_deref() != Some(host_id) {
            self.remote_entries.clear();
            self.remote_path = "/".into();
            self.selected_remote = None;
            self.remote_error = None;
        }
        self.host_id = Some(host_id.to_string());
        self.session = SessionState::Connecting;
        self.open_seq += 1;
        let seq = self.open_seq;
        cx.notify();
        let ctx = crate::services::ctx(cx);
        let host = host_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move { ops_ssh::sftp::sftp_open(ctx, host).await }).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                if seq != this.open_seq {
                    return;
                }
                match result {
                    Ok(caps) => {
                        this.session = SessionState::Ready;
                        this.navigate(Side::Remote, &caps.home_dir, cx);
                    }
                    Err(e) => this.session = SessionState::Failed(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn close_session(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host_id.take() else { return };
        self.open_seq += 1;
        self.session = SessionState::Idle;
        let ctx = crate::services::ctx(cx);
        let _ = ops_ssh::sftp::sftp_close(&ctx.sftp, host);
        cx.notify();
    }

    // ---- listing ----

    pub fn path(&self, side: Side) -> &str {
        match side {
            Side::Local => &self.local_path,
            Side::Remote => &self.remote_path,
        }
    }

    pub fn entries(&self, side: Side) -> &[FileEntry] {
        match side {
            Side::Local => &self.local_entries,
            Side::Remote => &self.remote_entries,
        }
    }

    pub fn selected(&self, side: Side) -> Option<&FileEntry> {
        let path = match side {
            Side::Local => self.selected_local.as_deref(),
            Side::Remote => self.selected_remote.as_deref(),
        }?;
        self.entries(side).iter().find(|e| e.path == path)
    }

    pub fn select(&mut self, side: Side, path: Option<String>, cx: &mut Context<Self>) {
        match side {
            Side::Local => self.selected_local = path,
            Side::Remote => self.selected_remote = path,
        }
        cx.notify();
    }

    pub fn navigate(&mut self, side: Side, path: &str, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        let path = path.to_string();
        match side {
            Side::Remote => {
                let Some(host) = self.host_id.clone() else { return };
                self.remote_seq += 1;
                let seq = self.remote_seq;
                self.remote_loading = true;
                self.remote_error = None;
                cx.notify();
                cx.spawn(async move |this, cx| {
                    let p = path.clone();
                    let result = ops_core::rt::run(async move { ops_ssh::sftp::sftp_list_dir(ctx, host, p).await }).await.and_then(|r| r);
                    let _ = this.update(cx, |this, cx| {
                        if seq != this.remote_seq {
                            return;
                        }
                        this.remote_loading = false;
                        match result {
                            Ok(entries) => {
                                this.remote_entries = entries;
                                this.remote_path = path;
                                this.selected_remote = None;
                            }
                            Err(e) => this.remote_error = Some(e),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Side::Local => {
                self.local_seq += 1;
                let seq = self.local_seq;
                self.local_loading = true;
                self.local_error = None;
                cx.notify();
                cx.spawn(async move |this, cx| {
                    let p = path.clone();
                    let result = ops_core::rt::run(async move { ops_ssh::local_fs::local_list_dir(&ctx.local_fs, p).await }).await.and_then(|r| r);
                    let _ = this.update(cx, |this, cx| {
                        if seq != this.local_seq {
                            return;
                        }
                        this.local_loading = false;
                        match result {
                            Ok(entries) => {
                                this.local_entries = entries;
                                this.local_path = path;
                                this.selected_local = None;
                            }
                            Err(e) => this.local_error = Some(e),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    pub fn go_up(&mut self, side: Side, cx: &mut Context<Self>) {
        let parent = dirname_from_path(self.path(side));
        self.navigate(side, &parent, cx);
    }

    pub fn refresh(&mut self, side: Side, cx: &mut Context<Self>) {
        let p = self.path(side).to_string();
        self.navigate(side, &p, cx);
    }

    /// Allow a user-chosen folder (the picker lives in the panel) and show it.
    pub fn authorize_local(&mut self, dir: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        let ctx = crate::services::ctx(cx);
        let canonical = ops_ssh::local_fs::local_authorize_directory(&ctx.local_fs, dir)?;
        self.navigate(Side::Local, &canonical, cx);
        Ok(())
    }

    // ---- transfers ----

    fn start_transfer(&mut self, direction: Direction, local_path: String, remote_path: String, cx: &mut Context<Self>) {
        let Some(host) = self.host_id.clone() else { return };
        let ctx = crate::services::ctx(cx);
        let id = new_id(if direction == Direction::Upload { "upload" } else { "download" });
        let file_name = basename_from_path(if direction == Direction::Upload { &local_path } else { &remote_path }, "file");
        self.transfer_index.insert(id.clone(), self.transfers.len());
        self.transfers.push(Transfer {
            id: id.clone(),
            direction,
            file_name,
            local_path: local_path.clone(),
            remote_path: remote_path.clone(),
            file_size: 0,
            status: TransferStatus::Transferring,
            progress: 0.0,
            speed: 0.0,
            duration_ms: None,
            error: None,
        });
        cx.notify();
        let task_id = id.clone();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                match direction {
                    Direction::Upload => ops_ssh::sftp::sftp_upload(ctx, host, local_path, remote_path, task_id).await,
                    Direction::Download => ops_ssh::sftp::sftp_download(ctx, host, remote_path, local_path, task_id).await,
                }
            })
            .await
            .and_then(|r| r);
            if let Err(error) = result {
                // The backend also emits a `done` event; this covers failures before that point (no session...).
                let _ = this.update(cx, |this, cx| this.finish_transfer(&id, false, None, None, Some(error), cx));
            }
        })
        .detach();
    }

    /// Upload a local file into the current remote directory.
    pub fn upload(&mut self, local_path: &str, cx: &mut Context<Self>) {
        let remote = join_path(&self.remote_path, &basename_from_path(local_path, "file"));
        self.start_transfer(Direction::Upload, local_path.to_string(), remote, cx);
    }

    /// Download a remote file into the current local directory.
    pub fn download(&mut self, remote_path: &str, cx: &mut Context<Self>) {
        if self.local_path.is_empty() {
            return;
        }
        let local = join_path(&self.local_path, &basename_from_path(remote_path, "file"));
        self.start_transfer(Direction::Download, local, remote_path.to_string(), cx);
    }

    fn on_transfer_event(&mut self, id: &str, kind: &str, payload: &serde_json::Value, cx: &mut Context<Self>) {
        let Some(&ix) = self.transfer_index.get(id) else { return };
        match kind {
            "start" => {
                if let Some(size) = payload.get("fileSize").and_then(|v| v.as_u64()) {
                    self.transfers[ix].file_size = size;
                    cx.notify();
                }
            }
            "done" => {
                let success = payload.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                let size = payload.get("fileSize").and_then(|v| v.as_u64());
                let dur = payload.get("durationMs").and_then(|v| v.as_u64());
                let err = payload.get("error").and_then(|v| v.as_str()).map(str::to_string);
                self.finish_transfer(id, success, size, dur, err, cx);
            }
            _ => {}
        }
    }

    fn finish_transfer(&mut self, id: &str, success: bool, size: Option<u64>, duration_ms: Option<u64>, error: Option<String>, cx: &mut Context<Self>) {
        let Some(&ix) = self.transfer_index.get(id) else { return };
        let t = &mut self.transfers[ix];
        if matches!(t.status, TransferStatus::Done | TransferStatus::Error) {
            return; // first outcome wins (done event vs. command result)
        }
        t.status = if success { TransferStatus::Done } else { TransferStatus::Error };
        if success {
            t.progress = 100.0;
        }
        if let Some(s) = size.filter(|s| *s > 0) {
            t.file_size = s;
        }
        t.duration_ms = duration_ms.filter(|d| *d > 0);
        t.error = error;
        t.speed = match t.duration_ms {
            Some(d) => t.file_size as f64 / d as f64,
            None => 0.0,
        };
        let direction = t.direction;
        if success {
            self.refresh(if direction == Direction::Upload { Side::Remote } else { Side::Local }, cx);
        }
        cx.emit(SftpEvent::TransferFinished { success });
        cx.notify();
    }

    pub fn active_transfers(&self) -> usize {
        self.transfers.iter().filter(|t| matches!(t.status, TransferStatus::Transferring | TransferStatus::Pending)).count()
    }

    // ---- file operations (each resolves with the backend result and refreshes the listing) ----

    fn op<T: Send + 'static>(
        &mut self,
        side: Side,
        cx: &mut Context<Self>,
        f: impl FnOnce(std::sync::Arc<ops_ssh::Ctx>, Option<String>) -> Result<T, String> + Send + 'static,
    ) -> Task<Result<T, String>> {
        let ctx = crate::services::ctx(cx);
        let host = self.host_id.clone();
        cx.spawn(async move |this, cx| {
            let result = blocking(move || f(ctx, host)).await;
            if result.is_ok() {
                let _ = this.update(cx, |this, cx| this.refresh(side, cx));
            }
            result
        })
    }

    pub fn create_file(&mut self, side: Side, path: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        match side {
            Side::Remote => self.op(Side::Remote, cx, move |ctx, host| {
                let host = host.ok_or("no SFTP session")?;
                ops_ssh::sftp::sftp_write_file(&ctx.db, &ctx.sftp, &ctx.registry, host, path, String::new())
            }),
            Side::Local => self.op(Side::Local, cx, move |ctx, _| {
                ctx.local_fs.ensure_authorized(&path)?;
                std::fs::File::create(&path).map(|_| ()).map_err(|e| format!("create failed: {e}"))
            }),
        }
    }

    pub fn mkdir(&mut self, side: Side, path: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        match side {
            Side::Remote => self.op(Side::Remote, cx, move |ctx, host| {
                let host = host.ok_or("no SFTP session")?;
                ops_ssh::sftp::sftp_mkdir(&ctx.db, &ctx.sftp, &ctx.registry, host, path)
            }),
            Side::Local => self.op(Side::Local, cx, move |ctx, _| ops_ssh::local_fs::local_mkdir(&ctx.local_fs, path)),
        }
    }

    pub fn rename(&mut self, side: Side, old: String, new: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        match side {
            Side::Remote => self.op(Side::Remote, cx, move |ctx, host| {
                let host = host.ok_or("no SFTP session")?;
                ops_ssh::sftp::sftp_rename(&ctx.sftp, host, old, new)
            }),
            Side::Local => self.op(Side::Local, cx, move |ctx, _| ops_ssh::local_fs::local_rename(&ctx.local_fs, old, new)),
        }
    }

    pub fn remove(&mut self, side: Side, path: String, is_dir: bool, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        match side {
            Side::Remote => self.op(Side::Remote, cx, move |ctx, host| {
                let host = host.ok_or("no SFTP session")?;
                if is_dir { ops_ssh::sftp::sftp_rmdir(&ctx.sftp, host, path) } else { ops_ssh::sftp::sftp_remove(&ctx.sftp, host, path) }
            }),
            Side::Local => self.op(Side::Local, cx, move |ctx, _| ops_ssh::local_fs::local_remove(&ctx.local_fs, path)),
        }
    }

    pub fn extract_archive(&mut self, archive: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.op(Side::Remote, cx, move |ctx, host| {
            let host = host.ok_or("no SFTP session")?;
            ops_ssh::sftp::sftp_extract_archive(&ctx.db, &ctx.registry, host, archive, String::new()).map(|_| ())
        })
    }

    // ---- preview ----

    pub fn open_preview(&mut self, side: Side, entry: FileEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            return;
        }
        let kind = super::kinds::preview_kind(&entry.name);
        if kind.opens_externally() {
            self.preview = Some(Preview { side, entry, data: None, loading: false, error: None, external: true });
            cx.notify();
            return;
        }
        self.preview = Some(Preview { side, entry: entry.clone(), data: None, loading: true, error: None, external: false });
        cx.notify();
        let ctx = crate::services::ctx(cx);
        let host = self.host_id.clone();
        let path = entry.path.clone();
        cx.spawn(async move |this, cx| {
            let max = kind.read_limit();
            let result = ops_core::rt::run(async move {
                use base64::Engine as _;
                let b64 = match side {
                    Side::Remote => ops_ssh::sftp::sftp_read_file(ctx, host.ok_or("no SFTP session")?, path, Some(max)).await?,
                    Side::Local => ops_ssh::local_fs::local_read_file(&ctx.local_fs, path, Some(max)).await?,
                };
                base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = this.preview.as_mut().filter(|p| p.entry.path == entry.path && p.side == side) {
                    p.loading = false;
                    match result {
                        Ok(bytes) => p.data = Some(bytes),
                        Err(e) => p.error = Some(e),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn set_system_opener(&mut self, opener: std::rc::Rc<dyn Fn(&std::path::Path, &mut App)>) {
        self.system_opener = opener;
    }

    /// Open the previewed file with the operating system's default application. Remote files are downloaded to a
    /// temporary folder first.
    pub fn open_preview_externally(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.preview.as_mut() else { return };
        let (side, entry) = (p.side, p.entry.clone());
        if side == Side::Local {
            let opener = self.system_opener.clone();
            opener(std::path::Path::new(&entry.path), cx);
            return;
        }
        let Some(host) = self.host_id.clone() else { return };
        p.loading = true;
        p.error = None;
        cx.notify();
        let ctx = crate::services::ctx(cx);
        let dir = ctx.core.data_dir().join("previews").join(new_id("pv"));
        let target = dir.join(&entry.name);
        let (remote, local, task_id) = (entry.path.clone(), target.display().to_string(), new_id("preview"));
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                ops_ssh::sftp::sftp_download(ctx, host, remote, local, task_id).await
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = this.preview.as_mut().filter(|p| p.entry.path == entry.path) {
                    p.loading = false;
                    match result {
                        Ok(()) => {
                            let opener = this.system_opener.clone();
                            opener(&target, cx)
                        }
                        Err(e) => p.error = Some(e),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = None;
        cx.notify();
    }

    // ---- bookmarks ----

    pub fn add_bookmark(&mut self, side: Side, cx: &mut Context<Self>) {
        let path = self.path(side).to_string();
        if path.is_empty() {
            return;
        }
        let name = basename_from_path(&path, &path);
        self.bookmarks.push(Bookmark { id: new_id("bm"), name, path, side });
        self.persist_bookmarks(cx);
    }

    pub fn remove_bookmark(&mut self, id: &str, cx: &mut Context<Self>) {
        self.bookmarks.retain(|b| b.id != id);
        self.persist_bookmarks(cx);
    }

    fn persist_bookmarks(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        let ctx = crate::services::ctx(cx);
        let raw = serde_json::to_string(&self.bookmarks).unwrap_or_else(|_| "[]".into());
        ops_core::rt::spawn(async move {
            let mut m = HashMap::new();
            m.insert(BOOKMARKS_KEY.to_string(), raw);
            if let Err(e) = ops_core::settings::save_general_settings(&ctx.db, m).await {
                log::warn!("could not save SFTP bookmarks: {e}");
            }
        });
    }
}
