//! 平台原生文件剪贴板读取

use std::path::PathBuf;

/// 读取系统剪贴板中的文件路径列表
pub(super) fn read_clipboard_file_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        read_macos_clipboard_files()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// macOS: 通过 objc runtime 调用 NSPasteboard 读取文件 URL
#[cfg(target_os = "macos")]
fn read_macos_clipboard_files() -> Vec<PathBuf> {
    use objc::runtime::{BOOL, NO};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ptr::null_mut;

    type ObjId = *mut objc::runtime::Object;

    unsafe {
        let pb: ObjId = msg_send![class!(NSPasteboard), generalPasteboard];
        if pb.is_null() {
            return Vec::new();
        }

        let url_class: *mut objc::runtime::Class = msg_send![class!(NSURL), class];
        let classes: ObjId = msg_send![class!(NSArray), arrayWithObject: url_class];

        let nil_dict: ObjId = null_mut();
        let objects: ObjId = msg_send![pb, readObjectsForClasses: classes options: nil_dict];
        if objects.is_null() {
            return Vec::new();
        }

        let count: usize = msg_send![objects, count];
        if count == 0 {
            return Vec::new();
        }

        let mut paths = Vec::with_capacity(count);
        for i in 0..count {
            let url: ObjId = msg_send![objects, objectAtIndex: i];
            if url.is_null() {
                continue;
            }
            let is_file: BOOL = msg_send![url, isFileURL];
            if is_file == NO {
                continue;
            }
            let path_str: ObjId = msg_send![url, path];
            if path_str.is_null() {
                continue;
            }
            let c_str: *const i8 = msg_send![path_str, UTF8String];
            if c_str.is_null() {
                continue;
            }
            let path = std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned();
            paths.push(PathBuf::from(path));
        }

        paths
    }
}
