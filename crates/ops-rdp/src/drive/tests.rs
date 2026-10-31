use std::path::PathBuf;

use ironrdp::rdpdr::pdu::efs::{
    DeviceIoRequest, FileDispositionInformation, FileEndOfFileInformation, MajorFunction, MinorFunction, SharedAccess,
};
use ironrdp::rdpdr::Rdpdr;
use ironrdp::svc::SvcProcessor;

use super::*;

/// A throw-away shared folder and the backend serving it.
struct Share {
    dir: PathBuf,
    backend: DriveBackend,
    completion: u32,
}

impl Share {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("opsbatch-drive-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let backend = DriveBackend::new(&dir).unwrap();
        Self { dir, backend, completion: 0 }
    }

    fn io(&mut self, major: MajorFunction, file_id: u32) -> DeviceIoRequest {
        self.completion += 1;
        DeviceIoRequest { device_id: DRIVE_DEVICE_ID, file_id, completion_id: self.completion, major_function: major, minor_function: MinorFunction::from(0) }
    }

    fn create(&mut self, path: &str, disposition: CreateDisposition, options: CreateOptions) -> (NtStatus, u32) {
        let device_io_request = self.io(MajorFunction::Create, 0);
        let request = DeviceCreateRequest {
            device_io_request,
            desired_access: DesiredAccess::from_bits_retain(0x0012_019f),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: disposition,
            create_options: options,
            path: path.to_string(),
        };
        match self.backend.handle(ServerDriveIoRequest::ServerCreateDriveRequest(request)).remove(0) {
            RdpdrPdu::DeviceCreateResponse(r) => (r.device_io_reply.io_status, r.file_id),
            other => panic!("not a create response: {}", name_of(&other)),
        }
    }

    fn open_file(&mut self, path: &str, disposition: CreateDisposition) -> u32 {
        let (status, id) = self.create(path, disposition, CreateOptions::FILE_NON_DIRECTORY_FILE);
        assert_eq!(status, NtStatus::SUCCESS, "{path}");
        id
    }

    fn close(&mut self, file_id: u32) -> NtStatus {
        let device_io_request = self.io(MajorFunction::Close, file_id);
        match self.backend.handle(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest { device_io_request })).remove(0) {
            RdpdrPdu::DeviceCloseResponse(r) => r.device_io_response.io_status,
            other => panic!("not a close response: {}", name_of(&other)),
        }
    }

    fn write(&mut self, file_id: u32, offset: u64, data: &[u8]) -> (NtStatus, u32) {
        let device_io_request = self.io(MajorFunction::Write, file_id);
        match self.backend.handle(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest { device_io_request, offset, write_data: data.to_vec() })).remove(0) {
            RdpdrPdu::DeviceWriteResponse(r) => (r.device_io_reply.io_status, r.length),
            other => panic!("not a write response: {}", name_of(&other)),
        }
    }

    fn read(&mut self, file_id: u32, offset: u64, length: u32) -> (NtStatus, Vec<u8>) {
        let device_io_request = self.io(MajorFunction::Read, file_id);
        match self.backend.handle(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest { device_io_request, length, offset })).remove(0) {
            RdpdrPdu::DeviceReadResponse(r) => (r.device_io_reply.io_status, r.read_data),
            other => panic!("not a read response: {}", name_of(&other)),
        }
    }

    fn query(&mut self, file_id: u32, level: FileInformationClassLevel) -> (NtStatus, Option<FileInformationClass>) {
        let device_io_request = self.io(MajorFunction::QueryInformation, file_id);
        match self.backend.handle(ServerDriveIoRequest::ServerDriveQueryInformationRequest(ServerDriveQueryInformationRequest { device_io_request, file_info_class_lvl: level })).remove(0) {
            RdpdrPdu::ClientDriveQueryInformationResponse(r) => (r.device_io_response.io_status, r.buffer),
            other => panic!("not a query response: {}", name_of(&other)),
        }
    }

    fn list(&mut self, dir_id: u32, pattern: &str, initial: bool, level: FileInformationClassLevel) -> (NtStatus, Option<FileInformationClass>) {
        let device_io_request = self.io(MajorFunction::DirectoryControl, dir_id);
        let request = ServerDriveQueryDirectoryRequest { device_io_request, file_info_class_lvl: level, initial_query: initial as u8, path: pattern.to_string() };
        match self.backend.handle(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(request)).remove(0) {
            RdpdrPdu::ClientDriveQueryDirectoryResponse(r) => (r.device_io_reply.io_status, r.buffer),
            other => panic!("not a directory response: {}", name_of(&other)),
        }
    }

    /// Every name a directory listing hands out, in order.
    fn names(&mut self, path: &str, pattern: &str) -> Vec<String> {
        let (status, id) = self.create(path, CreateDisposition::FILE_OPEN, CreateOptions::FILE_DIRECTORY_FILE);
        assert_eq!(status, NtStatus::SUCCESS, "{path}");
        let mut names = Vec::new();
        let mut initial = true;
        loop {
            let (status, buffer) = self.list(id, pattern, initial, FileInformationClassLevel::FILE_NAMES_INFORMATION);
            initial = false;
            match buffer {
                Some(FileInformationClass::Names(n)) => {
                    assert_eq!(status, NtStatus::SUCCESS);
                    names.push(n.file_name);
                }
                _ => {
                    assert!(status == NtStatus::NO_MORE_FILES || status == NtStatus::NO_SUCH_FILE, "{status:?}");
                    break;
                }
            }
        }
        self.close(id);
        names
    }

    fn set(&mut self, file_id: u32, buffer: FileInformationClass) -> NtStatus {
        let device_io_request = self.io(MajorFunction::SetInformation, file_id);
        match self.backend.handle(ServerDriveIoRequest::ServerDriveSetInformationRequest(ServerDriveSetInformationRequest { device_io_request, set_buffer: buffer })).remove(0) {
            RdpdrPdu::ClientDriveSetInformationResponse(r) => status_of_set(&r),
            other => panic!("not a set-information response: {}", name_of(&other)),
        }
    }

    fn rename(&mut self, file_id: u32, to: &str, replace: bool) -> NtStatus {
        self.set(file_id, FileInformationClass::Rename(FileRenameInformation { replace_if_exists: boolean(replace), file_name: to.to_string() }))
    }

    fn volume(&mut self, level: FileSystemInformationClassLevel) -> FileSystemInformationClass {
        let device_io_request = self.io(MajorFunction::QueryVolumeInformation, 0);
        match self.backend.handle(ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(ServerDriveQueryVolumeInformationRequest { device_io_request, fs_info_class_lvl: level })).remove(0) {
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(r) => r.buffer.expect("volume information"),
            other => panic!("not a volume response: {}", name_of(&other)),
        }
    }

    fn put(&self, relative: &str, content: &str) {
        let path = self.dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn name_of(pdu: &RdpdrPdu) -> &'static str {
    match pdu {
        RdpdrPdu::DeviceCreateResponse(_) => "create",
        RdpdrPdu::EmptyResponse => "empty",
        _ => "other",
    }
}

/// The status of a set-information response, read back from its encoding (the field is private).
fn status_of_set(r: &ClientDriveSetInformationResponse) -> NtStatus {
    let bytes = ironrdp::core::encode_vec(&RdpdrPdu::ClientDriveSetInformationResponse(r.clone())).unwrap();
    // shared header (4) + device id (4) + completion id (4) + io status (4)
    NtStatus::from(u32::from_le_bytes(bytes[12..16].try_into().unwrap()))
}

fn open_dir(share: &mut Share, path: &str) -> u32 {
    let (status, id) = share.create(path, CreateDisposition::FILE_OPEN, CreateOptions::FILE_DIRECTORY_FILE);
    assert_eq!(status, NtStatus::SUCCESS, "{path}");
    id
}

#[test]
fn paths_never_leave_the_shared_folder() {
    let share = Share::new();
    assert_eq!(share.backend.resolve("\\..\\secret"), Err(NtStatus::ACCESS_DENIED));
    assert_eq!(share.backend.resolve("\\a\\..\\..\\x"), Err(NtStatus::ACCESS_DENIED));
    assert_eq!(share.backend.resolve("..").unwrap_err(), NtStatus::ACCESS_DENIED);
    assert_eq!(share.backend.resolve("\\file.txt:stream"), Err(NtStatus::from(STATUS_OBJECT_NAME_INVALID)));
    assert_eq!(share.backend.resolve("\\"), Ok(share.backend.root.clone()));
    assert_eq!(share.backend.resolve("\\a\\b.txt"), Ok(share.backend.root.join("a").join("b.txt")));
    assert_eq!(share.backend.resolve("/a/./b.txt"), Ok(share.backend.root.join("a").join("b.txt")));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_cannot_lead_out_of_the_shared_folder() {
    let outside = std::env::temp_dir().join(format!("opsbatch-drive-outside-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "secret").unwrap();
    let mut share = Share::new();
    std::os::unix::fs::symlink(&outside, share.dir.join("link")).unwrap();
    std::os::unix::fs::symlink(outside.join("not-there"), share.dir.join("dangling")).unwrap();
    // reading through the link, and creating through a link whose target does not exist yet, are both refused
    assert_eq!(share.create("\\link\\secret.txt", CreateDisposition::FILE_OPEN, CreateOptions::empty()).0, NtStatus::ACCESS_DENIED);
    assert_eq!(share.create("\\link", CreateDisposition::FILE_OPEN, CreateOptions::empty()).0, NtStatus::ACCESS_DENIED);
    assert_eq!(share.create("\\dangling", CreateDisposition::FILE_CREATE, CreateOptions::empty()).0, NtStatus::ACCESS_DENIED);
    assert!(!outside.join("not-there").exists());
    // a link that stays inside is fine
    share.put("real/file.txt", "x");
    std::os::unix::fs::symlink(share.dir.join("real"), share.dir.join("inner")).unwrap();
    assert_eq!(share.create("\\inner\\file.txt", CreateDisposition::FILE_OPEN, CreateOptions::empty()).0, NtStatus::SUCCESS);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn a_file_is_created_written_read_and_closed() {
    let mut share = Share::new();
    let id = share.open_file("\\hello.txt", CreateDisposition::FILE_CREATE);
    assert_eq!(share.write(id, 0, b"hello"), (NtStatus::SUCCESS, 5));
    assert_eq!(share.write(id, 5, b" world"), (NtStatus::SUCCESS, 6));
    assert_eq!(share.read(id, 0, 100), (NtStatus::SUCCESS, b"hello world".to_vec()));
    assert_eq!(share.read(id, 6, 5), (NtStatus::SUCCESS, b"world".to_vec()));
    assert_eq!(share.read(id, 50, 10), (NtStatus::SUCCESS, Vec::new()), "reading past the end is an empty read");
    // an overwrite in the middle leaves the rest alone
    share.write(id, 0, b"J");
    assert_eq!(share.close(id), NtStatus::SUCCESS);
    assert_eq!(fs::read_to_string(share.dir.join("hello.txt")).unwrap(), "Jello world");
    assert_eq!(share.close(id), NtStatus::from(STATUS_INVALID_HANDLE), "a closed handle is gone");
    assert_eq!(share.read(id, 0, 1).0, NtStatus::from(STATUS_INVALID_HANDLE));
}

#[test]
fn a_big_read_is_cut_to_the_limit() {
    let mut share = Share::new();
    share.put("big.bin", &"x".repeat(MAX_READ as usize + 10));
    let id = share.open_file("\\big.bin", CreateDisposition::FILE_OPEN);
    let (status, data) = share.read(id, 0, u32::MAX);
    assert_eq!((status, data.len()), (NtStatus::SUCCESS, MAX_READ as usize));
}

#[test]
fn create_dispositions_follow_the_windows_rules() {
    let mut share = Share::new();
    share.put("old.txt", "old content");
    assert_eq!(share.create("\\nope.txt", CreateDisposition::FILE_OPEN, CreateOptions::empty()).0, NtStatus::NO_SUCH_FILE);
    assert_eq!(share.create("\\nope.txt", CreateDisposition::FILE_OVERWRITE, CreateOptions::empty()).0, NtStatus::NO_SUCH_FILE);
    assert_eq!(share.create("\\old.txt", CreateDisposition::FILE_CREATE, CreateOptions::empty()).0, NtStatus::OBJECT_NAME_COLLISION);
    assert_eq!(share.create("\\missing\\new.txt", CreateDisposition::FILE_CREATE, CreateOptions::empty()).0, NtStatus::from(STATUS_OBJECT_PATH_NOT_FOUND));
    // open-if opens what is there and creates what is not
    let id = share.open_file("\\old.txt", CreateDisposition::FILE_OPEN_IF);
    assert_eq!(share.read(id, 0, 100).1, b"old content");
    share.close(id);
    let id = share.open_file("\\fresh.txt", CreateDisposition::FILE_OPEN_IF);
    share.close(id);
    assert!(share.dir.join("fresh.txt").exists());
    // overwrite-if empties an existing file
    let id = share.open_file("\\old.txt", CreateDisposition::FILE_OVERWRITE_IF);
    share.close(id);
    assert_eq!(fs::read_to_string(share.dir.join("old.txt")).unwrap(), "");
    // a directory is not a file, and a file is not a directory
    share.put("dir/inner.txt", "x");
    assert_eq!(share.create("\\dir", CreateDisposition::FILE_OPEN, CreateOptions::FILE_NON_DIRECTORY_FILE).0, NtStatus::from(STATUS_FILE_IS_A_DIRECTORY));
    assert_eq!(share.create("\\old.txt", CreateDisposition::FILE_OPEN, CreateOptions::FILE_DIRECTORY_FILE).0, NtStatus::NOT_A_DIRECTORY);
    // a new directory
    let (status, id) = share.create("\\made", CreateDisposition::FILE_CREATE, CreateOptions::FILE_DIRECTORY_FILE);
    assert_eq!(status, NtStatus::SUCCESS);
    share.close(id);
    assert!(share.dir.join("made").is_dir());
    assert_eq!(share.create("\\made", CreateDisposition::FILE_CREATE, CreateOptions::FILE_DIRECTORY_FILE).0, NtStatus::OBJECT_NAME_COLLISION);
}

#[test]
fn names_are_found_without_regard_to_case() {
    let mut share = Share::new();
    share.put("Docs/Report.TXT", "r");
    let id = share.open_file("\\docs\\report.txt", CreateDisposition::FILE_OPEN);
    assert_eq!(share.read(id, 0, 10).1, b"r");
}

#[test]
fn file_information_reports_size_attributes_and_windows_times() {
    use std::time::SystemTime;
    let mut share = Share::new();
    share.put("data.bin", "12345");
    share.put(".hidden", "h");
    share.put("sub/x", "x");
    let one_day = UNIX_EPOCH + Duration::from_secs(86_400);
    File::options().write(true).open(share.dir.join("data.bin")).unwrap().set_modified(one_day).unwrap();

    let id = share.open_file("\\data.bin", CreateDisposition::FILE_OPEN);
    let Some(FileInformationClass::Basic(b)) = share.query(id, FileInformationClassLevel::FILE_BASIC_INFORMATION).1 else { panic!("basic information") };
    assert_eq!(b.last_write_time, ((WINDOWS_TO_UNIX_SECS + 86_400) * 10_000_000) as i64, "FILETIME is 100 ns ticks since 1601");
    assert_eq!(b.file_attributes, FileAttributes::FILE_ATTRIBUTE_NORMAL);
    let Some(FileInformationClass::Standard(s)) = share.query(id, FileInformationClassLevel::FILE_STANDARD_INFORMATION).1 else { panic!("standard information") };
    assert_eq!((s.end_of_file, s.allocation_size, s.number_of_links, s.directory, s.delete_pending), (5, 5, 1, Boolean::False, Boolean::False));
    let Some(FileInformationClass::AttributeTag(t)) = share.query(id, FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION).1 else { panic!("attribute tag") };
    assert_eq!((t.file_attributes, t.reparse_tag), (FileAttributes::FILE_ATTRIBUTE_NORMAL, 0));
    assert_eq!(share.query(id, FileInformationClassLevel::FILE_NAMES_INFORMATION).0, NtStatus::NOT_SUPPORTED);

    let hidden = share.open_file("\\.hidden", CreateDisposition::FILE_OPEN);
    let Some(FileInformationClass::Basic(b)) = share.query(hidden, FileInformationClassLevel::FILE_BASIC_INFORMATION).1 else { panic!() };
    assert_eq!(b.file_attributes, FileAttributes::FILE_ATTRIBUTE_HIDDEN, "dot files are hidden");
    let dir = open_dir(&mut share, "\\sub");
    let Some(FileInformationClass::Standard(s)) = share.query(dir, FileInformationClassLevel::FILE_STANDARD_INFORMATION).1 else { panic!() };
    assert_eq!((s.directory, s.end_of_file), (Boolean::True, 0));
    let Some(FileInformationClass::Basic(b)) = share.query(dir, FileInformationClassLevel::FILE_BASIC_INFORMATION).1 else { panic!() };
    assert_eq!(b.file_attributes, FileAttributes::FILE_ATTRIBUTE_DIRECTORY);
    let _ = SystemTime::now();
}

#[test]
fn a_directory_is_listed_one_entry_per_request() {
    let mut share = Share::new();
    share.put("b.txt", "bb");
    share.put("A.txt", "a");
    share.put("sub/c.log", "c");
    // the root has no "." and ".."; entries come sorted by name
    assert_eq!(share.names("\\", "*"), vec!["A.txt", "b.txt", "sub"]);
    assert_eq!(share.names("\\", "*.txt"), vec!["A.txt", "b.txt"]);
    assert_eq!(share.names("\\", "B.TXT"), vec!["b.txt"], "case does not matter");
    assert_eq!(share.names("\\", "*.*"), vec!["A.txt", "b.txt", "sub"]);
    assert!(share.names("\\", "*.zip").is_empty(), "no match: nothing is listed");
    assert_eq!(share.names("\\sub", "*"), vec![".", "..", "c.log"], "a sub folder starts with . and ..");

    // the class asked for decides what an entry carries
    let id = open_dir(&mut share, "\\");
    let (status, buffer) = share.list(id, "*", true, FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION);
    let Some(FileInformationClass::BothDirectory(e)) = buffer else { panic!("both directory information") };
    assert_eq!((status, e.file_name.as_str(), e.end_of_file), (NtStatus::SUCCESS, "A.txt", 1));
    let (_, buffer) = share.list(id, "*", false, FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION);
    assert!(matches!(buffer, Some(FileInformationClass::FullDirectory(e)) if e.file_name == "b.txt" && e.end_of_file == 2));
    let (_, buffer) = share.list(id, "*", false, FileInformationClassLevel::FILE_DIRECTORY_INFORMATION);
    let Some(FileInformationClass::Directory(e)) = buffer else { panic!("directory information") };
    assert_eq!((e.file_name.as_str(), e.file_attributes), ("sub", FileAttributes::FILE_ATTRIBUTE_DIRECTORY));
    assert_eq!(share.list(id, "*", false, FileInformationClassLevel::FILE_NAMES_INFORMATION).0, NtStatus::NO_MORE_FILES);
    // starting again lists from the top
    assert!(matches!(share.list(id, "*", true, FileInformationClassLevel::FILE_NAMES_INFORMATION).1, Some(FileInformationClass::Names(n)) if n.file_name == "A.txt"));
    // a file cannot be listed
    let file = share.open_file("\\A.txt", CreateDisposition::FILE_OPEN);
    assert_eq!(share.list(file, "*", true, FileInformationClassLevel::FILE_NAMES_INFORMATION).0, NtStatus::NOT_A_DIRECTORY);
}

#[test]
fn delete_pending_removes_on_close_and_a_full_directory_refuses() {
    let mut share = Share::new();
    share.put("gone.txt", "x");
    share.put("full/inner.txt", "x");
    let id = share.open_file("\\gone.txt", CreateDisposition::FILE_OPEN);
    assert_eq!(share.set(id, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 })), NtStatus::SUCCESS);
    let Some(FileInformationClass::Standard(s)) = share.query(id, FileInformationClassLevel::FILE_STANDARD_INFORMATION).1 else { panic!() };
    assert_eq!(s.delete_pending, Boolean::True);
    assert!(share.dir.join("gone.txt").exists(), "still there until the handle is closed");
    assert_eq!(share.close(id), NtStatus::SUCCESS);
    assert!(!share.dir.join("gone.txt").exists());

    let full = open_dir(&mut share, "\\full");
    assert_eq!(share.set(full, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 })), NtStatus::DIRECTORY_NOT_EMPTY);
    share.close(full);
    assert!(share.dir.join("full").is_dir());

    // taking the delete back, and an empty directory that is deleted
    share.put("keep.txt", "k");
    let id = share.open_file("\\keep.txt", CreateDisposition::FILE_OPEN);
    share.set(id, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }));
    share.set(id, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 0 }));
    share.close(id);
    assert!(share.dir.join("keep.txt").exists());
    fs::create_dir(share.dir.join("empty")).unwrap();
    let empty = open_dir(&mut share, "\\empty");
    share.set(empty, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }));
    share.close(empty);
    assert!(!share.dir.join("empty").exists());

    // the shared folder itself cannot be deleted
    let root = open_dir(&mut share, "\\");
    assert_eq!(share.set(root, FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 })), NtStatus::ACCESS_DENIED);
    share.close(root);
    assert!(share.dir.is_dir());
}

#[test]
fn delete_on_close_from_the_create_request() {
    let mut share = Share::new();
    let (status, id) = share.create("\\temp.tmp", CreateDisposition::FILE_CREATE, CreateOptions::FILE_DELETE_ON_CLOSE);
    assert_eq!(status, NtStatus::SUCCESS);
    assert!(share.dir.join("temp.tmp").exists());
    share.close(id);
    assert!(!share.dir.join("temp.tmp").exists());
}

#[test]
fn rename_moves_inside_the_folder_and_respects_replace() {
    let mut share = Share::new();
    share.put("a.txt", "A");
    share.put("b.txt", "B");
    share.put("sub/keep", "k");
    let a = share.open_file("\\a.txt", CreateDisposition::FILE_OPEN);
    assert_eq!(share.rename(a, "\\b.txt", false), NtStatus::OBJECT_NAME_COLLISION);
    assert_eq!(share.rename(a, "\\sub", true), NtStatus::OBJECT_NAME_COLLISION, "a directory is never replaced");
    assert_eq!(share.rename(a, "\\..\\escaped.txt", true), NtStatus::ACCESS_DENIED);
    assert!(!share.dir.parent().unwrap().join("escaped.txt").exists());
    assert_eq!(share.rename(a, "\\sub\\moved.txt", false), NtStatus::SUCCESS);
    assert!(!share.dir.join("a.txt").exists());
    assert_eq!(fs::read_to_string(share.dir.join("sub/moved.txt")).unwrap(), "A");
    // the handle follows the file
    assert_eq!(share.read(a, 0, 10).1, b"A");
    assert_eq!(share.rename(a, "\\b.txt", true), NtStatus::SUCCESS);
    assert_eq!(fs::read_to_string(share.dir.join("b.txt")).unwrap(), "A");
    let root = open_dir(&mut share, "\\");
    assert_eq!(share.rename(root, "\\other", false), NtStatus::ACCESS_DENIED);
}

#[test]
fn end_of_file_and_times_and_attributes_can_be_set() {
    let mut share = Share::new();
    share.put("f.txt", "0123456789");
    let id = share.open_file("\\f.txt", CreateDisposition::FILE_OPEN);
    assert_eq!(share.set(id, FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 4 })), NtStatus::SUCCESS);
    assert_eq!(fs::read_to_string(share.dir.join("f.txt")).unwrap(), "0123");
    assert_eq!(share.set(id, FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 8 })), NtStatus::SUCCESS);
    assert_eq!(fs::metadata(share.dir.join("f.txt")).unwrap().len(), 8);
    let stamp = ((WINDOWS_TO_UNIX_SECS + 1_000_000) * 10_000_000) as i64;
    let basic = |attributes: FileAttributes, written: i64| FileInformationClass::Basic(FileBasicInformation { creation_time: 0, last_access_time: 0, last_write_time: written, change_time: 0, file_attributes: attributes });
    assert_eq!(share.set(id, basic(FileAttributes::FILE_ATTRIBUTE_READONLY, stamp)), NtStatus::SUCCESS);
    assert_eq!(fs::metadata(share.dir.join("f.txt")).unwrap().modified().unwrap(), UNIX_EPOCH + Duration::from_secs(1_000_000));
    assert!(fs::metadata(share.dir.join("f.txt")).unwrap().permissions().readonly());
    let Some(FileInformationClass::Basic(b)) = share.query(id, FileInformationClassLevel::FILE_BASIC_INFORMATION).1 else { panic!() };
    assert_eq!(b.file_attributes, FileAttributes::FILE_ATTRIBUTE_READONLY);
    // "leave the times as they are" (0 and -1) changes nothing, and NORMAL lets go of read-only
    assert_eq!(share.set(id, basic(FileAttributes::FILE_ATTRIBUTE_NORMAL, -1)), NtStatus::SUCCESS);
    assert_eq!(fs::metadata(share.dir.join("f.txt")).unwrap().modified().unwrap(), UNIX_EPOCH + Duration::from_secs(1_000_000));
    assert!(!fs::metadata(share.dir.join("f.txt")).unwrap().permissions().readonly());
}

#[test]
fn volume_information_describes_the_folder_as_a_drive() {
    let mut share = Share::new();
    let label = share.dir.file_name().unwrap().to_string_lossy().into_owned();
    let FileSystemInformationClass::FileFsVolumeInformation(v) = share.volume(FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION) else { panic!() };
    assert_eq!(v.volume_label, label);
    assert_eq!(share.backend.label(), label);
    let FileSystemInformationClass::FileFsSizeInformation(s) = share.volume(FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION) else { panic!() };
    assert!(s.total_alloc_units > 0 && s.available_alloc_units <= s.total_alloc_units);
    assert_eq!((s.sectors_per_alloc_unit, s.bytes_per_sector), (SECTORS_PER_UNIT, BYTES_PER_SECTOR));
    let FileSystemInformationClass::FileFsFullSizeInformation(f) = share.volume(FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION) else { panic!() };
    assert_eq!(f.total_alloc_units, s.total_alloc_units);
    let FileSystemInformationClass::FileFsAttributeInformation(a) = share.volume(FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION) else { panic!() };
    assert_eq!((a.file_system_name.as_str(), a.max_component_name_len), ("NTFS", 255));
    let FileSystemInformationClass::FileFsDeviceInformation(d) = share.volume(FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION) else { panic!() };
    assert_eq!(d.device_type, 7);
}

#[test]
fn a_folder_that_cannot_be_shared_says_why() {
    let missing = std::env::temp_dir().join(format!("opsbatch-drive-missing-{}", uuid::Uuid::new_v4()));
    let error = DriveBackend::new(&missing).unwrap_err();
    assert!(error.contains("共享目录不可用"), "{error}");
    let file = std::env::temp_dir().join(format!("opsbatch-drive-file-{}", uuid::Uuid::new_v4()));
    fs::write(&file, "x").unwrap();
    assert!(DriveBackend::new(&file).unwrap_err().contains("不是目录"));
    let _ = fs::remove_file(&file);
}

#[test]
fn wildcards_follow_dos_rules() {
    assert!(wildcard_matches("*", "anything.txt"));
    assert!(wildcard_matches("*.*", "no-dot"));
    assert!(wildcard_matches("*.txt", "A.TXT"));
    assert!(!wildcard_matches("*.txt", "a.txt.bak"));
    assert!(wildcard_matches("a?c", "abc"));
    assert!(!wildcard_matches("a?c", "ac"));
    assert!(wildcard_matches("a*b*c", "aXXbYYc"));
    assert!(!wildcard_matches("a*b*c", "aXXbYY"));
    assert!(wildcard_matches("exact.txt", "Exact.TXT"));
    assert!(wildcard_matches("*", ""));
    assert!(wildcard_matches("<.txt", "x.txt"), "the DOS star");
    assert!(wildcard_matches("a>c", "abc"), "the DOS question mark");
    assert!(wildcard_matches("日*", "日本語.txt"));
}

#[test]
fn filetime_round_trips_and_odd_values_are_ignored() {
    let stamp = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
    let filetime = system_to_filetime(Ok(stamp));
    assert_eq!(filetime, ((1_700_000_000 + WINDOWS_TO_UNIX_SECS) * 10_000_000 + 1_234_567) as i64);
    assert_eq!(filetime_to_system(filetime), Some(stamp));
    assert_eq!(filetime_to_system(0), None);
    assert_eq!(filetime_to_system(-1), None);
    assert_eq!(filetime_to_system(1_000), None, "before 1970");
}

// ---- through the real channel processor: bytes in, bytes out ----

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// An RDPDR device I/O request as the server sends it: shared header, request header, then the major-function body.
fn request(major: u32, minor: u32, file_id: u32, completion: u32, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0x72, 0x44, 0x52, 0x49]; // RDPDR_CTYP_CORE, PAKID_CORE_DEVICE_IOREQUEST
    for v in [DRIVE_DEVICE_ID, file_id, completion, major, minor] {
        le32(&mut out, v);
    }
    out.extend_from_slice(body);
    out
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

fn processor(dir: &Path) -> Rdpdr {
    let backend = DriveBackend::new(dir).unwrap();
    let label = backend.label().to_string();
    Rdpdr::new(Box::new(backend), "OpsBatch".to_string()).with_drives(Some(vec![(DRIVE_DEVICE_ID, label)]))
}

fn answer(rdpdr: &mut Rdpdr, request: &[u8]) -> Vec<u8> {
    let mut replies = rdpdr.process(request).unwrap();
    assert_eq!(replies.len(), 1, "one request, one completion");
    replies.remove(0).encode_unframed_pdu().unwrap()
}

#[test]
fn the_channel_decodes_requests_and_encodes_completions() {
    let share = Share::new();
    share.put("hello.txt", "hello over rdpdr");
    let mut rdpdr = processor(&share.dir);

    // IRP_MJ_CREATE for \hello.txt, FILE_OPEN
    let mut body = Vec::new();
    for v in [0x0012_019f_u32] {
        le32(&mut body, v); // DesiredAccess
    }
    body.extend_from_slice(&0u64.to_le_bytes()); // AllocationSize
    for v in [0u32, 0, 1, 0x40] {
        le32(&mut body, v); // FileAttributes, SharedAccess, CreateDisposition (FILE_OPEN), CreateOptions (non-directory)
    }
    let path = utf16z("\\hello.txt");
    le32(&mut body, path.len() as u32);
    body.extend_from_slice(&path);
    let reply = answer(&mut rdpdr, &request(0, 0, 0, 7, &body));
    // shared header (RDPDR_CTYP_CORE, PAKID_CORE_DEVICE_IOCOMPLETION), DeviceId, CompletionId, IoStatus, FileId, Information
    assert_eq!(&reply[..4], &[0x72, 0x44, 0x43, 0x49]);
    assert_eq!(u32::from_le_bytes(reply[4..8].try_into().unwrap()), DRIVE_DEVICE_ID);
    assert_eq!(u32::from_le_bytes(reply[8..12].try_into().unwrap()), 7, "the completion id comes back");
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0, "STATUS_SUCCESS");
    let file_id = u32::from_le_bytes(reply[16..20].try_into().unwrap());
    assert_eq!(reply[20], 1, "FILE_OPENED");

    // IRP_MJ_READ: length, offset, 20 bytes of padding
    let mut body = Vec::new();
    le32(&mut body, 5);
    body.extend_from_slice(&6u64.to_le_bytes());
    body.extend_from_slice(&[0; 20]);
    let reply = answer(&mut rdpdr, &request(3, 0, file_id, 8, &body));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(reply[16..20].try_into().unwrap()), 5, "length of the data");
    assert_eq!(&reply[20..25], b"over ");

    // IRP_MJ_CLOSE (32 bytes of padding)
    let reply = answer(&mut rdpdr, &request(2, 0, file_id, 9, &[0; 32]));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0);

    // a missing file is STATUS_NO_SUCH_FILE
    let mut body = Vec::new();
    le32(&mut body, 0x0012_019f);
    body.extend_from_slice(&0u64.to_le_bytes());
    for v in [0u32, 0, 1, 0x40] {
        le32(&mut body, v);
    }
    let path = utf16z("\\missing.txt");
    le32(&mut body, path.len() as u32);
    body.extend_from_slice(&path);
    let reply = answer(&mut rdpdr, &request(0, 0, 0, 10, &body));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0xC000_000F);
}

#[test]
fn a_directory_listing_goes_through_the_channel() {
    let share = Share::new();
    share.put("one.txt", "1");
    let mut rdpdr = processor(&share.dir);

    let mut body = Vec::new();
    le32(&mut body, 0x0010_0001);
    body.extend_from_slice(&0u64.to_le_bytes());
    for v in [0x10u32, 3, 1, 0x21] {
        le32(&mut body, v); // directory attributes, share read+write, FILE_OPEN, FILE_DIRECTORY_FILE | synchronous
    }
    let path = utf16z("\\");
    le32(&mut body, path.len() as u32);
    body.extend_from_slice(&path);
    let reply = answer(&mut rdpdr, &request(0, 0, 0, 1, &body));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0);
    let dir_id = u32::from_le_bytes(reply[16..20].try_into().unwrap());

    // IRP_MJ_DIRECTORY_CONTROL / IRP_MN_QUERY_DIRECTORY with FileNamesInformation (12): class, initial query, path
    let mut body = Vec::new();
    le32(&mut body, 12);
    body.push(1);
    let pattern = utf16z("\\*");
    le32(&mut body, pattern.len() as u32);
    body.extend_from_slice(&[0; 23]);
    body.extend_from_slice(&pattern);
    let reply = answer(&mut rdpdr, &request(0x0c, 1, dir_id, 2, &body));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0, "STATUS_SUCCESS");
    let length = u32::from_le_bytes(reply[16..20].try_into().unwrap()) as usize;
    let info = &reply[20..20 + length];
    // FileNamesInformation: NextEntryOffset, FileIndex, FileNameLength, FileName (UTF-16)
    let name_len = u32::from_le_bytes(info[8..12].try_into().unwrap()) as usize;
    let name: Vec<u16> = info[12..12 + name_len].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
    assert_eq!(String::from_utf16(&name).unwrap(), "one.txt");

    // the next request has nothing left: STATUS_NO_MORE_FILES
    let mut body = Vec::new();
    le32(&mut body, 12);
    body.push(0);
    le32(&mut body, pattern.len() as u32);
    body.extend_from_slice(&[0; 23]);
    body.extend_from_slice(&pattern);
    let reply = answer(&mut rdpdr, &request(0x0c, 1, dir_id, 3, &body));
    assert_eq!(u32::from_le_bytes(reply[12..16].try_into().unwrap()), 0x8000_0006);
}
