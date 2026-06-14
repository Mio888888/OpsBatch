use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use arboard::Clipboard;
use ironrdp::cliprdr::backend::CliprdrBackend;
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest,
    FormatDataResponse, LockDataId, OwnedFileContentsResponse, OwnedFormatDataResponse,
};
use ironrdp::core::{AsAny, IntoOwned};

use super::clipboard_files::read_clipboard_file_paths;

#[derive(Debug, Clone)]
pub(super) struct ClipboardBridge {
    inner: Arc<Mutex<ClipboardState>>,
}

#[derive(Debug, Default)]
struct ClipboardState {
    actions: VecDeque<ClipboardAction>,
    last_local_text: Option<String>,
    last_local_files: Vec<String>,
    pending_remote_format: Option<ClipboardFormatId>,
    format_list_accepted: bool,
}

#[derive(Debug)]
pub(super) enum ClipboardAction {
    // 文本剪贴板
    AdvertiseText(bool),
    RequestRemoteText(ClipboardFormatId),
    SendTextResponse(OwnedFormatDataResponse),
    // 文件上传（本地 → 远程）
    InitiateFileCopy(Vec<FileDescriptor>),
    ServeFileContentsRequest(FileContentsRequest),
    // 本地复制文件（宿主机 Cmd+C → 远程 Ctrl+V）
    AdvertiseLocalFiles(Vec<String>),
    // 文件下载（远程 → 本地）
    RequestRemoteFileList(ClipboardFormatId),
    StartFileDownload {
        files: Vec<FileDescriptor>,
        clip_data_id: Option<u32>,
    },
    ProcessFileContentsResponse(OwnedFileContentsResponse),
}

impl ClipboardBridge {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClipboardState::default())),
        }
    }

    pub(super) fn poll_local_clipboard(&self) {
        let text = read_clipboard_text();
        let file_paths = read_clipboard_file_paths();
        let file_path_strings: Vec<String> = file_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        // 文本变化检测
        if state.last_local_text != text {
            state.last_local_text = text.clone();
            state
                .actions
                .push_back(ClipboardAction::AdvertiseText(text.is_some()));
        }

        // 文件变化检测：本地复制的文件 → 广告到远程剪贴板
        if state.last_local_files != file_path_strings {
            state.last_local_files = file_path_strings.clone();
            if !file_path_strings.is_empty() {
                state
                    .actions
                    .push_back(ClipboardAction::AdvertiseLocalFiles(file_path_strings));
            }
        }
    }

    pub(super) fn drain_actions(&self) -> Vec<ClipboardAction> {
        let Ok(mut state) = self.inner.lock() else {
            return Vec::new();
        };
        state.actions.drain(..).collect()
    }

    /// 检查远程是否已确认接受我们广告的剪贴板格式列表（含文件）
    pub(super) fn take_format_list_accepted(&self) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        std::mem::replace(&mut state.format_list_accepted, false)
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

    // ===== 文件传输方法 =====

    /// 前端拖拽上传：广告本地文件到远程
    pub(super) fn advertise_files(&self, files: Vec<FileDescriptor>) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::InitiateFileCopy(files));
    }

    /// 远端复制了文件：请求 FileGroupDescriptorW 格式的文件列表
    fn request_remote_file_list(&self, format: ClipboardFormatId) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::RequestRemoteFileList(format));
    }

    /// 收到远程文件列表元数据：开始下载
    fn start_file_download(&self, files: Vec<FileDescriptor>, clip_data_id: Option<u32>) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::StartFileDownload {
                files,
                clip_data_id,
            });
    }

    /// 远端请求上传方向的文件内容（本地 → 远程）
    fn serve_file_contents_request(&self, request: FileContentsRequest) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::ServeFileContentsRequest(request));
    }

    /// 收到下载方向的文件内容响应（远程 → 本地）
    fn process_file_contents_response(&self, response: FileContentsResponse<'_>) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state
            .actions
            .push_back(ClipboardAction::ProcessFileContentsResponse(response.into_owned()));
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
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        self.bridge.request_initial_format_list();
    }

    fn on_format_list_response(&mut self, ok: bool) {
        if ok {
            if let Ok(mut state) = self.bridge.inner.lock() {
                state.format_list_accepted = true;
            }
        }
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        _capabilities: ClipboardGeneralCapabilityFlags,
    ) {
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // 优先检查文件格式（FileGroupDescriptorW）
        if let Some(format) = available_formats.iter().find(|f| {
            f.name
                .as_ref()
                .is_some_and(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
        }) {
            self.bridge.request_remote_file_list(format.id);
            return;
        }
        // 回退到文本
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

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.bridge.serve_file_contents_request(request);
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.bridge.process_file_contents_response(response);
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.bridge.start_file_download(files.to_vec(), clip_data_id);
    }

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
