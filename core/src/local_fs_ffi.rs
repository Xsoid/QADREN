//! Narrow metadata-only Local Filesystem ABI for the macOS file-manager shell.
use crate::local_fs::LocalFilesystem;
use serde_json::json;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

fn input<'a>(pointer: *const c_char) -> Option<&'a str> {
    (!pointer.is_null())
        .then(|| unsafe { CStr::from_ptr(pointer) }.to_str().ok())
        .flatten()
        .filter(|s| !s.is_empty())
}
fn response(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
}

#[no_mangle]
/// # Safety
/// Pointers must reference valid NUL-terminated UTF-8 strings during this call.
pub unsafe extern "C" fn qadren_local_list(
    root: *const c_char,
    relative: *const c_char,
) -> *mut c_char {
    let (Some(root), Some(relative)) = (input(root), input(relative)) else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    match LocalFilesystem::open(Path::new(root)).and_then(|fs| fs.list(Path::new(relative))) {
        Ok(entries) => response(
            json!({"ok":true,"data":entries.into_iter().map(|e| json!({"name":e.name,"is_directory":e.is_directory,"size":e.size})).collect::<Vec<_>>() }),
        ),
        Err(_) => response(json!({"ok":false,"error":"local_metadata_unavailable"})),
    }
}

#[no_mangle]
/// # Safety
/// Pointers must reference valid NUL-terminated UTF-8 strings during this call.
pub unsafe extern "C" fn qadren_local_mkdir(
    root: *const c_char,
    relative: *const c_char,
) -> *mut c_char {
    let (Some(root), Some(relative)) = (input(root), input(relative)) else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    match LocalFilesystem::open(Path::new(root))
        .and_then(|fs| fs.create_directory(Path::new(relative)))
    {
        Ok(()) => response(json!({"ok":true,"data":true})),
        Err(_) => response(json!({"ok":false,"error":"local_mkdir_unavailable"})),
    }
}

#[no_mangle]
/// # Safety
/// Pointer must be null or a value previously returned by this ABI exactly once.
pub unsafe extern "C" fn qadren_local_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}
