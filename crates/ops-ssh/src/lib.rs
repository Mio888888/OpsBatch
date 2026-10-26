//! OpsBatch SSH: connection pool, jump hosts, terminal channels (SSH + local PTY), SFTP, forwarding, batch
//! execution and transfer. Local-terminal/session code lives here (not in ops-terminal) because the terminal
//! manager owns both SSH-backed and PTY-backed sessions.

pub mod ctx;
pub mod execution;
pub mod forward;
pub mod groups;
pub mod hosts;
pub mod import;
pub mod local_fs;
pub mod proxyjump;
pub mod sftp;
pub mod ssh;
pub mod tags;
pub mod terminal;
pub mod terminal_shell;
pub mod transfer;

pub use ctx::Ctx;
