mod commands;
mod db;
mod keychain;
mod security;
mod ssh;
mod tls;

use std::sync::Arc;
use tauri::webview::PageLoadEvent;
#[cfg(debug_assertions)]
use tauri::Listener;
use tauri::Manager;

pub fn run() {
    tls::install_default_crypto_provider();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            let event = match payload.event() {
                PageLoadEvent::Started => "started",
                PageLoadEvent::Finished => "finished",
            };
            crate::commands::diagnostics::append_diagnostic_log(
                webview.app_handle(),
                "page-load",
                &format!(
                    "label={} event={} url={}",
                    webview.label(),
                    event,
                    payload.url()
                ),
            );
        })
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&app_data_dir).ok();
            let db_path = app_data_dir.join("opsbatch.db");
            let database = db::Database::new(&db_path).expect("failed to open database");
            database.init_tables().expect("failed to init tables");
            commands::diagnostics::append_diagnostic_log(
                app.handle(),
                "startup",
                &format!(
                    "app started app_data_dir={} db_path={}",
                    app_data_dir.display(),
                    db_path.display()
                ),
            );
            app.manage(database);
            app.manage(commands::terminal::TerminalManager::new());
            app.manage(commands::sftp::SftpManager::new());
            let local_fs = commands::local_fs::LocalFsManager::new();
            let _ = local_fs.authorize_root(&app_data_dir);
            app.manage(local_fs);
            app.manage(commands::forward::ForwardManager::new());
            app.manage(commands::rdp::RdpManager::new());
            app.manage(commands::rdp::RdpWebRtcManager::new());
            app.manage(commands::vnc::VncSessionManager::new());
            app.manage(Arc::new(commands::mcp::McpManager::new()));
            let registry = ssh::SshConnectionRegistry::new();
            app.manage(registry);
            let ssh_registry = app.state::<ssh::SshConnectionRegistry>();
            ssh_registry.set_app_handle(app.handle().clone());
            ssh::SshConnectionRegistry::start_idle_reaper(app.handle().clone());

            // Emit startup log
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    crate::commands::app_log::emit_log(
                        &handle,
                        "info",
                        "system",
                        "OpsBatch backend started",
                        "backend",
                    );
                });
            }
            commands::github::spawn_startup_repo_updates(app.handle().clone());

            // Open devtools for all windows in debug (dev) builds
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
                let handle = app.handle().clone();
                app.listen("tauri://window-created", move |_event| {
                    for window in handle.webview_windows().values() {
                        window.open_devtools();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App Log
            commands::app_log::ping_log,
            commands::app_log::get_log_history,
            commands::app_log::emit_frontend_log,
            commands::diagnostics::write_diagnostic_log,
            // Hosts
            commands::hosts::list_hosts,
            commands::hosts::add_host,
            commands::hosts::update_host,
            commands::hosts::delete_host,
            commands::hosts::check_host_status,
            commands::hosts::get_host_system_info,
            commands::hosts::get_host_monitor_snapshot,
            // Groups
            commands::groups::list_groups,
            commands::groups::add_group,
            commands::groups::update_group,
            commands::groups::delete_group,
            // Tags
            commands::tags::list_tags,
            commands::tags::add_tag,
            commands::tags::update_tag,
            commands::tags::delete_tag,
            // Execution
            commands::execution::list_execution_history,
            commands::execution::get_execution_detail,
            commands::execution::execute_command,
            commands::execution::get_task_output,
            // Transfer
            commands::transfer::file_transfer,
            // Import/Export
            commands::import::import_hosts_csv,
            commands::import::export_hosts_csv,
            // AI
            commands::ai::get_ai_config,
            commands::ai::save_ai_config,
            commands::ai::ai_chat,
            commands::ai::ai_chat_stream,
            commands::ai::ai_chat_cancel,
            commands::ai::ai_assess_action,
            commands::ai::ai_record_action_event,
            commands::ai::ai_validate_command_plan,
            commands::ai::ai_list_conversations,
            commands::ai::ai_get_conversation,
            commands::ai::ai_delete_conversation,
            commands::ai::ai_generate_script,
            commands::ai::ai_analyze_results,
            commands::ai::ai_diagnose_error,
            commands::ai::ai_risk_assessment,
            commands::ai::ai_list_models,
            commands::ai::ai_keychain_store,
            commands::ai::ai_keychain_get,
            commands::ai::ai_keychain_delete,
            // RAG
            commands::rag::rag_create_collection,
            commands::rag::rag_list_collections,
            commands::rag::rag_import_document,
            commands::rag::rag_delete_collection,
            commands::rag::rag_search,
            // MCP
            commands::mcp::mcp_list_servers,
            commands::mcp::mcp_add_server,
            commands::mcp::mcp_remove_server,
            commands::mcp::mcp_connect_stdio,
            commands::mcp::mcp_disconnect,
            commands::mcp::mcp_call_tool,
            commands::mcp::mcp_list_tools,
            // GitHub
            commands::github::list_repos,
            commands::github::add_repo,
            commands::github::delete_repo,
            commands::github::toggle_repo,
            commands::github::set_repo_update_on_startup,
            commands::github::pull_repo,
            // Settings
            commands::settings::list_danger_rules,
            commands::settings::add_danger_rule,
            commands::settings::delete_danger_rule,
            commands::settings::toggle_danger_rule,
            // Cloud
            commands::cloud::list_cloud_providers,
            commands::cloud::save_cloud_providers,
            commands::cloud::fetch_cloud_instances,
            commands::cloud::import_cloud_instances,
            // Asciinema
            commands::asciinema::write_asciinema_recording,
            commands::asciinema::read_asciinema_recording,
            commands::asciinema::list_recordings,
            // Quick Actions
            commands::quick_actions::list_quick_actions,
            commands::quick_actions::add_quick_action,
            commands::quick_actions::update_quick_action,
            commands::quick_actions::delete_quick_action,
            commands::quick_actions::reorder_quick_actions,
            commands::quick_actions::toggle_star_quick_action,
            // Library (commands + scripts)
            commands::library::list_commands,
            commands::library::add_command,
            commands::library::update_command,
            commands::library::delete_command,
            commands::library::toggle_star_command,
            commands::library::list_scripts,
            commands::library::add_script,
            commands::library::update_script,
            commands::library::delete_script,
            commands::library::toggle_star_script,
            commands::library::list_script_versions,
            commands::library::save_script_version,
            // Workflow
            commands::workflow::list_workflows,
            commands::workflow::create_workflow,
            commands::workflow::update_workflow,
            commands::workflow::delete_workflow,
            // Workflow Templates
            commands::workflow::list_workflow_templates,
            commands::workflow::save_workflow_template,
            commands::workflow::delete_workflow_template,
            // Scheduled Tasks
            commands::workflow::list_scheduled_tasks,
            commands::workflow::add_scheduled_task,
            commands::workflow::update_scheduled_task,
            commands::workflow::delete_scheduled_task,
            commands::workflow::check_scheduled_tasks,
            // Execution cancel
            commands::execution::cancel_execution,
            // General Settings
            commands::settings::get_general_settings,
            commands::settings::save_general_settings,
            commands::settings::list_system_font_families,
            commands::settings::export_database_backup,
            // System
            commands::system::get_local_performance_snapshot,
            // RDP
            commands::rdp::rdp_connect,
            commands::rdp::rdp_send_input,
            commands::rdp::rdp_disconnect,
            commands::rdp::webrtc::rdp_webrtc_create_offer,
            commands::rdp::webrtc::rdp_webrtc_set_answer,
            commands::rdp::webrtc::rdp_webrtc_close,
            // VNC
            commands::vnc::vnc_connect,
            commands::vnc::vnc_send_input,
            commands::vnc::vnc_disconnect,
            commands::vnc::start_vnc_session,
            commands::vnc::send_vnc_pointer_event,
            commands::vnc::send_vnc_key_event,
            commands::vnc::refresh_vnc_session,
            commands::vnc::close_vnc_session,
            commands::vnc::get_vnc_session_status,
            commands::vnc::send_vnc_ctrl_alt_delete,
            // Terminal
            commands::terminal::terminal_connect,
            commands::terminal::terminal_connect_local,
            commands::terminal::terminal_write,
            commands::terminal::terminal_batch_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_disconnect,
            // SFTP
            commands::sftp::sftp_open,
            commands::sftp::sftp_close,
            commands::sftp::sftp_list_dir,
            commands::sftp::sftp_stat,
            commands::sftp::sftp_read_file,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_remove,
            commands::sftp::sftp_rmdir,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_download,
            commands::sftp::sftp_exists,
            commands::sftp::sftp_home_dir,
            commands::sftp::sftp_extract_archive,
            commands::sftp::sftp_warmup,
            // Local FS
            commands::local_fs::local_list_dir,
            commands::local_fs::local_read_file,
            commands::local_fs::local_mkdir,
            commands::local_fs::local_rename,
            commands::local_fs::local_remove,
            commands::local_fs::local_home_dir,
            commands::local_fs::local_authorize_directory,
            commands::local_fs::local_is_authorized,
            // Editor
            commands::sftp::sftp_write_file,
            commands::sftp::sftp_read_file_tree,
            // Port Forwarding
            commands::forward::forward_list,
            commands::forward::forward_add,
            commands::forward::forward_remove,
            commands::forward::forward_stop,
            // ProxyJump
            commands::proxyjump::parse_ssh_config,
            commands::proxyjump::get_jump_topology,
            commands::proxyjump::resolve_jump_chain,
            commands::proxyjump::cascade_disconnect,
            commands::proxyjump::import_ssh_config_hosts,
            commands::window::open_managed_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
