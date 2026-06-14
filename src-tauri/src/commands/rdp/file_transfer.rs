//! RDP 文件传输状态管理
//!
//! 负责上传（本地→远程）和下载（远程→本地）两个方向的文件传输状态跟踪。
//! 上传方向：当本地文件通过拖拽或复制粘贴被广告到远程后，远程通过
//! FileContentsRequest 拉取文件数据；本模块负责读取本地文件块并构造响应。
//! 下载方向：当远程复制文件后，本模块逐文件、逐块（SIZE→RANGE）拉取
//! 数据并写入本地磁盘。

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor,
};

/// 单次 RANGE 请求的最大字节数（1 MB），兼顾吞吐与内存占用
pub(super) const FILE_TRANSFER_CHUNK_SIZE: u32 = 1_048_576;

/// 本地待上传文件元数据
#[derive(Debug, Clone)]
pub(super) struct UploadFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub last_write_time: u64,
}

/// 远程→本地下载进度跟踪
#[derive(Debug)]
pub(super) struct DownloadProgress {
    pub files: Vec<FileDescriptor>,
    pub current_index: usize,
    pub file_handle: Option<File>,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub bytes_written: u64,
    pub current_stream_id: u32,
    pub awaiting_size: bool,
    pub clip_data_id: Option<u32>,
    pub completed: Vec<String>,
    pub failed: Vec<String>,
}

/// 文件传输全局状态
#[derive(Debug)]
pub(super) struct FileTransferState {
    pub upload_files: Vec<UploadFile>,
    pub download_dir: PathBuf,
    pub next_stream_id: u32,
    pub download_progress: Option<DownloadProgress>,
    pub auto_paste_position: Option<(u16, u16)>,
}

impl FileTransferState {
    pub(super) fn new(download_dir: PathBuf) -> Self {
        Self {
            upload_files: Vec::new(),
            download_dir,
            next_stream_id: 1,
            download_progress: None,
            auto_paste_position: None,
        }
    }

}

impl DownloadProgress {
    pub(super) fn new(files: Vec<FileDescriptor>, clip_data_id: Option<u32>) -> Self {
        Self {
            files,
            current_index: 0,
            file_handle: None,
            file_path: PathBuf::new(),
            file_size: 0,
            bytes_written: 0,
            current_stream_id: 0,
            awaiting_size: false,
            clip_data_id,
            completed: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub(super) fn is_finished(&self) -> bool {
        self.current_index >= self.files.len()
    }

    pub(super) fn build_size_request(
        &mut self,
        next_stream_id: &mut u32,
    ) -> Option<FileContentsRequest> {
        if self.current_index >= self.files.len() {
            return None;
        }
        self.awaiting_size = true;
        self.current_stream_id = allocate_stream_id(next_stream_id);
        Some(FileContentsRequest {
            stream_id: self.current_stream_id,
            index: self.current_index as i32,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: self.clip_data_id,
        })
    }

    pub(super) fn handle_response(
        &mut self,
        response: &FileContentsResponse<'_>,
        download_dir: &Path,
        next_stream_id: &mut u32,
    ) -> Result<Option<FileContentsRequest>, String> {
        if response.stream_id() != self.current_stream_id {
            return Ok(None);
        }

        if response.is_error() {
            let name = self
                .files
                .get(self.current_index)
                .map(|f| f.name.clone())
                .unwrap_or_default();
            self.failed.push(name);
            self.file_handle = None;
            self.current_index += 1;
            return Ok(self.build_size_request(next_stream_id));
        }

        if self.awaiting_size {
            let size = response
                .data_as_size()
                .map_err(|e| format!("解析文件大小失败: {e}"))?;
            self.awaiting_size = false;
            self.file_size = size;
            self.bytes_written = 0;

            let file_name = &self.files[self.current_index].name;
            let output_path = unique_output_path(download_dir, file_name);
            self.file_path = output_path.clone();

            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&output_path)
                .map_err(|e| format!("创建下载文件失败 {}: {e}", output_path.display()))?;
            self.file_handle = Some(file);

            if size == 0 {
                self.file_handle = None;
                self.completed.push(file_name.clone());
                self.current_index += 1;
                return Ok(self.build_size_request(next_stream_id));
            }

            let chunk = FILE_TRANSFER_CHUNK_SIZE.min(size as u32);
            self.current_stream_id = allocate_stream_id(next_stream_id);
            return Ok(Some(FileContentsRequest {
                stream_id: self.current_stream_id,
                index: self.current_index as i32,
                flags: FileContentsFlags::RANGE,
                position: 0,
                requested_size: chunk,
                data_id: self.clip_data_id,
            }));
        }

        let data = response.data();
        if let Some(ref mut file) = self.file_handle {
            file.write_all(data)
                .map_err(|e| format!("写入下载文件失败: {e}"))?;
        }
        self.bytes_written += data.len() as u64;

        if self.bytes_written >= self.file_size {
            self.file_handle = None;
            let name = self.files[self.current_index].name.clone();
            self.completed.push(name);
            self.current_index += 1;
            return Ok(self.build_size_request(next_stream_id));
        }

        let remaining = self.file_size - self.bytes_written;
        let chunk = FILE_TRANSFER_CHUNK_SIZE.min(remaining as u32);
        self.current_stream_id = allocate_stream_id(next_stream_id);
        Ok(Some(FileContentsRequest {
            stream_id: self.current_stream_id,
            index: self.current_index as i32,
            flags: FileContentsFlags::RANGE,
            position: self.bytes_written,
            requested_size: chunk,
            data_id: self.clip_data_id,
        }))
    }
}

pub(super) fn collect_upload_files(paths: &[PathBuf]) -> Vec<UploadFile> {
    paths
        .iter()
        .filter_map(|path| {
            let metadata = std::fs::metadata(path).ok()?;
            if !metadata.is_file() {
                return None;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unnamed".to_string());
            Some(UploadFile {
                path: path.clone(),
                name,
                size: metadata.len(),
                last_write_time: system_time_to_filetime(metadata.modified().ok()),
            })
        })
        .collect()
}

pub(super) fn build_file_descriptors(files: &[UploadFile]) -> Vec<FileDescriptor> {
    files
        .iter()
        .map(|f| {
            FileDescriptor::new(f.name.clone())
                .with_attributes(ClipboardFileAttributes::NORMAL)
                .with_file_size(f.size)
                .with_last_write_time(f.last_write_time)
        })
        .collect()
}

pub(super) fn read_upload_file_chunk(
    request: &FileContentsRequest,
    upload_files: &[UploadFile],
) -> FileContentsResponse<'static> {
    let stream_id = request.stream_id;
    let index = match usize::try_from(request.index) {
        Ok(i) if i < upload_files.len() => i,
        _ => return FileContentsResponse::new_error(stream_id),
    };
    let file = &upload_files[index];

    if request.flags.contains(FileContentsFlags::SIZE) {
        return FileContentsResponse::new_size_response(stream_id, file.size);
    }

    if request.flags.contains(FileContentsFlags::RANGE) {
        let read_result = || -> Result<Vec<u8>, std::io::Error> {
            let mut f = File::open(&file.path)?;
            f.seek(SeekFrom::Start(request.position))?;
            let mut buf = vec![0u8; request.requested_size as usize];
            let n = f.read(&mut buf)?;
            buf.truncate(n);
            Ok(buf)
        };
        return match read_result() {
            Ok(data) => FileContentsResponse::new_data_response(stream_id, data),
            Err(_) => FileContentsResponse::new_error(stream_id),
        };
    }

    FileContentsResponse::new_error(stream_id)
}

fn allocate_stream_id(counter: &mut u32) -> u32 {
    let id = *counter;
    *counter = counter.wrapping_add(1);
    if *counter == 0 {
        *counter = 1;
    }
    id
}

fn system_time_to_filetime(time: Option<SystemTime>) -> u64 {
    let Some(time) = time else {
        return 0;
    };
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return 0;
    };
    let unix_seconds = duration.as_secs();
    (unix_seconds + 11_644_473_600) * 10_000_000 + u64::from(duration.subsec_nanos()) / 100
}

fn unique_output_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name)
        .extension()
        .map(|s| s.to_string_lossy().into_owned());

    for counter in 1..10_000u32 {
        let new_name = match &ext {
            Some(ext) => format!("{stem} ({counter}).{ext}"),
            None => format!("{stem} ({counter})"),
        };
        let candidate = dir.join(&new_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!(
        "{}_{}.{}",
        stem,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        ext.unwrap_or_default()
    ))
}
