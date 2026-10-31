//! RDP drive redirection (MS-RDPEFS): one local folder shows up on the remote machine as a drive.
//!
//! `ironrdp-rdpdr` carries the protocol (capabilities, device announce, request decoding, response encoding); this is
//! the file system behind it, the part FreeRDP calls `drive_file.c`. The remote machine sends I/O requests (create,
//! read, write, query, list a directory, rename, delete) and every one is answered from the shared folder.
//!
//! Safety: a request never leaves the shared folder. Paths with `..`, NTFS stream names and anything that resolves
//! (through a symbolic link, too) outside of the root are refused.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ironrdp::pdu::PduResult;
use ironrdp::rdpdr::pdu::efs::{
    Boolean, Characteristics, ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse, CreateDisposition, CreateOptions,
    DesiredAccess, DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse,
    DeviceCreateRequest, DeviceCreateResponse, DeviceIoResponse, DeviceReadRequest, DeviceReadResponse,
    DeviceWriteRequest, DeviceWriteResponse, FileAttributeTagInformation, FileAttributes, FileBasicInformation,
    FileBothDirectoryInformation, FileDirectoryInformation, FileFsAttributeInformation, FileFsDeviceInformation,
    FileFsFullSizeInformation, FileFsSizeInformation, FileFsVolumeInformation, FileFullDirectoryInformation,
    FileInformationClass, FileInformationClassLevel, FileNamesInformation, FileRenameInformation,
    FileStandardInformation, FileSystemAttributes, FileSystemInformationClass, FileSystemInformationClassLevel,
    Information, NtStatus, ServerDeviceAnnounceResponse, ServerDriveIoRequest, ServerDriveLockControlRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest, ServerDriveQueryVolumeInformationRequest,
    ServerDriveSetInformationRequest,
};
use ironrdp::rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp::rdpdr::pdu::RdpdrPdu;
use ironrdp::rdpdr::RdpdrBackend;
use ironrdp::svc::{impl_as_any, SvcMessage};

/// The device id the shared folder is announced under.
pub(crate) const DRIVE_DEVICE_ID: u32 = 1;

const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_OBJECT_NAME_INVALID: u32 = 0xC000_0033;
const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
const STATUS_DISK_FULL: u32 = 0xC000_007F;
const STATUS_FILE_IS_A_DIRECTORY: u32 = 0xC000_00BA;

/// One read never returns more than this (the remote machine asks for what its application wants, up to 4 GiB).
const MAX_READ: u32 = 1 << 20;
/// Seconds between 1601-01-01 (the Windows epoch) and 1970-01-01.
const WINDOWS_TO_UNIX_SECS: u64 = 11_644_473_600;
const BYTES_PER_SECTOR: u32 = 512;
const SECTORS_PER_UNIT: u32 = 8;

#[derive(Debug)]
pub(crate) struct DriveBackend {
    root: PathBuf,
    label: String,
    open: HashMap<u32, Open>,
    next_file_id: u32,
}

impl_as_any!(DriveBackend);

#[derive(Debug)]
struct Open {
    path: PathBuf,
    node: Node,
    delete_on_close: bool,
}

#[derive(Debug)]
enum Node {
    File(File),
    /// `pending` is the listing of the directory still to be handed out, one entry per request.
    Dir { pending: Option<VecDeque<DirEntry>> },
}

#[derive(Debug, Clone)]
struct DirEntry {
    name: String,
    meta: Meta,
}

#[derive(Debug, Clone, Copy)]
struct Meta {
    is_dir: bool,
    size: u64,
    created: i64,
    accessed: i64,
    written: i64,
    readonly: bool,
    hidden: bool,
}

impl DriveBackend {
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| format!("共享目录不可用 {}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("共享路径不是目录: {}", root.display()));
        }
        let label = root.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "OpsBatch".to_string());
        Ok(Self { root, label, open: HashMap::new(), next_file_id: 1 })
    }

    /// The name the remote machine shows for the drive.
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    fn handle(&mut self, req: ServerDriveIoRequest) -> Vec<RdpdrPdu> {
        match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(r) => vec![self.create(r)],
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(r) => vec![self.query_information(r)],
            ServerDriveIoRequest::DeviceCloseRequest(r) => vec![self.close(r)],
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => vec![self.query_directory(r)],
            // A change notification stays pending until something changes; there is nothing to watch, so it is never answered.
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => Vec::new(),
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(r) => vec![self.query_volume(r)],
            ServerDriveIoRequest::DeviceControlRequest(r) => vec![device_control(r)],
            ServerDriveIoRequest::DeviceReadRequest(r) => vec![self.read(r)],
            ServerDriveIoRequest::DeviceWriteRequest(r) => vec![self.write(r)],
            ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => vec![self.set_information(r)],
            ServerDriveIoRequest::ServerDriveLockControlRequest(r) => vec![lock_control(r)],
        }
    }

    // ---- paths ----

    /// The local path of a path the remote machine sent (`\dir\file.txt`), or why it is refused.
    fn resolve(&self, remote: &str) -> Result<PathBuf, NtStatus> {
        let mut current = self.root.clone();
        for part in remote.split(['\\', '/']) {
            match part {
                "" | "." => continue,
                ".." => return Err(NtStatus::ACCESS_DENIED),
                p if p.contains(':') || p.contains('\0') => return Err(NtStatus::from(STATUS_OBJECT_NAME_INVALID)),
                _ => {}
            }
            let direct = current.join(part);
            // Windows looks names up without regard to case; a case-sensitive disk has to be searched for the match.
            current = if direct.symlink_metadata().is_ok() { direct } else { case_insensitive_child(&current, part).unwrap_or(direct) };
        }
        if self.inside_root(&current) {
            Ok(current)
        } else {
            Err(NtStatus::ACCESS_DENIED)
        }
    }

    /// Whether what `path` really is (symbolic links followed) lies in the shared folder. A path that does not exist
    /// yet counts by its closest existing parent.
    fn inside_root(&self, path: &Path) -> bool {
        let mut probe = path;
        loop {
            match probe.canonicalize() {
                Ok(real) => return real.starts_with(&self.root),
                Err(_) => {
                    // a link whose target is missing could be created through: refuse it
                    if probe.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                        return false;
                    }
                    match probe.parent() {
                        Some(parent) => probe = parent,
                        None => return false,
                    }
                }
            }
        }
    }

    fn insert(&mut self, path: PathBuf, node: Node, delete_on_close: bool) -> u32 {
        let id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1).max(1);
        self.open.insert(id, Open { path, node, delete_on_close });
        id
    }

    // ---- requests ----

    fn create(&mut self, r: DeviceCreateRequest) -> RdpdrPdu {
        let io = r.device_io_request.clone();
        let reply = |status: NtStatus, file_id: u32, information: Information| {
            RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse { device_io_reply: DeviceIoResponse::new(io.clone(), status), file_id, information })
        };
        let path = match self.resolve(&r.path) {
            Ok(p) => p,
            Err(status) => return reply(status, 0, Information::empty()),
        };
        let wants_directory = r.create_options.contains(CreateOptions::FILE_DIRECTORY_FILE);
        let forbids_directory = r.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE);
        let delete_on_close = r.create_options.contains(CreateOptions::FILE_DELETE_ON_CLOSE);
        let disposition = r.create_disposition;
        let wants_write = r.desired_access.intersects(
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY | DesiredAccess::GENERIC_WRITE | DesiredAccess::GENERIC_ALL,
        );

        match fs::metadata(&path) {
            Ok(m) if m.is_dir() => {
                if forbids_directory {
                    return reply(NtStatus::from(STATUS_FILE_IS_A_DIRECTORY), 0, Information::empty());
                }
                if disposition == CreateDisposition::FILE_CREATE {
                    return reply(NtStatus::OBJECT_NAME_COLLISION, 0, Information::empty());
                }
                if matches!(disposition, CreateDisposition::FILE_OVERWRITE | CreateDisposition::FILE_OVERWRITE_IF | CreateDisposition::FILE_SUPERSEDE) {
                    return reply(NtStatus::ACCESS_DENIED, 0, Information::empty());
                }
                let id = self.insert(path, Node::Dir { pending: None }, delete_on_close);
                reply(NtStatus::SUCCESS, id, Information::FILE_OPENED)
            }
            Ok(_) => {
                if wants_directory {
                    return reply(NtStatus::NOT_A_DIRECTORY, 0, Information::empty());
                }
                if disposition == CreateDisposition::FILE_CREATE {
                    return reply(NtStatus::OBJECT_NAME_COLLISION, 0, Information::empty());
                }
                let overwrite = matches!(disposition, CreateDisposition::FILE_OVERWRITE | CreateDisposition::FILE_OVERWRITE_IF | CreateDisposition::FILE_SUPERSEDE);
                let information = match disposition {
                    CreateDisposition::FILE_SUPERSEDE => Information::FILE_SUPERSEDED,
                    _ if overwrite => Information::FILE_OVERWRITTEN,
                    _ => Information::FILE_OPENED,
                };
                match OpenOptions::new().read(true).write(wants_write || overwrite).truncate(overwrite).open(&path) {
                    Ok(file) => {
                        let id = self.insert(path, Node::File(file), delete_on_close);
                        reply(NtStatus::SUCCESS, id, information)
                    }
                    Err(e) => reply(status_of(&e), 0, Information::empty()),
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if matches!(disposition, CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OVERWRITE) {
                    return reply(NtStatus::NO_SUCH_FILE, 0, Information::empty());
                }
                if wants_directory {
                    return match fs::create_dir(&path) {
                        Ok(()) => {
                            let id = self.insert(path, Node::Dir { pending: None }, delete_on_close);
                            reply(NtStatus::SUCCESS, id, Information::FILE_SUPERSEDED)
                        }
                        Err(e) => reply(create_status_of(&e), 0, Information::empty()),
                    };
                }
                match OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
                    Ok(file) => {
                        let id = self.insert(path, Node::File(file), delete_on_close);
                        reply(NtStatus::SUCCESS, id, Information::FILE_SUPERSEDED)
                    }
                    Err(e) => reply(create_status_of(&e), 0, Information::empty()),
                }
            }
            Err(e) => reply(status_of(&e), 0, Information::empty()),
        }
    }

    fn close(&mut self, r: DeviceCloseRequest) -> RdpdrPdu {
        let io = r.device_io_request;
        let status = match self.open.remove(&io.file_id) {
            None => NtStatus::from(STATUS_INVALID_HANDLE),
            Some(Open { path, node, delete_on_close }) => {
                let is_dir = matches!(node, Node::Dir { .. });
                // the file has to be closed before it can be removed (Windows)
                drop(node);
                if delete_on_close && path != self.root {
                    let result = if is_dir { fs::remove_dir(&path) } else { fs::remove_file(&path) };
                    result.err().map_or(NtStatus::SUCCESS, |e| status_of(&e))
                } else {
                    NtStatus::SUCCESS
                }
            }
        };
        RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse { device_io_response: DeviceIoResponse::new(io, status) })
    }

    fn read(&mut self, r: DeviceReadRequest) -> RdpdrPdu {
        let io = r.device_io_request;
        let reply = |status: NtStatus, read_data: Vec<u8>| RdpdrPdu::DeviceReadResponse(DeviceReadResponse { device_io_reply: DeviceIoResponse::new(io.clone(), status), read_data });
        let Some(Open { node: Node::File(file), .. }) = self.open.get_mut(&io.file_id) else {
            return reply(NtStatus::from(STATUS_INVALID_HANDLE), Vec::new());
        };
        let want = r.length.min(MAX_READ) as usize;
        let mut data = Vec::with_capacity(want.min(64 * 1024));
        let result = file.seek(SeekFrom::Start(r.offset)).and_then(|_| file.take(want as u64).read_to_end(&mut data));
        match result {
            Ok(_) => reply(NtStatus::SUCCESS, data),
            Err(e) => reply(status_of(&e), Vec::new()),
        }
    }

    fn write(&mut self, r: DeviceWriteRequest) -> RdpdrPdu {
        let io = r.device_io_request;
        let reply = |status: NtStatus, length: u32| RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse { device_io_reply: DeviceIoResponse::new(io.clone(), status), length });
        let Some(Open { node: Node::File(file), .. }) = self.open.get_mut(&io.file_id) else {
            return reply(NtStatus::from(STATUS_INVALID_HANDLE), 0);
        };
        match file.seek(SeekFrom::Start(r.offset)).and_then(|_| file.write_all(&r.write_data)) {
            Ok(()) => reply(NtStatus::SUCCESS, r.write_data.len() as u32),
            Err(e) => reply(status_of(&e), 0),
        }
    }

    fn query_information(&mut self, r: ServerDriveQueryInformationRequest) -> RdpdrPdu {
        let io = r.device_io_request;
        let reply = |status: NtStatus, buffer: Option<FileInformationClass>| {
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse { device_io_response: DeviceIoResponse::new(io.clone(), status), buffer })
        };
        let Some(open) = self.open.get(&io.file_id) else { return reply(NtStatus::from(STATUS_INVALID_HANDLE), None) };
        let name = open.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let delete_pending = open.delete_on_close;
        let meta = match Meta::read(&open.path, &name) {
            Ok(m) => m,
            Err(e) => return reply(status_of(&e), None),
        };
        let class = match r.file_info_class_lvl {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => FileInformationClass::Basic(FileBasicInformation {
                creation_time: meta.created,
                last_access_time: meta.accessed,
                last_write_time: meta.written,
                change_time: meta.written,
                file_attributes: meta.attributes(),
            }),
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => FileInformationClass::Standard(FileStandardInformation {
                allocation_size: meta.size as i64,
                end_of_file: meta.size as i64,
                number_of_links: 1,
                delete_pending: boolean(delete_pending),
                directory: boolean(meta.is_dir),
            }),
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => FileInformationClass::AttributeTag(FileAttributeTagInformation { file_attributes: meta.attributes(), reparse_tag: 0 }),
            _ => return reply(NtStatus::NOT_SUPPORTED, None),
        };
        reply(NtStatus::SUCCESS, Some(class))
    }

    fn query_directory(&mut self, r: ServerDriveQueryDirectoryRequest) -> RdpdrPdu {
        let io = r.device_io_request.clone();
        let reply = |status: NtStatus, buffer: Option<FileInformationClass>| {
            RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse { device_io_reply: DeviceIoResponse::new(io.clone(), status), buffer })
        };
        let is_root = self.open.get(&io.file_id).is_some_and(|o| o.path == self.root);
        let Some(open) = self.open.get_mut(&io.file_id) else { return reply(NtStatus::from(STATUS_INVALID_HANDLE), None) };
        let Node::Dir { pending } = &mut open.node else { return reply(NtStatus::NOT_A_DIRECTORY, None) };
        if r.initial_query != 0 || pending.is_none() {
            // the pattern is the last part of the path the request carries (`\*`, `\sub\*.txt`)
            let pattern = r.path.rsplit(['\\', '/']).next().filter(|p| !p.is_empty()).unwrap_or("*");
            match list_directory(&open.path, is_root, pattern) {
                Ok(entries) if entries.is_empty() => {
                    *pending = Some(VecDeque::new());
                    return reply(NtStatus::NO_SUCH_FILE, None);
                }
                Ok(entries) => *pending = Some(entries),
                Err(e) => return reply(status_of(&e), None),
            }
        }
        match pending.as_mut().and_then(VecDeque::pop_front) {
            None => reply(NtStatus::NO_MORE_FILES, None),
            Some(entry) => reply(NtStatus::SUCCESS, Some(directory_class(&r.file_info_class_lvl, entry))),
        }
    }

    fn query_volume(&mut self, r: ServerDriveQueryVolumeInformationRequest) -> RdpdrPdu {
        let status = NtStatus::SUCCESS;
        let io = r.device_io_request.clone();
        let unit = u64::from(BYTES_PER_SECTOR) * u64::from(SECTORS_PER_UNIT);
        let (total, available) = (fs4::total_space(&self.root).unwrap_or(1 << 40), fs4::available_space(&self.root).unwrap_or(1 << 39));
        let class = match r.fs_info_class_lvl {
            FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => FileSystemInformationClass::FileFsVolumeInformation(FileFsVolumeInformation {
                volume_creation_time: Meta::read(&self.root, "").map(|m| m.created).unwrap_or(0),
                volume_serial_number: 0x0b5b_a7c4,
                supports_objects: Boolean::False,
                volume_label: self.label.clone(),
            }),
            FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => FileSystemInformationClass::FileFsSizeInformation(FileFsSizeInformation {
                total_alloc_units: (total / unit) as i64,
                available_alloc_units: (available / unit) as i64,
                sectors_per_alloc_unit: SECTORS_PER_UNIT,
                bytes_per_sector: BYTES_PER_SECTOR,
            }),
            FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => FileSystemInformationClass::FileFsFullSizeInformation(FileFsFullSizeInformation {
                total_alloc_units: (total / unit) as i64,
                caller_available_alloc_units: (available / unit) as i64,
                actual_available_alloc_units: (available / unit) as i64,
                sectors_per_alloc_unit: SECTORS_PER_UNIT,
                bytes_per_sector: BYTES_PER_SECTOR,
            }),
            FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION => FileSystemInformationClass::FileFsAttributeInformation(FileFsAttributeInformation {
                file_system_attributes: FileSystemAttributes::FILE_CASE_PRESERVED_NAMES | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                max_component_name_len: 255,
                file_system_name: "NTFS".to_string(),
            }),
            FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION => FileSystemInformationClass::FileFsDeviceInformation(FileFsDeviceInformation {
                // FILE_DEVICE_DISK
                device_type: 7,
                characteristics: Characteristics::empty(),
            }),
            _ => return RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse::new(io, NtStatus::NOT_SUPPORTED, None)),
        };
        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse::new(io, status, Some(class)))
    }

    fn set_information(&mut self, r: ServerDriveSetInformationRequest) -> RdpdrPdu {
        let status = self.set_information_status(&r);
        ClientDriveSetInformationResponse::new(&r, status).map(RdpdrPdu::ClientDriveSetInformationResponse).unwrap_or(RdpdrPdu::EmptyResponse)
    }

    fn set_information_status(&mut self, r: &ServerDriveSetInformationRequest) -> NtStatus {
        let id = r.device_io_request.file_id;
        if let FileInformationClass::Rename(rename) = &r.set_buffer {
            return self.rename(id, rename);
        }
        let Some(open) = self.open.get_mut(&id) else { return NtStatus::from(STATUS_INVALID_HANDLE) };
        match &r.set_buffer {
            FileInformationClass::Basic(b) => {
                let mut status = NtStatus::SUCCESS;
                let attributes = b.file_attributes.bits();
                if attributes != 0 {
                    let readonly = attributes & FileAttributes::FILE_ATTRIBUTE_READONLY.bits() != 0;
                    if let Err(e) = set_readonly(&open.path, readonly) {
                        status = status_of(&e);
                    }
                }
                if let (Node::File(file), Some(time)) = (&open.node, filetime_to_system(b.last_write_time)) {
                    // not being able to set a time is not worth failing a copy for
                    let _ = file.set_modified(time);
                }
                status
            }
            FileInformationClass::EndOfFile(e) => match &open.node {
                Node::File(file) => file.set_len(e.end_of_file.max(0) as u64).err().map_or(NtStatus::SUCCESS, |e| status_of(&e)),
                Node::Dir { .. } => NtStatus::ACCESS_DENIED,
            },
            FileInformationClass::Allocation(_) => NtStatus::SUCCESS,
            FileInformationClass::Disposition(d) => {
                if d.delete_pending == 0 {
                    open.delete_on_close = false;
                    return NtStatus::SUCCESS;
                }
                if open.path == self.root {
                    return NtStatus::ACCESS_DENIED;
                }
                if matches!(open.node, Node::Dir { .. }) && fs::read_dir(&open.path).map(|mut d| d.next().is_some()).unwrap_or(false) {
                    return NtStatus::DIRECTORY_NOT_EMPTY;
                }
                open.delete_on_close = true;
                NtStatus::SUCCESS
            }
            _ => NtStatus::NOT_SUPPORTED,
        }
    }

    fn rename(&mut self, id: u32, rename: &FileRenameInformation) -> NtStatus {
        let target = match self.resolve(&rename.file_name) {
            Ok(t) => t,
            Err(status) => return status,
        };
        let Some(open) = self.open.get_mut(&id) else { return NtStatus::from(STATUS_INVALID_HANDLE) };
        if open.path == self.root || target == self.root {
            return NtStatus::ACCESS_DENIED;
        }
        if target.exists() && target != open.path {
            let replace = rename.replace_if_exists == Boolean::True;
            if !replace || target.is_dir() {
                return NtStatus::OBJECT_NAME_COLLISION;
            }
        }
        match fs::rename(&open.path, &target) {
            Ok(()) => {
                open.path = target;
                NtStatus::SUCCESS
            }
            Err(e) => status_of(&e),
        }
    }
}

impl RdpdrBackend for DriveBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(self.handle(req).into_iter().map(SvcMessage::from).collect())
    }
}

/// Device control codes are of no use to a folder; answering "done" keeps the remote machine going (what FreeRDP does).
fn device_control(r: DeviceControlRequest<ironrdp::rdpdr::pdu::efs::AnyIoCtlCode>) -> RdpdrPdu {
    RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(r, NtStatus::SUCCESS, None))
}

/// File locks are not enforced; the lock request is answered as done.
fn lock_control(r: ServerDriveLockControlRequest) -> RdpdrPdu {
    RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse { device_io_response: DeviceIoResponse::new(r.device_io_request, NtStatus::SUCCESS) })
}

// ---- helpers ----

fn boolean(value: bool) -> Boolean {
    if value {
        Boolean::True
    } else {
        Boolean::False
    }
}

fn status_of(e: &io::Error) -> NtStatus {
    match e.kind() {
        io::ErrorKind::NotFound => NtStatus::NO_SUCH_FILE,
        io::ErrorKind::PermissionDenied => NtStatus::ACCESS_DENIED,
        io::ErrorKind::AlreadyExists => NtStatus::OBJECT_NAME_COLLISION,
        io::ErrorKind::DirectoryNotEmpty => NtStatus::DIRECTORY_NOT_EMPTY,
        io::ErrorKind::NotADirectory => NtStatus::NOT_A_DIRECTORY,
        io::ErrorKind::IsADirectory => NtStatus::from(STATUS_FILE_IS_A_DIRECTORY),
        io::ErrorKind::StorageFull => NtStatus::from(STATUS_DISK_FULL),
        _ => NtStatus::UNSUCCESSFUL,
    }
}

/// Like [`status_of`], but a missing parent while creating is "path not found".
fn create_status_of(e: &io::Error) -> NtStatus {
    if e.kind() == io::ErrorKind::NotFound {
        NtStatus::from(STATUS_OBJECT_PATH_NOT_FOUND)
    } else {
        status_of(e)
    }
}

fn case_insensitive_child(dir: &Path, name: &str) -> Option<PathBuf> {
    let wanted = name.to_lowercase();
    fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == wanted).map(|e| e.path())
}

fn system_to_filetime(time: io::Result<SystemTime>) -> i64 {
    let since_unix = time.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).unwrap_or_default();
    ((since_unix.as_secs() + WINDOWS_TO_UNIX_SECS) * 10_000_000 + u64::from(since_unix.subsec_nanos() / 100)) as i64
}

/// `None` for the "leave it as it is" values a remote machine sends (0 and -1) and for times before 1970.
fn filetime_to_system(filetime: i64) -> Option<SystemTime> {
    let ticks = u64::try_from(filetime).ok().filter(|t| *t > 0)?;
    let secs = (ticks / 10_000_000).checked_sub(WINDOWS_TO_UNIX_SECS)?;
    Some(UNIX_EPOCH + Duration::new(secs, ((ticks % 10_000_000) * 100) as u32))
}

fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = permissions.mode();
        // only the owner's write bit: set_readonly(false) would make the file writable for everybody
        permissions.set_mode(if readonly { mode & !0o222 } else { mode | 0o200 });
    }
    #[cfg(not(unix))]
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions)
}

impl Meta {
    fn read(path: &Path, name: &str) -> io::Result<Self> {
        let m = fs::metadata(path)?;
        let written = system_to_filetime(m.modified());
        Ok(Self {
            is_dir: m.is_dir(),
            size: if m.is_dir() { 0 } else { m.len() },
            created: m.created().map(|t| system_to_filetime(Ok(t))).unwrap_or(written),
            accessed: m.accessed().map(|t| system_to_filetime(Ok(t))).unwrap_or(written),
            written,
            readonly: m.permissions().readonly(),
            hidden: name.starts_with('.') && name != "." && name != "..",
        })
    }

    fn attributes(&self) -> FileAttributes {
        let mut attributes = if self.is_dir { FileAttributes::FILE_ATTRIBUTE_DIRECTORY } else { FileAttributes::empty() };
        if self.readonly && !self.is_dir {
            attributes |= FileAttributes::FILE_ATTRIBUTE_READONLY;
        }
        if self.hidden {
            attributes |= FileAttributes::FILE_ATTRIBUTE_HIDDEN;
        }
        if attributes.is_empty() {
            attributes = FileAttributes::FILE_ATTRIBUTE_NORMAL;
        }
        attributes
    }
}

/// The entries of a directory that match `pattern`, `.` and `..` first (a root has neither), the rest by name.
fn list_directory(dir: &Path, is_root: bool, pattern: &str) -> io::Result<VecDeque<DirEntry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !wildcard_matches(pattern, &name) {
            continue;
        }
        // an entry that cannot be read (a dangling link, removed meanwhile) is left out
        if let Ok(meta) = Meta::read(&item.path(), &name) {
            entries.push(DirEntry { name, meta });
        }
    }
    entries.sort_by_key(|e| e.name.to_lowercase());
    let mut list = VecDeque::new();
    if !is_root {
        for dots in [".", ".."] {
            if wildcard_matches(pattern, dots) {
                if let Ok(meta) = Meta::read(dir, dots) {
                    list.push_back(DirEntry { name: dots.to_string(), meta: Meta { hidden: false, ..meta } });
                }
            }
        }
    }
    list.extend(entries);
    Ok(list)
}

fn directory_class(level: &FileInformationClassLevel, entry: DirEntry) -> FileInformationClass {
    let m = entry.meta;
    let (created, accessed, written, change, size, attributes, name) = (m.created, m.accessed, m.written, m.written, m.size as i64, m.attributes(), entry.name);
    match *level {
        FileInformationClassLevel::FILE_NAMES_INFORMATION => FileInformationClass::Names(FileNamesInformation::new(name)),
        FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => FileInformationClass::FullDirectory(FileFullDirectoryInformation::new(created, accessed, written, change, size, attributes, name)),
        FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => FileInformationClass::BothDirectory(FileBothDirectoryInformation::new(created, accessed, written, change, size, attributes, name)),
        _ => FileInformationClass::Directory(FileDirectoryInformation::new(created, accessed, written, change, size, attributes, name)),
    }
}

/// DOS-style wildcard match, case-insensitive: `*` any run, `?` any one character (`*.*` is "everything", as on Windows).
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*.*" {
        return true;
    }
    let p: Vec<char> = pattern.to_lowercase().chars().map(|c| match c {
        '<' => '*',
        '>' | '"' => '?',
        c => c,
    }).collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests;
