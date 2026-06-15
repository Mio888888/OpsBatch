mod config;
mod manager;
mod types;

use tauri::AppHandle;

use crate::db::Database;

pub use config::{parse_vnc_settings, vnc_port_from_settings};
pub use manager::VncSessionManager;
pub use types::{
    StartVncSessionRequest, VncConnectResponse, VncInputEvent, VncKeyEventRequest,
    VncPointerEventRequest, VncSessionStarted, VncSessionStatus, VncSimpleRequest,
};

pub(crate) fn append_vnc_diagnostic_log(app: &AppHandle, message: &str) {
    crate::commands::diagnostics::append_diagnostic_log(app, "vnc-backend", message);
}

#[tauri::command]
pub fn vnc_connect(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    manager: tauri::State<'_, VncSessionManager>,
    host_id: String,
    session_id: String,
) -> Result<VncConnectResponse, String> {
    append_vnc_diagnostic_log(
        &app,
        &format!("command received hostId={host_id} sessionId={session_id}"),
    );
    let config = match config::load_vnc_host_config(&db, &host_id) {
        Ok(config) => config,
        Err(error) => {
            append_vnc_diagnostic_log(
                &app,
                &format!(
                    "config load failed hostId={host_id} sessionId={session_id} error={error}"
                ),
            );
            return Err(error);
        }
    };
    append_vnc_diagnostic_log(
        &app,
        &format!(
            "config loaded hostId={} sessionId={} host={} port={} usernameSet={} passwordSet={} authMethod={} shared={} viewOnly={} proxySet={}",
            host_id,
            session_id,
            config.host,
            config.port,
            config.username.is_some(),
            config.password.as_ref().is_some_and(|value| !value.is_empty()),
            config.auth_method,
            config.options.shared_session,
            config.options.view_only,
            config.proxy.is_some(),
        ),
    );
    let proxy = config.proxy.clone();
    let request = StartVncSessionRequest::from_host_config(session_id.clone(), config);
    let started = manager.start_session(app.clone(), request, proxy)?;
    append_vnc_diagnostic_log(
        &app,
        &format!(
            "command completed hostId={host_id} sessionId={session_id} websocketUrl={}",
            started.websocket_url
        ),
    );
    Ok(VncConnectResponse {
        session_id,
        host_id,
        websocket_url: started.websocket_url,
        username: started.username,
        password: started.password,
        auth_method: started.auth_method,
        shared: started.shared,
        view_only: started.view_only,
    })
}

#[tauri::command]
pub fn vnc_send_input(
    manager: tauri::State<'_, VncSessionManager>,
    session_id: String,
    event: VncInputEvent,
) -> Result<(), String> {
    match event {
        VncInputEvent::Mouse { x, y, buttons } => manager.pointer_event(VncPointerEventRequest {
            session_id,
            x,
            y,
            button_mask: buttons,
        }),
        VncInputEvent::Key { keycode, down } => manager.key_event(VncKeyEventRequest {
            session_id,
            key: keycode,
            down,
        }),
        VncInputEvent::Refresh => manager.refresh(VncSimpleRequest { session_id }),
        VncInputEvent::Clipboard { text } => {
            let _ = text;
            manager.refresh(VncSimpleRequest { session_id })
        }
    }
}

#[tauri::command]
pub fn vnc_disconnect(
    manager: tauri::State<'_, VncSessionManager>,
    session_id: String,
) -> Result<(), String> {
    manager.close_session(VncSimpleRequest { session_id })
}

#[tauri::command]
pub fn start_vnc_session(
    app: AppHandle,
    manager: tauri::State<'_, VncSessionManager>,
    mut request: StartVncSessionRequest,
) -> Result<VncSessionStarted, String> {
    if request.password().is_none() {
        if let Some(owner_id) = request.secret_owner_id().map(str::to_string) {
            request.set_password(
                crate::keychain::get_host_password(&owner_id)
                    .map(Some)
                    .map_err(|error| format!("failed to read VNC password: {error}"))?,
            );
        }
    }
    manager.start_session(app, request, None)
}

#[tauri::command]
pub fn send_vnc_pointer_event(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncPointerEventRequest,
) -> Result<(), String> {
    manager.pointer_event(request)
}

#[tauri::command]
pub fn send_vnc_key_event(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncKeyEventRequest,
) -> Result<(), String> {
    manager.key_event(request)
}

#[tauri::command]
pub fn refresh_vnc_session(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncSimpleRequest,
) -> Result<(), String> {
    manager.refresh(request)
}

#[tauri::command]
pub fn close_vnc_session(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncSimpleRequest,
) -> Result<(), String> {
    manager.close_session(request)
}

#[tauri::command]
pub fn get_vnc_session_status(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncSimpleRequest,
) -> Result<VncSessionStatus, String> {
    manager.session_status(request)
}

#[tauri::command]
pub fn send_vnc_ctrl_alt_delete(
    manager: tauri::State<'_, VncSessionManager>,
    request: VncSimpleRequest,
) -> Result<(), String> {
    manager.send_ctrl_alt_delete(request)
}

#[cfg(test)]
mod tests {
    use super::types::{VncAuthMethod, DEFAULT_VNC_PORT};
    use super::*;

    #[test]
    fn vnc_port_prefers_settings_over_host_port() {
        assert_eq!(
            5902,
            vnc_port_from_settings(r#"{"protocol":"vnc","vncPort":5902}"#, 22)
        );
    }

    #[test]
    fn vnc_port_falls_back_to_default_when_missing() {
        assert_eq!(
            DEFAULT_VNC_PORT,
            vnc_port_from_settings(r#"{"protocol":"vnc"}"#, 22)
        );
    }

    #[test]
    fn vnc_settings_parse_dedicated_username() {
        let settings = parse_vnc_settings(
            r#"{"protocol":"vnc","vncUsername":"alice","vncPassword":"secret"}"#,
        );

        assert_eq!(settings.vnc_username.as_deref(), Some("alice"));
        assert_eq!(settings.vnc_password.as_deref(), Some("secret"));
    }

    #[test]
    fn vnc_auth_method_defaults_to_vnc_auth_and_accepts_ard() {
        let default_settings = parse_vnc_settings(r#"{"protocol":"vnc"}"#);
        let ard_settings = parse_vnc_settings(r#"{"protocol":"vnc","vncAuthMethod":"ard"}"#);

        assert_eq!(default_settings.vnc_auth_method, VncAuthMethod::VncAuth);
        assert_eq!(
            ard_settings.vnc_auth_method,
            VncAuthMethod::AppleRemoteDesktop
        );
    }

    #[test]
    fn start_request_accepts_optional_username() {
        let json = r#"{"sessionId":"vnc-1","host":"mac.local","username":"bob","password":"pw","authMethod":"ard"}"#;
        let request: StartVncSessionRequest =
            serde_json::from_str(json).expect("request deserializes");
        assert_eq!(request.username.as_deref(), Some("bob"));
        assert_eq!(request.auth_method, VncAuthMethod::AppleRemoteDesktop);
    }

    #[test]
    fn start_request_username_defaults_to_none() {
        let json = r#"{"sessionId":"vnc-1","host":"mac.local"}"#;
        let request: StartVncSessionRequest =
            serde_json::from_str(json).expect("request deserializes");
        assert!(request.username.is_none());
        assert_eq!(request.auth_method, VncAuthMethod::VncAuth);
    }
}
