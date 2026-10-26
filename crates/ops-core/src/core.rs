//! `Core` replaces Tauri's managed state: everything a service needs, cheaply cloneable and `Send + Sync`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::db::Database;
use crate::events::{EventBus, TerminalStreams};

#[derive(Clone)]
pub struct Core {
    pub db: Arc<Database>,
    pub events: EventBus,
    pub terminals: TerminalStreams,
    pub data_dir: PathBuf,
}

impl Core {
    /// Open (creating if necessary) the data directory, initialise the vault location and the database.
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, String> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("create {}: {e}", data_dir.display()))?;
        crate::keychain::init_local_vault_dir(data_dir.clone())?;
        let db = Database::new(&data_dir.join("opsbatch.db"))?;
        db.init_tables()?;
        Ok(Self {
            db: Arc::new(db),
            events: EventBus::new(),
            terminals: TerminalStreams::new(),
            data_dir,
        })
    }

    /// Fresh core in a unique temp directory (tests and spikes).
    pub fn open_temp(label: &str) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("opsbatch-{label}-{}", uuid::Uuid::new_v4()));
        Self::open(dir)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Equivalent of `AppHandle::emit`.
    pub fn emit<S: Serialize>(&self, topic: &str, payload: S) {
        self.events.publish(topic, payload);
    }
}
