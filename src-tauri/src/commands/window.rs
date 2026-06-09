use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

enum ManagedWindow {
    BatchTerminal,
    BatchTransfer,
    Settings,
    GlobalLog,
}

impl ManagedWindow {
    fn parse(kind: &str) -> Result<Self, String> {
        match kind {
            "batch-terminal" => Ok(Self::BatchTerminal),
            "batch-transfer" => Ok(Self::BatchTransfer),
            "settings" => Ok(Self::Settings),
            "global-log" => Ok(Self::GlobalLog),
            _ => Err("window kind is not allowed".to_string()),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::BatchTerminal => "batch-terminal",
            Self::BatchTransfer => "batch-transfer",
            Self::Settings => "settings",
            Self::GlobalLog => "global-log",
        }
    }

    fn title(&self, host_count: usize) -> String {
        match self {
            Self::BatchTerminal => format!("批量终端 ({})", host_count),
            Self::BatchTransfer => format!("批量传输 ({})", host_count),
            Self::Settings => "设置".to_string(),
            Self::GlobalLog => "全局日志".to_string(),
        }
    }

    fn route(&self, host_ids: &[String]) -> Result<String, String> {
        match self {
            Self::BatchTerminal => Ok(format!(
                "/batch-terminal?hostIds={}",
                encode_host_ids(host_ids)?
            )),
            Self::BatchTransfer => Ok(format!(
                "/batch-transfer?hostIds={}",
                encode_host_ids(host_ids)?
            )),
            Self::Settings => Ok("/settings".to_string()),
            Self::GlobalLog => Ok("/global-log".to_string()),
        }
    }

    fn size(&self) -> (f64, f64, f64, f64) {
        match self {
            Self::BatchTerminal => (1200.0, 800.0, 800.0, 600.0),
            Self::BatchTransfer => (900.0, 700.0, 700.0, 550.0),
            Self::Settings => (900.0, 680.0, 760.0, 560.0),
            Self::GlobalLog => (720.0, 600.0, 500.0, 400.0),
        }
    }

    fn requires_hosts(&self) -> bool {
        matches!(self, Self::BatchTerminal | Self::BatchTransfer)
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

fn is_safe_host_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn spa_entry_for_route(route: &str) -> String {
    format!("index.html#{}", route)
}

#[tauri::command]
pub fn open_managed_window(
    app: tauri::AppHandle,
    kind: String,
    host_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let window = ManagedWindow::parse(&kind)?;
    let host_ids = host_ids.unwrap_or_default();
    if window.requires_hosts() && host_ids.is_empty() {
        return Err("host ids are required".to_string());
    }

    let label = window.label();
    if let Some(existing) = app.get_webview_window(label) {
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let (width, height, min_width, min_height) = window.size();
    let route = window.route(&host_ids)?;
    let url = spa_entry_for_route(&route);
    WebviewWindowBuilder::new(&app, label, WebviewUrl::App(url.into()))
        .title(window.title(host_ids.len()))
        .inner_size(width, height)
        .min_inner_size(min_width, min_height)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .build()
        .map_err(|e| e.to_string())?;

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
    fn spa_entry_keeps_route_in_hash_fragment() {
        assert_eq!(spa_entry_for_route("/settings"), "index.html#/settings");
        assert_eq!(
            spa_entry_for_route("/batch-terminal?hostIds=a_b-1"),
            "index.html#/batch-terminal?hostIds=a_b-1"
        );
    }
}
