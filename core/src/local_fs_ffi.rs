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
fn input_allow_empty<'a>(pointer: *const c_char) -> Option<&'a str> {
    (!pointer.is_null())
        .then(|| unsafe { CStr::from_ptr(pointer) }.to_str().ok())
        .flatten()
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
    // An empty relative path is the pinned root listing, not an invalid
    // request. The root itself must still be explicit and non-empty.
    let (Some(root), Some(relative)) = (input(root), input_allow_empty(relative)) else {
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
    if input(root).is_none() || input(relative).is_none() {
        return response(json!({"ok":false,"error":"invalid_request"}));
    }
    // Keep the legacy symbol fail-closed for ABI compatibility. A direct
    // filesystem mutation would bypass the sole durable JobStore path.
    response(json!({"ok":false,"error":"local_mkdir_requires_job"}))
}

#[no_mangle]
/// # Safety
/// Pointer must be null or a value previously returned by this ABI exactly once.
pub unsafe extern "C" fn qadren_local_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn root_listing_accepts_an_empty_relative_path() {
        let root_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-local-ffi-root-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir(&root_path).unwrap();
        let root = CString::new(root_path.to_str().unwrap()).unwrap();
        let relative = CString::new("").unwrap();
        let result = unsafe { qadren_local_list(root.as_ptr(), relative.as_ptr()) };
        assert!(unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .contains("\"ok\":true"));
        unsafe { qadren_local_free_string(result) };
        std::fs::remove_dir(&root_path).unwrap();
    }

    #[test]
    fn legacy_mkdir_ffi_cannot_bypass_the_job_store() {
        let root_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-local-ffi-mkdir-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir(&root_path).unwrap();
        let root = CString::new(root_path.to_str().unwrap()).unwrap();
        let relative = CString::new("must-not-exist").unwrap();
        let result = unsafe { qadren_local_mkdir(root.as_ptr(), relative.as_ptr()) };
        let message = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(message.contains("\"error\":\"local_mkdir_requires_job\""));
        assert!(!root_path.join("must-not-exist").exists());
        unsafe { qadren_local_free_string(result) };
        std::fs::remove_dir(&root_path).unwrap();
    }
}
