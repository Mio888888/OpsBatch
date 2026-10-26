//! `Ctx` bundles everything the SSH-side services need; it replaces Tauri's managed `State<T>` set.

use std::ops::Deref;
use std::sync::Arc;

use ops_core::Core;

use crate::forward::ForwardManager;
use crate::local_fs::LocalFsManager;
use crate::sftp::SftpManager;
use crate::ssh::SshConnectionRegistry;
use crate::terminal::TerminalManager;

pub struct Ctx {
    pub core: Core,
    pub registry: Arc<SshConnectionRegistry>,
    pub sftp: Arc<SftpManager>,
    pub forwards: Arc<ForwardManager>,
    pub local_fs: Arc<LocalFsManager>,
    pub terminals_mgr: Arc<TerminalManager>,
}

impl Ctx {
    /// Build all managers, hook the registry up to the event bus and start its idle reaper.
    pub fn new(core: Core) -> Arc<Self> {
        let registry = Arc::new(SshConnectionRegistry::new());
        registry.set_core(core.clone());
        SshConnectionRegistry::start_idle_reaper(registry.clone());

        let local_fs = Arc::new(LocalFsManager::new());
        let _ = local_fs.authorize_root(core.data_dir());

        Arc::new(Self {
            core,
            registry,
            sftp: Arc::new(SftpManager::new()),
            forwards: Arc::new(ForwardManager::new()),
            local_fs,
            terminals_mgr: Arc::new(TerminalManager::new()),
        })
    }
}

impl Deref for Ctx {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}
