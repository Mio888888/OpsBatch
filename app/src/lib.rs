//! OpsBatch GPUI application (library part so UI can be tested headlessly).

// GPUI view builders take many parts and pass shared callbacks/handles around; splitting them only moves the noise.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod ai;
pub mod ansi;
pub mod app;
pub mod asset_menu;
pub mod asset_tree;
pub mod batch_exec;
pub mod broadcast;
pub mod cloud_dialog;
pub mod editor;
pub mod forward;
pub mod group_dialog;
pub mod host_dialog;
pub mod host_form;
pub mod language;
pub mod library;
pub mod log;
pub mod logging;
pub mod mcp_bridge;
pub mod rdp;
pub mod remote;
pub mod repo_sync;
pub mod scale;
pub mod services;
pub mod settings;
pub mod sftp;
pub mod stores;
pub mod terminal;
pub mod theme;
pub mod transfer;
pub mod tray;
pub mod ui_state;
pub mod update;
pub mod vault;
pub mod vault_dialog;
pub mod vnc;
pub mod widgets;
pub mod window;
pub mod workbench;
pub mod workflow;
