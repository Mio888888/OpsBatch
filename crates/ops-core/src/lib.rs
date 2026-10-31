//! OpsBatch core: models, database, vault, settings, event bus, runtime bridge (TASKS M1-01..13).

pub mod asciinema;
pub mod clipboard;
pub mod core;
pub mod db;
pub mod events;
#[cfg(feature = "real-vault")]
#[path = "keychain_real.rs"]
pub mod keychain;
#[cfg(not(feature = "real-vault"))]
pub mod keychain;
pub mod log;
pub mod pathnames;
pub mod paths;
pub mod rt;
pub mod security;
pub mod settings;
pub mod system;
pub mod theme;
pub mod tls;
pub mod update;
pub mod vnc_settings;

pub use crate::core::Core;
pub use db::Database;
