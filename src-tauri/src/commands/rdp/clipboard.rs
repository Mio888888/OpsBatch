use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use arboard::Clipboard;
use ironrdp::cliprdr::backend::CliprdrBackend;
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp::core::AsAny;

#[derive(Debug, Clone)]
pub(super) struct ClipboardBridge {
    inner: Arc<Mutex<ClipboardState>>,
}

#[derive(Debug, Default)]
struct ClipboardState {
    actions: VecDeque<ClipboardAction>,
    last_local_text: Option<String>,
    pending_remote_format: Option<ClipboardFormatId>,
}

#[derive(Debug)]
pub(super) enum ClipboardAction {
    AdvertiseText(bool),
    RequestRemoteText(ClipboardFormatId),
    SendTextResponse(OwnedFormatDataResponse),
}

impl ClipboardBridge {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClipboardState::default())),
        }
    }

    pub(super) fn poll_local_clipboard(&self) {
        let text = read_clipboard_text();
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if state.last_local_text == text {
            return;
        }
        state.last_local_text = text.clone();
        state
            .actions
            .push_back(ClipboardAction::AdvertiseText(text.is_some()));
    }

    pub(super) fn drain_actions(&self) -> Vec<ClipboardAction> {
        let Ok(mut state) = self.inner.lock() else {
            return Vec::new();
        };
        state.actions.drain(..).collect()
    }

    fn request_initial_format_list(&self) {
        let text = read_clipboard_text();
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state.last_local_text = text.clone();
        state
            .actions
            .push_back(ClipboardAction::AdvertiseText(text.is_some()));
    }

    fn request_remote_text(&self, format: ClipboardFormatId) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state.pending_remote_format = Some(format);
        state
            .actions
            .push_back(ClipboardAction::RequestRemoteText(format));
    }

    fn send_local_text_response(&self, request: FormatDataRequest) {
        let response = match read_clipboard_text() {
            Some(text) if request.format == ClipboardFormatId::CF_TEXT => {
                OwnedFormatDataResponse::new_string(&text)
            }
            Some(text) => OwnedFormatDataResponse::new_unicode_string(&text),
            None => OwnedFormatDataResponse::new_error(),
        };
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::SendTextResponse(response));
    }

    fn store_remote_text(&self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        let format = {
            let Ok(mut state) = self.inner.lock() else {
                return;
            };
            state.pending_remote_format.take()
        };
        let text = match format {
            Some(ClipboardFormatId::CF_TEXT) => response.to_string().ok(),
            _ => response.to_unicode_string().ok(),
        };
        let Some(text) = text else {
            return;
        };
        if write_clipboard_text(&text).is_ok() {
            if let Ok(mut state) = self.inner.lock() {
                state.last_local_text = Some(text);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TextClipboardBackend {
    bridge: ClipboardBridge,
}

impl TextClipboardBackend {
    pub(super) fn new(bridge: ClipboardBridge) -> Self {
        Self { bridge }
    }
}

impl AsAny for TextClipboardBackend {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl CliprdrBackend for TextClipboardBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        self.bridge.request_initial_format_list();
    }

    fn on_format_list_response(&mut self, _ok: bool) {}

    fn on_process_negotiated_capabilities(
        &mut self,
        _capabilities: ClipboardGeneralCapabilityFlags,
    ) {
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if let Some(format) = preferred_text_format(available_formats) {
            self.bridge.request_remote_text(format);
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.bridge.send_local_text_response(request);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.bridge.store_remote_text(response);
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

pub(super) fn text_clipboard_formats(enabled: bool) -> Vec<ClipboardFormat> {
    if enabled {
        vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
    } else {
        Vec::new()
    }
}

fn preferred_text_format(formats: &[ClipboardFormat]) -> Option<ClipboardFormatId> {
    formats
        .iter()
        .find(|format| format.id == ClipboardFormatId::CF_UNICODETEXT)
        .or_else(|| {
            formats
                .iter()
                .find(|format| format.id == ClipboardFormatId::CF_TEXT)
        })
        .map(|format| format.id)
}

fn read_clipboard_text() -> Option<String> {
    Clipboard::new().ok()?.get_text().ok()
}

fn write_clipboard_text(text: &str) -> Result<(), arboard::Error> {
    Clipboard::new()?.set_text(text.to_string())
}
