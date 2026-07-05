use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

enum ManagedWindow {
    BatchTerminal,
    BatchTransfer,
    Editor,
    Settings,
    GlobalLog,
    Rdp,
    Vnc,
}

impl ManagedWindow {
    fn parse(kind: &str) -> Result<Self, String> {
        match kind {
            "batch-terminal" => Ok(Self::BatchTerminal),
            "batch-transfer" => Ok(Self::BatchTransfer),
            "editor" => Ok(Self::Editor),
            "settings" => Ok(Self::Settings),
            "global-log" => Ok(Self::GlobalLog),
            "rdp" => Ok(Self::Rdp),
            "vnc" => Ok(Self::Vnc),
            _ => Err("window kind is not allowed".to_string()),
        }
    }

    fn label(
        &self,
        host_ids: &[String],
        query: Option<&HashMap<String, String>>,
    ) -> Result<String, String> {
        match self {
            Self::BatchTerminal => Ok("batch-terminal".to_string()),
            Self::BatchTransfer => Ok("batch-transfer".to_string()),
            Self::Editor => editor_label(host_ids, query),
            Self::Settings => Ok("settings".to_string()),
            Self::GlobalLog => Ok("global-log".to_string()),
            Self::Rdp => Ok(format!("rdp-{}", encode_single_host_id(host_ids)?)),
            Self::Vnc => Ok(format!("vnc-{}", encode_single_host_id(host_ids)?)),
        }
    }

    fn title(&self, host_count: usize) -> String {
        match self {
            Self::BatchTerminal => format!("批量终端 ({})", host_count),
            Self::BatchTransfer => format!("批量传输 ({})", host_count),
            Self::Editor => "远程编辑".to_string(),
            Self::Settings => "设置".to_string(),
            Self::GlobalLog => "全局日志".to_string(),
            Self::Rdp => "RDP 远程桌面".to_string(),
            Self::Vnc => "VNC 远程桌面".to_string(),
        }
    }

    fn route(
        &self,
        host_ids: &[String],
        query: Option<&HashMap<String, String>>,
    ) -> Result<String, String> {
        match self {
            Self::BatchTerminal => Ok(format!(
                "/batch-terminal?hostIds={}",
                encode_host_ids(host_ids)?
            )),
            Self::BatchTransfer => Ok(format!(
                "/batch-transfer?hostIds={}",
                encode_host_ids(host_ids)?
            )),
            Self::Editor => editor_route(host_ids, query),
            Self::Settings => Ok("/settings".to_string()),
            Self::GlobalLog => Ok("/global-log".to_string()),
            Self::Rdp => Ok(format!("/rdp?hostId={}", encode_single_host_id(host_ids)?)),
            Self::Vnc => Ok(format!(
                "/vnc?hostId={}&vncDebug=1",
                encode_single_host_id(host_ids)?
            )),
        }
    }

    fn size(&self) -> (f64, f64, f64, f64) {
        match self {
            Self::BatchTerminal => (1200.0, 800.0, 800.0, 600.0),
            Self::BatchTransfer => (900.0, 700.0, 700.0, 550.0),
            Self::Editor => (1100.0, 760.0, 760.0, 560.0),
            Self::Settings => (900.0, 680.0, 760.0, 560.0),
            Self::GlobalLog => (720.0, 600.0, 500.0, 400.0),
            Self::Rdp => (1280.0, 820.0, 760.0, 560.0),
            Self::Vnc => (1280.0, 820.0, 760.0, 560.0),
        }
    }

    fn requires_hosts(&self) -> bool {
        matches!(
            self,
            Self::BatchTerminal | Self::BatchTransfer | Self::Editor | Self::Rdp | Self::Vnc
        )
    }
}

fn encode_host_ids(host_ids: &[String]) -> Result<String, String> {
    if host_ids.is_empty() {
        return Err("host ids are required".to_string());
    }
    if host_ids.iter().any(|id| !is_safe_host_id(id)) {
        return Err("host id contains illegal characters".to_string());
    }
    Ok(host_ids.join(","))
}

fn encode_single_host_id(host_ids: &[String]) -> Result<String, String> {
    if host_ids.len() != 1 {
        return Err("exactly one host id is required".to_string());
    }
    encode_host_ids(host_ids)
}

struct EditorWindowQuery<'a> {
    mode: &'a str,
    path: &'a str,
}

fn parse_editor_query<'a>(
    query: Option<&'a HashMap<String, String>>,
) -> Result<EditorWindowQuery<'a>, String> {
    let query = query.ok_or_else(|| "editor query is required".to_string())?;
    let mode = query
        .get("mode")
        .ok_or_else(|| "editor mode is required".to_string())?;
    if mode != "file" && mode != "dir" {
        return Err("editor mode is invalid".to_string());
    }

    let path = query
        .get("path")
        .ok_or_else(|| "editor path is required".to_string())?;
    if !is_safe_remote_path(path) {
        return Err("editor path is invalid".to_string());
    }

    Ok(EditorWindowQuery { mode, path })
}

fn editor_label(
    host_ids: &[String],
    query: Option<&HashMap<String, String>>,
) -> Result<String, String> {
    let host_id = encode_single_host_id(host_ids)?;
    let target = parse_editor_query(query)?;
    Ok(format!(
        "editor-{}-{}",
        host_id,
        editor_target_hash(target.mode, target.path)
    ))
}

fn editor_route(
    host_ids: &[String],
    query: Option<&HashMap<String, String>>,
) -> Result<String, String> {
    let host_id = encode_single_host_id(host_ids)?;
    let target = parse_editor_query(query)?;

    Ok(format!(
        "/editor?hostId={}&mode={}&path={}",
        host_id,
        target.mode,
        encode_query_value(target.path)
    ))
}

fn editor_target_hash(mode: &str, path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    mode.hash(&mut hasher);
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_safe_host_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn is_safe_remote_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.chars().any(|ch| ch == '\0' || ch.is_control())
}

fn encode_route(route: &str) -> String {
    route
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn encode_query_value(value: &str) -> String {
    encode_route(value)
}

fn spa_entry_for_route(route: &str) -> String {
    format!("index.html?route={}", encode_route(route))
}

#[tauri::command]
pub async fn open_managed_window(
    app: tauri::AppHandle,
    kind: String,
    host_ids: Option<Vec<String>>,
    query: Option<HashMap<String, String>>,
) -> Result<(), String> {
    let window = ManagedWindow::parse(&kind)?;
    let host_ids = host_ids.unwrap_or_default();
    if window.requires_hosts() && host_ids.is_empty() {
        return Err("host ids are required".to_string());
    }

    let label = window.label(&host_ids, query.as_ref())?;
    let route = window.route(&host_ids, query.as_ref())?;
    let url = spa_entry_for_route(&route);
    if let Some(existing) = app.get_webview_window(&label) {
        crate::commands::diagnostics::append_diagnostic_log(
            app.app_handle(),
            "window",
            &format!("reuse label={label} kind={kind} route={route} url={url}"),
        );
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let (width, height, min_width, min_height) = window.size();
    let title = window.title(host_ids.len());
    crate::commands::diagnostics::append_diagnostic_log(
        app.app_handle(),
        "window",
        &format!("schedule create kind={kind} label={label} route={route} url={url}"),
    );

    let app_for_create = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::diagnostics::append_diagnostic_log(
            app_for_create.app_handle(),
            "window",
            &format!("create kind={kind} label={label} route={route} url={url}"),
        );

        let builder =
            WebviewWindowBuilder::new(&app_for_create, label.clone(), WebviewUrl::App(url.into()))
                .title(title)
                .inner_size(width, height)
                .min_inner_size(min_width, min_height)
                .decorations(false);

        #[cfg(target_os = "macos")]
        let builder = builder
            .transparent(true)
            .background_color(tauri::utils::config::Color(0, 0, 0, 0));

        if let Err(e) = builder.build() {
            let message = e.to_string();
            crate::commands::diagnostics::append_diagnostic_log(
                app_for_create.app_handle(),
                "window",
                &format!("create failed label={label} error={message}"),
            );
            return;
        }

        crate::commands::diagnostics::append_diagnostic_log(
            app_for_create.app_handle(),
            "window",
            &format!("created label={label}"),
        );
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_ids_allow_uuid_like_values_only() {
        assert!(is_safe_host_id("host_123-abc"));
        assert!(!is_safe_host_id("../settings"));
        assert!(!is_safe_host_id("a,b"));
        assert!(!is_safe_host_id(""));
    }

    #[test]
    fn spa_entry_loads_index_and_passes_route_in_query() {
        assert_eq!(
            spa_entry_for_route("/settings"),
            "index.html?route=%2Fsettings"
        );
        assert_eq!(
            spa_entry_for_route("/batch-terminal?hostIds=a_b-1"),
            "index.html?route=%2Fbatch-terminal%3FhostIds%3Da_b-1"
        );
    }

    #[test]
    fn rdp_window_uses_single_host_route_and_label() {
        let host_ids = vec!["host_123-abc".to_string()];
        let window = ManagedWindow::parse("rdp").unwrap();

        assert_eq!(window.label(&host_ids, None).unwrap(), "rdp-host_123-abc");
        assert_eq!(
            window.route(&host_ids, None).unwrap(),
            "/rdp?hostId=host_123-abc"
        );
        assert!(window.requires_hosts());
    }

    #[test]
    fn rdp_window_rejects_multiple_host_ids() {
        let host_ids = vec!["host-1".to_string(), "host-2".to_string()];
        let window = ManagedWindow::parse("rdp").unwrap();

        assert!(window.label(&host_ids, None).is_err());
        assert!(window.route(&host_ids, None).is_err());
    }

    #[test]
    fn editor_window_uses_target_route_and_stable_label() {
        let host_ids = vec!["host_123-abc".to_string()];
        let mut query = HashMap::new();
        query.insert("mode".to_string(), "file".to_string());
        query.insert("path".to_string(), "/home/deploy/app config.toml".to_string());
        let window = ManagedWindow::parse("editor").unwrap();

        assert_eq!(
            window.label(&host_ids, Some(&query)).unwrap(),
            format!(
                "editor-host_123-abc-{}",
                editor_target_hash("file", "/home/deploy/app config.toml")
            )
        );
        assert_eq!(
            window.route(&host_ids, Some(&query)).unwrap(),
            "/editor?hostId=host_123-abc&mode=file&path=%2Fhome%2Fdeploy%2Fapp%20config.toml"
        );
        assert!(window.requires_hosts());
    }

    #[test]
    fn editor_window_rejects_invalid_query() {
        let host_ids = vec!["host_123-abc".to_string()];
        let window = ManagedWindow::parse("editor").unwrap();
        let mut query = HashMap::new();
        query.insert("mode".to_string(), "file".to_string());
        query.insert("path".to_string(), "../etc/passwd".to_string());

        assert!(window.route(&host_ids, Some(&query)).is_err());
    }
}
