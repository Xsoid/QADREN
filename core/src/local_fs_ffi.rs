//! Narrow metadata-only Local Filesystem ABI for macOS scenario folder selection.
use crate::local_fs::LocalFilesystem;
use crate::registry::Registry;
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

fn metadata_snapshot(root: &str) -> crate::local_fs::Result<serde_json::Value> {
    const MAX_ENTRIES: usize = 10_000;
    LocalFilesystem::open(Path::new(root)).and_then(|fs| {
        fs.list_tree(Path::new(""), MAX_ENTRIES).map(|entries| {
            json!(entries
                .into_iter()
                .filter_map(|(relative_path, entry)| {
                    (!entry.is_directory).then(|| {
                        json!({
                            "relative_path": relative_path.to_string_lossy(),
                            "object_revision": entry.object_revision,
                            "size": entry.size,
                            "checksum_md5": null
                        })
                    })
                })
                .collect::<Vec<_>>())
        })
    })
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

/// Return a bounded recursive snapshot of file metadata below one explicit
/// Local Filesystem root. This opens no file contents and produces no hashes.
/// The UI must obtain its own per-request authorization before calling this
/// function; this ABI does not submit a Job or change FileSyncState.
///
/// # Safety
/// Pointer must reference a valid NUL-terminated UTF-8 string during this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_local_metadata_snapshot(root: *const c_char) -> *mut c_char {
    let Some(root) = input(root) else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    match metadata_snapshot(root) {
        Ok(entries) => response(json!({"ok":true,"data":entries})),
        Err(_) => response(json!({"ok":false,"error":"local_metadata_unavailable"})),
    }
}

/// Return a metadata-only snapshot for one enabled Local StorageConnection.
/// The Registry check happens before opening the root, so a disabled, missing,
/// or non-Local Scenario endpoint cannot cause a filesystem scan. The caller
/// still owns the exact Scenario revision and no content is opened or hashed.
///
/// # Safety
/// Pointers must reference valid NUL-terminated UTF-8 strings during this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_local_scenario_metadata_snapshot(
    registry_path: *const c_char,
    connection_id: *const c_char,
    root: *const c_char,
) -> *mut c_char {
    let (Some(registry_path), Some(connection_id), Some(root)) =
        (input(registry_path), input(connection_id), input(root))
    else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    let connection = match Registry::open(Path::new(registry_path))
        .and_then(|registry| registry.get(connection_id))
    {
        Ok(Some(connection)) => connection,
        Ok(None) => {
            return response(json!({"ok":false,"error":"local_scenario_connection_unavailable"}))
        }
        Err(_) => return response(json!({"ok":false,"error":"registry_unavailable"})),
    };
    if connection.adapter != "local_filesystem" || !connection.enabled {
        return response(json!({"ok":false,"error":"local_scenario_connection_unavailable"}));
    }
    match metadata_snapshot(root) {
        Ok(entries) => response(json!({"ok":true,"data":entries})),
        Err(_) => response(json!({"ok":false,"error":"local_metadata_unavailable"})),
    }
}

/// Return a metadata-only, no-overwrite candidate for one manual Local copy.
/// The result is a planning aid only: it does not create a Job, read content,
/// reserve storage, or authorize an effect. Roots that are identical or nested
/// are rejected so a later executor cannot mistake this preview for a safe
/// cross-root transfer.
///
/// # Safety
/// Pointers must reference valid NUL-terminated UTF-8 strings during this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_local_copy_preview(
    source_root: *const c_char,
    source_relative: *const c_char,
    destination_root: *const c_char,
    destination_relative: *const c_char,
) -> *mut c_char {
    let (
        Some(source_root),
        Some(source_relative),
        Some(destination_root),
        Some(destination_relative),
    ) = (
        input(source_root),
        input(source_relative),
        input(destination_root),
        input_allow_empty(destination_relative),
    )
    else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    let result = (|| {
        let source = LocalFilesystem::open(Path::new(source_root))?;
        let destination = LocalFilesystem::open(Path::new(destination_root))?;
        if source.overlaps_root(&destination)? {
            return Err(crate::local_fs::LocalError::UnsafeEntry);
        }
        let source_path = Path::new(source_relative);
        let source_entry = source.stat(source_path)?;
        if source_entry.is_directory {
            return Err(crate::local_fs::LocalError::UnsafeEntry);
        }
        // Listing the destination directory proves only metadata access. It
        // does not reserve the name or authorize a future write.
        destination.list(Path::new(destination_relative))?;
        let name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(crate::local_fs::LocalError::InvalidPath)?;
        let destination_path = if destination_relative.is_empty() {
            name.to_owned()
        } else {
            format!("{destination_relative}/{name}")
        };
        let destination_exists = match destination.stat(Path::new(&destination_path)) {
            Ok(_) => true,
            Err(crate::local_fs::LocalError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                false
            }
            Err(error) => return Err(error),
        };
        Ok(json!({
            "source_relative": source_relative,
            "destination_relative": destination_path,
            "size": source_entry.size,
            "destination_exists": destination_exists
        }))
    })();
    match result {
        Ok(data) => response(json!({"ok":true,"data":data})),
        Err(_) => response(json!({"ok":false,"error":"local_copy_preview_unavailable"})),
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

    #[test]
    fn metadata_snapshot_is_recursive_file_metadata_only() {
        // On macOS, /var can be a symlink to /private/var. Open the physical
        // root so this test exercises the same confinement contract as the
        // adapter rather than an alias that must fail closed.
        let root_path = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "qadren-local-ffi-snapshot-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(root_path.join("nested")).unwrap();
        std::fs::write(root_path.join("nested/report.txt"), b"payload").unwrap();
        let root = CString::new(root_path.to_str().unwrap()).unwrap();
        let result = unsafe { qadren_local_metadata_snapshot(root.as_ptr()) };
        let value: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"].as_array().unwrap().len(), 1);
        assert_eq!(value["data"][0]["relative_path"], "nested/report.txt");
        assert_eq!(value["data"][0]["size"], 7);
        let first_revision = value["data"][0]["object_revision"]
            .as_str()
            .expect("Local metadata snapshot must identify a local file")
            .to_owned();
        assert!(value["data"][0]["checksum_md5"].is_null());
        unsafe { qadren_local_free_string(result) };
        std::fs::write(root_path.join("nested/report.txt"), b"changed").unwrap();
        let result = unsafe { qadren_local_metadata_snapshot(root.as_ptr()) };
        let value: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_ne!(value["data"][0]["object_revision"], first_revision);
        unsafe { qadren_local_free_string(result) };
        std::fs::remove_dir_all(root_path).unwrap();
    }

    #[test]
    fn copy_preview_uses_metadata_and_never_creates_a_destination() {
        let base = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "qadren-local-ffi-copy-preview-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let source_path = base.join("source");
        let destination_path = base.join("destination");
        std::fs::create_dir_all(&source_path).unwrap();
        std::fs::create_dir(&destination_path).unwrap();
        std::fs::write(source_path.join("report.txt"), b"payload").unwrap();
        let source_root = CString::new(source_path.to_str().unwrap()).unwrap();
        let source_relative = CString::new("report.txt").unwrap();
        let destination_root = CString::new(destination_path.to_str().unwrap()).unwrap();
        let destination_relative = CString::new("").unwrap();
        let result = unsafe {
            qadren_local_copy_preview(
                source_root.as_ptr(),
                source_relative.as_ptr(),
                destination_root.as_ptr(),
                destination_relative.as_ptr(),
            )
        };
        let value: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["source_relative"], "report.txt");
        assert_eq!(value["data"]["destination_relative"], "report.txt");
        assert_eq!(value["data"]["size"], 7);
        assert_eq!(value["data"]["destination_exists"], false);
        assert!(!destination_path.join("report.txt").exists());
        unsafe { qadren_local_free_string(result) };
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("outside.txt", destination_path.join("report.txt")).unwrap();
            let result = unsafe {
                qadren_local_copy_preview(
                    source_root.as_ptr(),
                    source_relative.as_ptr(),
                    destination_root.as_ptr(),
                    destination_relative.as_ptr(),
                )
            };
            let value: serde_json::Value =
                serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
            assert_eq!(value["ok"], false);
            unsafe { qadren_local_free_string(result) };
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
