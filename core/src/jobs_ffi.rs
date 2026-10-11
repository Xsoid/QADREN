//! Narrow Job commands exposed to the local macOS application.
//! Execution and state transitions remain inside Rust.
use crate::jobs::JobStore;
use serde_json::json;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

fn input<'a>(pointer: *const c_char) -> Option<&'a str> {
    if pointer.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .ok()
        .filter(|value| !value.is_empty())
}

fn response(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("JSON output cannot contain raw NUL")
        .into_raw()
}

/// Return JSON-encoded persisted jobs for a caller-owned C string path.
///
/// # Safety
/// `path` must be null or point to a valid NUL-terminated C string whose memory
/// remains readable for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_list(path: *const c_char) -> *mut c_char {
    let Some(path) = input(path) else {
        return response(json!({"ok": false, "error": "invalid_path"}));
    };
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

/// Read whether local credential removal must stop for unresolved attempts.
/// This is an inventory check only; it never resolves, retries, or changes Jobs.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_connection_has_unresolved_attempts(
    path: *const c_char,
    connection_id: *const c_char,
) -> *mut c_char {
    let (Some(path), Some(connection_id)) = (input(path), input(connection_id)) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match JobStore::open(Path::new(path))
        .and_then(|store| store.connection_has_unresolved_attempts(connection_id))
    {
        Ok(unresolved) => response(json!({"ok": true, "data": {"unresolved": unresolved}})),
        Err(crate::jobs::JobStoreError::InvalidId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(_) => response(json!({"ok": false, "error": "job_store_unavailable"})),
    }
}

/// Start one explicit, app-owned G1 mock create Job. This command never uses
/// real cloud connections or user content; Rust owns the full execution.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_mock_create(
    database_path: *const c_char,
    storage_root: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(storage_root), Some(job_id)) =
        (input(database_path), input(storage_root), input(job_id))
    else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::run_mock_create(
        Path::new(database_path),
        Path::new(storage_root),
        job_id,
        true,
    ) {
        Ok(crate::job_execution::MockCreateStatus::Queued) => {
            response(json!({"ok": false, "error": "invalid_state"}))
        }
        Ok(crate::job_execution::MockCreateStatus::Completed) => {
            response(json!({"ok": true, "data": {"status": "completed"}}))
        }
        Ok(crate::job_execution::MockCreateStatus::Existing(state)) => response(
            json!({"ok": true, "data": {"status": format!("{:?}", state).to_ascii_lowercase(), "existing": true}}),
        ),
        Err(crate::job_execution::MockCreateError::ConsentRequired) => {
            response(json!({"ok": false, "error": "consent_required"}))
        }
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::OccupiedTarget) => {
            response(json!({"ok": false, "error": "target_exists"}))
        }
        Err(crate::job_execution::MockCreateError::VerificationFailed) => {
            response(json!({"ok": false, "error": "verification_failed"}))
        }
        // I/O after an effect is never converted to success or an automatic retry.
        Err(crate::job_execution::MockCreateError::Io(_))
        | Err(crate::job_execution::MockCreateError::SimulatedCrashAfterEffect) => {
            response(json!({"ok": false, "error": "outcome_unknown_or_io"}))
        }
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
    }
}

/// Persist one app-owned mock Job without starting an effect.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_queue_mock_create(
    database_path: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(job_id)) = (input(database_path), input(job_id)) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::queue_mock_create(Path::new(database_path), job_id) {
        Ok(crate::job_execution::MockCreateStatus::Queued) => {
            response(json!({"ok": true, "data": {"status": "queued"}}))
        }
        Ok(crate::job_execution::MockCreateStatus::Existing(state)) => response(
            json!({"ok": true, "data": {"status": format!("{:?}", state).to_ascii_lowercase(), "existing": true}}),
        ),
        Ok(crate::job_execution::MockCreateStatus::Completed) => {
            response(json!({"ok": false, "error": "invalid_state"}))
        }
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "mock_queue_unavailable"})),
    }
}

/// Cancel only a Job that has no active or unknown operation attempt.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_cancel_mock_create(
    database_path: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(job_id)) = (input(database_path), input(job_id)) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::cancel_mock_create(Path::new(database_path), job_id) {
        Ok(()) => response(json!({"ok": true, "data": {"status": "cancelled"}})),
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::Store(crate::jobs::JobStoreError::NotFound)) => {
            response(json!({"ok": false, "error": "not_found"}))
        }
        Err(crate::job_execution::MockCreateError::Store(
            crate::jobs::JobStoreError::InvalidTransition,
        )) => response(json!({"ok": false, "error": "cannot_cancel_after_effect"})),
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "mock_cancel_unavailable"})),
    }
}

/// Pause only before a mock Job has a durable operation attempt.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_pause_mock_create(
    database_path: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(job_id)) = (input(database_path), input(job_id)) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::pause_mock_create(Path::new(database_path), job_id) {
        Ok(()) => response(json!({"ok": true, "data": {"status": "paused"}})),
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::Store(crate::jobs::JobStoreError::NotFound)) => {
            response(json!({"ok": false, "error": "not_found"}))
        }
        Err(crate::job_execution::MockCreateError::Store(
            crate::jobs::JobStoreError::InvalidTransition,
        )) => response(json!({"ok": false, "error": "cannot_pause_after_effect"})),
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "mock_pause_unavailable"})),
    }
}

/// Resume only a previously paused mock Job; execution still needs Run.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_resume_mock_create(
    database_path: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(job_id)) = (input(database_path), input(job_id)) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::resume_mock_create(Path::new(database_path), job_id) {
        Ok(()) => response(json!({"ok": true, "data": {"status": "queued"}})),
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::Store(crate::jobs::JobStoreError::NotFound)) => {
            response(json!({"ok": false, "error": "not_found"}))
        }
        Err(crate::job_execution::MockCreateError::Store(
            crate::jobs::JobStoreError::InvalidTransition,
        )) => response(json!({"ok": false, "error": "not_paused"})),
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "mock_resume_unavailable"})),
    }
}

/// Release a string returned by `qadren_jobs_list`.
///
/// # Safety
/// `pointer` must be null or a pointer returned by this API that has not already
/// been freed. It must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{confirmation::Confirmation, model::JobState};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    #[test]
    fn null_path_fails_without_opening_store() {
        let ptr = unsafe { qadren_jobs_list(std::ptr::null()) };
        let message = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(message.contains("invalid_path"));
        unsafe { qadren_jobs_free_string(ptr) };
    }

    #[test]
    fn mock_command_rejects_invalid_inputs() {
        let ptr = unsafe {
            qadren_jobs_run_mock_create(std::ptr::null(), std::ptr::null(), std::ptr::null())
        };
        let message = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(message.contains("invalid_request"));
        unsafe { qadren_jobs_free_string(ptr) };
    }

    #[test]
    fn mock_command_executes_through_the_c_abi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-g1-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let database = base.join("jobs.sqlite3");
        let storage = base.join("storage");
        let database = CString::new(database.to_str().unwrap()).unwrap();
        let storage = CString::new(storage.to_str().unwrap()).unwrap();
        let job = CString::new("ffi-job").unwrap();
        let ptr = unsafe {
            qadren_jobs_run_mock_create(database.as_ptr(), storage.as_ptr(), job.as_ptr())
        };
        let message = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(message.contains("completed"));
        unsafe { qadren_jobs_free_string(ptr) };
    }

    #[test]
    fn queued_job_can_be_cancelled_through_the_c_abi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-cancel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let database = CString::new(base.join("jobs.sqlite3").to_str().unwrap()).unwrap();
        let job = CString::new("queued-job").unwrap();
        let queued = unsafe { qadren_jobs_queue_mock_create(database.as_ptr(), job.as_ptr()) };
        assert!(unsafe { CStr::from_ptr(queued) }
            .to_str()
            .unwrap()
            .contains("queued"));
        unsafe { qadren_jobs_free_string(queued) };
        let cancelled = unsafe { qadren_jobs_cancel_mock_create(database.as_ptr(), job.as_ptr()) };
        assert!(unsafe { CStr::from_ptr(cancelled) }
            .to_str()
            .unwrap()
            .contains("cancelled"));
        unsafe { qadren_jobs_free_string(cancelled) };
    }

    #[test]
    fn queued_job_can_pause_and_resume_through_the_c_abi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-pause-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let database = CString::new(base.join("jobs.sqlite3").to_str().unwrap()).unwrap();
        let job = CString::new("paused-job").unwrap();
        let queued = unsafe { qadren_jobs_queue_mock_create(database.as_ptr(), job.as_ptr()) };
        unsafe { qadren_jobs_free_string(queued) };
        let paused = unsafe { qadren_jobs_pause_mock_create(database.as_ptr(), job.as_ptr()) };
        assert!(unsafe { CStr::from_ptr(paused) }
            .to_str()
            .unwrap()
            .contains("paused"));
        unsafe { qadren_jobs_free_string(paused) };
        let resumed = unsafe { qadren_jobs_resume_mock_create(database.as_ptr(), job.as_ptr()) };
        assert!(unsafe { CStr::from_ptr(resumed) }
            .to_str()
            .unwrap()
            .contains("queued"));
        unsafe { qadren_jobs_free_string(resumed) };
    }

    #[test]
    fn connection_attempt_gate_is_available_through_the_c_abi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-connection-gate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let path = base.join("jobs.sqlite3");
        let mut store = JobStore::open(&path).unwrap();
        store.enqueue("job", None).unwrap();
        store
            .advance(
                "job",
                JobState::Queued,
                JobState::Running,
                Confirmation::Unknown,
            )
            .unwrap();
        store
            .prepare_bound_attempt(
                "job",
                "attempt",
                "copy",
                "connection-a",
                "/object",
                1,
                "abcdef0123456789abcdef0123456789",
            )
            .unwrap();
        drop(store);
        let database = CString::new(path.to_str().unwrap()).unwrap();
        let blocked = CString::new("connection-a").unwrap();
        let unaffected = CString::new("connection-b").unwrap();
        let blocked_response = unsafe {
            qadren_jobs_connection_has_unresolved_attempts(database.as_ptr(), blocked.as_ptr())
        };
        assert!(unsafe { CStr::from_ptr(blocked_response) }
            .to_str()
            .unwrap()
            .contains("\"unresolved\":true"));
        unsafe { qadren_jobs_free_string(blocked_response) };
        let unaffected_response = unsafe {
            qadren_jobs_connection_has_unresolved_attempts(database.as_ptr(), unaffected.as_ptr())
        };
        assert!(unsafe { CStr::from_ptr(unaffected_response) }
            .to_str()
            .unwrap()
            .contains("\"unresolved\":false"));
        unsafe { qadren_jobs_free_string(unaffected_response) };
    }
}
