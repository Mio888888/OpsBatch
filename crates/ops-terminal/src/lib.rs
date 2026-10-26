//! OpsBatch terminal logic without UI. Ports of the pure TypeScript helpers (`src/utils/terminal*.ts`,
//! `backgroundCommand.ts`, `dangerCommandCheck.ts`, `hostMonitor.ts`); the PTY/SSH session manager lives in `ops-ssh`.

pub mod auth;
pub mod background;
pub mod danger;
pub mod input_line;
pub mod keys;
pub mod monitor;
pub mod search;
pub mod tabs;
pub mod tracker;
