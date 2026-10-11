//! Read-only job inventory exposed to the local macOS application.
//! Execution and state transitions remain inside Rust.
use crate::jobs::JobStore;
use serde_json::json;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

fn response(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("JSON output cannot contain raw NUL")
        .into_raw()
}

#[no_mangle]
pub extern "C" fn qadren_jobs_list(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return response(json!({"ok": false, "error": "invalid_path"}));
    }
    let path = unsafe { CStr::from_ptr(path) };
    let Ok(path) = path.to_str() else {
        return response(json!({"ok": false, "error": "invalid_path"}));
    };
    if path.is_empty() {
        return response(json!({"ok": false, "error": "invalid_path"}));
    }
    match JobStore::open(Path::new(path)).and_then(|store| store.list_jobs()) {
        Ok(jobs) => response(json!({
            "ok": true,
            "data": jobs.into_iter().map(|job| json!({
                "id": job.id,
                "state": format!("{:?}", job.state).to_ascii_lowercase(),
                "scenario_id": job.scenario_id,
                "scenario_revision": job.scenario_revision,
            })).collect::<Vec<_>>()
        })),
        Err(_) => response(json!({"ok": false, "error": "job_store_unavailable"})),
    }
}

#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_path_fails_without_opening_store() {
        let ptr = qadren_jobs_list(std::ptr::null());
        let message = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(message.contains("invalid_path"));
        unsafe { qadren_jobs_free_string(ptr) };
    }
}
