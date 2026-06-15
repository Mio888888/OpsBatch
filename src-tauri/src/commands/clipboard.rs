use arboard::{Clipboard, Error as ClipboardError};

fn clipboard_error(action: &str, error: ClipboardError) -> String {
    format!("failed to {action} local clipboard text: {error}")
}

#[tauri::command]
pub fn read_local_clipboard_text() -> Result<Option<String>, String> {
    let mut clipboard = Clipboard::new().map_err(|error| clipboard_error("open", error))?;

    match clipboard.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(ClipboardError::ContentNotAvailable) => Ok(None),
        Err(error) => Err(clipboard_error("read", error)),
    }
}

#[tauri::command]
pub fn write_local_clipboard_text(text: String) -> Result<(), String> {
    Clipboard::new()
        .map_err(|error| clipboard_error("open", error))?
        .set_text(text)
        .map_err(|error| clipboard_error("write", error))
}
