//! Narrow Job commands exposed to the local macOS application.
//! Execution and state transitions remain inside Rust.
use crate::jobs::JobStore;
use crate::registry::Registry;
use crate::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines;
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

/// Deliver completed Scenario-copy baselines before a later dry-run reads the
/// Registry. This never scans an adapter, opens content, or repeats a copy.
/// A Registry failure leaves the JobStore outbox pending.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_dispatch_scenario_copy_baselines(
    database_path: *const c_char,
    registry_path: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path)) = (input(database_path), input(registry_path))
    else {
        return response(json!({"ok":false,"error":"invalid_request"}));
    };
    let result = (|| {
        let registry = Registry::open(Path::new(registry_path))
            .map_err(|_| crate::jobs::JobStoreError::NotFound)?;
        let mut jobs = JobStore::open(Path::new(database_path))?;
        let dispatched = dispatch_pending_scenario_copy_baselines(&registry, &mut jobs)
            .map_err(|_| crate::jobs::JobStoreError::NotFound)?;
        Ok::<_, crate::jobs::JobStoreError>(dispatched)
    })();
    match result {
        Ok(dispatched) => response(json!({"ok":true,"data":{"dispatched":dispatched}})),
        Err(_) => response(json!({"ok":false,"error":"scenario_baseline_dispatch_unavailable"})),
    }
}

/// Read-only reconciliation for one UNKNOWN Local copy Job.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_reconcile_local_file_copy(
    database_path: *const c_char, registry_path: *const c_char,
    destination_root: *const c_char, destination_connection_id: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database), Some(registry), Some(root), Some(connection), Some(job)) = (
        input(database_path), input(registry_path), input(destination_root),
        input(destination_connection_id), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::reconcile_local_file_copy(
        Path::new(database), Path::new(registry), Path::new(root), connection, job,
    ) {
        Ok(crate::job_execution::LocalFileCopyReconciliationStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed","reconciled":true}})),
        Ok(crate::job_execution::LocalFileCopyReconciliationStatus::StillUnknown) => response(json!({"ok":true,"data":{"status":"unknown","reconciled":false}})),
        Ok(crate::job_execution::LocalFileCopyReconciliationStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true,"reconciled":false}})),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => response(json!({"ok":false,"error":"invalid_request"})),
        Err(_) => response(json!({"ok":false,"error":"local_copy_reconciliation_unavailable"})),
    }
}

/// List existing UNKNOWN Local Filesystem copy Jobs that are durably bound to
/// the supplied destination root and enabled connection. This inventory does
/// not inspect target paths, read content, complete a Job, or retry an effect.
/// JobStore retains its normal restart-to-UNKNOWN rule.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_list_local_file_copy_reconciliations(
    database_path: *const c_char,
    registry_path: *const c_char,
    destination_root: *const c_char,
    destination_connection_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(destination_root), Some(destination_connection_id)) = (
        input(database_path),
        input(registry_path),
        input(destination_root),
        input(destination_connection_id),
    ) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::local_file_copy_reconciliation_candidates(
        Path::new(database_path),
        Path::new(registry_path),
        Path::new(destination_root),
        destination_connection_id,
    ) {
        Ok(candidates) => response(json!({
            "ok": true,
            "data": candidates.into_iter().map(|candidate| json!({"job_id": candidate.job_id})).collect::<Vec<_>>()
        })),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::LocalFileCopyError::ConnectionUnavailable) => {
            response(json!({"ok": false, "error": "local_connection_unavailable"}))
        }
        Err(crate::job_execution::LocalFileCopyError::Local(_)) => {
            response(json!({"ok": false, "error": "reconciliation_unavailable"}))
        }
        Err(crate::job_execution::LocalFileCopyError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "reconciliation_failed"})),
    }
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

/// Seed one durable UNKNOWN mock attempt for the feature-gated native UI test
/// fixture. This symbol is absent from the production Core build.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[cfg(feature = "ui-test-fixtures")]
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_seed_unknown_ui_fixture(
    database_path: *const c_char,
    storage_root: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(storage_root), Some(job_id)) =
        (input(database_path), input(storage_root), input(job_id))
    else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::seed_unknown_mock_create_for_ui_fixture(
        Path::new(database_path),
        Path::new(storage_root),
        job_id,
    ) {
        Ok(()) => response(json!({"ok": true, "data": {"status": "unknown"}})),
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(_) => response(json!({"ok": false, "error": "fixture_failed"})),
    }
}

/// Execute one explicit Local Filesystem create-directory Job. Rust persists
/// the attempt before `mkdir`; the operation never reads user-file contents.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_directory_create(
    database_path: *const c_char,
    registry_path: *const c_char,
    local_root: *const c_char,
    connection_id: *const c_char,
    relative_path: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (
        Some(database_path),
        Some(registry_path),
        Some(local_root),
        Some(connection_id),
        Some(relative_path),
        Some(job_id),
    ) = (
        input(database_path),
        input(registry_path),
        input(local_root),
        input(connection_id),
        input(relative_path),
        input(job_id),
    )
    else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::run_local_directory_create(
        Path::new(database_path),
        Path::new(registry_path),
        Path::new(local_root),
        connection_id,
        Path::new(relative_path),
        job_id,
    ) {
        Ok(crate::job_execution::LocalDirectoryCreateStatus::Completed) => {
            response(json!({"ok": true, "data": {"status": "completed"}}))
        }
        Ok(crate::job_execution::LocalDirectoryCreateStatus::Existing(state)) => response(
            json!({"ok": true, "data": {"status": format!("{:?}", state).to_ascii_lowercase(), "existing": true}}),
        ),
        Err(crate::job_execution::LocalDirectoryCreateError::InvalidRequest) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::TargetExists) => {
            response(json!({"ok": false, "error": "target_exists"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::CapabilityUnavailable) => {
            response(json!({"ok": false, "error": "local_capability_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::ConnectionUnavailable) => {
            response(json!({"ok": false, "error": "local_connection_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::VerificationFailed) => {
            response(json!({"ok": false, "error": "verification_failed"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::OutcomeUnknown(_)) => {
            response(json!({"ok": false, "error": "outcome_unknown"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Local(_)) => {
            response(json!({"ok": false, "error": "local_operation_failed"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
    }
}

/// Execute one explicitly confirmed Local-to-Local file copy. The caller must
/// pass `consent=true` only after a user approved possible File Provider
/// materialization for this exact source file.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_file_copy(
    database_path: *const c_char,
    registry_path: *const c_char,
    source_root: *const c_char,
    source_connection_id: *const c_char,
    source_relative: *const c_char,
    destination_root: *const c_char,
    destination_connection_id: *const c_char,
    destination_relative: *const c_char,
    job_id: *const c_char,
    consent: bool,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(source_root), Some(source_connection_id),
        Some(source_relative), Some(destination_root), Some(destination_connection_id),
        Some(destination_relative), Some(job_id)) = (
        input(database_path), input(registry_path), input(source_root), input(source_connection_id),
        input(source_relative), input(destination_root), input(destination_connection_id),
        input(destination_relative), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::run_local_file_copy(
        Path::new(database_path), Path::new(registry_path), Path::new(source_root), source_connection_id,
        Path::new(source_relative), Path::new(destination_root), destination_connection_id,
        Path::new(destination_relative), job_id, consent,
    ) {
        Ok(crate::job_execution::LocalFileCopyStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed"}})),
        Ok(crate::job_execution::LocalFileCopyStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true}})),
        Err(crate::job_execution::LocalFileCopyError::ConsentRequired) => response(json!({"ok":false,"error":"consent_required"})),
        Err(crate::job_execution::LocalFileCopyError::TargetExists) => response(json!({"ok":false,"error":"target_exists"})),
        Err(crate::job_execution::LocalFileCopyError::ConnectionUnavailable) => response(json!({"ok":false,"error":"local_connection_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioUnavailable) => response(json!({"ok":false,"error":"scenario_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioScopeMismatch) => response(json!({"ok":false,"error":"scenario_scope_mismatch"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictUnavailable) => response(json!({"ok":false,"error":"conflict_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictStale) => response(json!({"ok":false,"error":"conflict_stale"})),
        Err(crate::job_execution::LocalFileCopyError::CapabilityUnavailable) => response(json!({"ok":false,"error":"local_capability_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::VerificationFailed) => response(json!({"ok":false,"error":"verification_failed"})),
        Err(crate::job_execution::LocalFileCopyError::OutcomeUnknown(_)) => response(json!({"ok":false,"error":"outcome_unknown"})),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => response(json!({"ok":false,"error":"invalid_request"})),
        Err(crate::job_execution::LocalFileCopyError::Local(_)) => response(json!({"ok":false,"error":"local_operation_failed"})),
        Err(crate::job_execution::LocalFileCopyError::Store(_)) => response(json!({"ok":false,"error":"job_store_unavailable"})),
    }
}

/// Execute one explicitly confirmed replacement of an existing Local file.
/// The caller must pass `consent=true` only after a user approved possible File
/// Provider materialization for this exact source file.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_file_replace(
    database_path: *const c_char,
    registry_path: *const c_char,
    source_root: *const c_char,
    source_connection_id: *const c_char,
    source_relative: *const c_char,
    destination_root: *const c_char,
    destination_connection_id: *const c_char,
    destination_relative: *const c_char,
    job_id: *const c_char,
    consent: bool,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(source_root), Some(source_connection_id),
        Some(source_relative), Some(destination_root), Some(destination_connection_id),
        Some(destination_relative), Some(job_id)) = (
        input(database_path), input(registry_path), input(source_root), input(source_connection_id),
        input(source_relative), input(destination_root), input(destination_connection_id),
        input(destination_relative), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::run_local_file_replace(
        Path::new(database_path), Path::new(registry_path), Path::new(source_root), source_connection_id,
        Path::new(source_relative), Path::new(destination_root), destination_connection_id,
        Path::new(destination_relative), job_id, consent,
    ) {
        Ok(crate::job_execution::LocalFileCopyStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed"}})),
        Ok(crate::job_execution::LocalFileCopyStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true}})),
        Err(crate::job_execution::LocalFileCopyError::ConsentRequired) => response(json!({"ok":false,"error":"consent_required"})),
        Err(crate::job_execution::LocalFileCopyError::TargetExists) => response(json!({"ok":false,"error":"target_exists"})),
        Err(crate::job_execution::LocalFileCopyError::ConnectionUnavailable) => response(json!({"ok":false,"error":"local_connection_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioUnavailable) => response(json!({"ok":false,"error":"scenario_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioScopeMismatch) => response(json!({"ok":false,"error":"scenario_scope_mismatch"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictUnavailable) => response(json!({"ok":false,"error":"conflict_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictStale) => response(json!({"ok":false,"error":"conflict_stale"})),
        Err(crate::job_execution::LocalFileCopyError::CapabilityUnavailable) => response(json!({"ok":false,"error":"local_capability_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::VerificationFailed) => response(json!({"ok":false,"error":"verification_failed"})),
        Err(crate::job_execution::LocalFileCopyError::OutcomeUnknown(_)) => response(json!({"ok":false,"error":"outcome_unknown"})),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => response(json!({"ok":false,"error":"invalid_request"})),
        Err(crate::job_execution::LocalFileCopyError::Local(_)) => response(json!({"ok":false,"error":"local_operation_failed"})),
        Err(crate::job_execution::LocalFileCopyError::Store(_)) => response(json!({"ok":false,"error":"job_store_unavailable"})),
    }
}

/// Execute one explicitly confirmed copy bound to an immutable Scenario revision.
/// The Rust executor rejects roots and connections that are not the direction
/// stored in that revision.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_scenario_file_copy(
    database_path: *const c_char, registry_path: *const c_char,
    scenario_id: *const c_char, scenario_revision: u64,
    source_root: *const c_char, source_connection_id: *const c_char, source_relative: *const c_char,
    destination_root: *const c_char, destination_connection_id: *const c_char, destination_relative: *const c_char,
    job_id: *const c_char, consent: bool,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(scenario_id), Some(source_root),
        Some(source_connection_id), Some(source_relative), Some(destination_root),
        Some(destination_connection_id), Some(destination_relative), Some(job_id)) = (
        input(database_path), input(registry_path), input(scenario_id), input(source_root),
        input(source_connection_id), input(source_relative), input(destination_root),
        input(destination_connection_id), input(destination_relative), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::run_local_scenario_file_copy(
        Path::new(database_path), Path::new(registry_path), scenario_id, scenario_revision,
        Path::new(source_root), source_connection_id, Path::new(source_relative),
        Path::new(destination_root), destination_connection_id, Path::new(destination_relative), job_id, consent,
    ) {
        Ok(crate::job_execution::LocalFileCopyStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed"}})),
        Ok(crate::job_execution::LocalFileCopyStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true}})),
        Err(crate::job_execution::LocalFileCopyError::ConsentRequired) => response(json!({"ok":false,"error":"consent_required"})),
        Err(crate::job_execution::LocalFileCopyError::TargetExists) => response(json!({"ok":false,"error":"target_exists"})),
        Err(crate::job_execution::LocalFileCopyError::ConnectionUnavailable) => response(json!({"ok":false,"error":"local_connection_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioUnavailable) => response(json!({"ok":false,"error":"scenario_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioScopeMismatch) => response(json!({"ok":false,"error":"scenario_scope_mismatch"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictUnavailable) => response(json!({"ok":false,"error":"conflict_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictStale) => response(json!({"ok":false,"error":"conflict_stale"})),
        Err(crate::job_execution::LocalFileCopyError::CapabilityUnavailable) => response(json!({"ok":false,"error":"local_capability_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::VerificationFailed) => response(json!({"ok":false,"error":"verification_failed"})),
        Err(crate::job_execution::LocalFileCopyError::OutcomeUnknown(_)) => response(json!({"ok":false,"error":"outcome_unknown"})),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => response(json!({"ok":false,"error":"invalid_request"})),
        Err(crate::job_execution::LocalFileCopyError::Local(_)) => response(json!({"ok":false,"error":"local_operation_failed"})),
        Err(crate::job_execution::LocalFileCopyError::Store(_)) => response(json!({"ok":false,"error":"job_store_unavailable"})),
    }
}

/// Execute one explicitly confirmed Scenario update through controlled Local
/// replacement. Rust validates the immutable revision and endpoint direction.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_scenario_file_replace(
    database_path: *const c_char, registry_path: *const c_char,
    scenario_id: *const c_char, scenario_revision: u64,
    source_root: *const c_char, source_connection_id: *const c_char, source_relative: *const c_char,
    destination_root: *const c_char, destination_connection_id: *const c_char, destination_relative: *const c_char,
    job_id: *const c_char, consent: bool,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(scenario_id), Some(source_root),
        Some(source_connection_id), Some(source_relative), Some(destination_root),
        Some(destination_connection_id), Some(destination_relative), Some(job_id)) = (
        input(database_path), input(registry_path), input(scenario_id), input(source_root),
        input(source_connection_id), input(source_relative), input(destination_root),
        input(destination_connection_id), input(destination_relative), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::run_local_scenario_file_replace(
        Path::new(database_path), Path::new(registry_path), scenario_id, scenario_revision,
        Path::new(source_root), source_connection_id, Path::new(source_relative),
        Path::new(destination_root), destination_connection_id, Path::new(destination_relative), job_id, consent,
    ) {
        Ok(crate::job_execution::LocalFileCopyStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed"}})),
        Ok(crate::job_execution::LocalFileCopyStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true}})),
        Err(crate::job_execution::LocalFileCopyError::ConsentRequired) => response(json!({"ok":false,"error":"consent_required"})),
        Err(crate::job_execution::LocalFileCopyError::TargetExists) => response(json!({"ok":false,"error":"target_exists"})),
        Err(crate::job_execution::LocalFileCopyError::ConnectionUnavailable) => response(json!({"ok":false,"error":"local_connection_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioUnavailable) => response(json!({"ok":false,"error":"scenario_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ScenarioScopeMismatch) => response(json!({"ok":false,"error":"scenario_scope_mismatch"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictUnavailable) => response(json!({"ok":false,"error":"conflict_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictStale) => response(json!({"ok":false,"error":"conflict_stale"})),
        Err(crate::job_execution::LocalFileCopyError::CapabilityUnavailable) => response(json!({"ok":false,"error":"local_capability_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::VerificationFailed) => response(json!({"ok":false,"error":"verification_failed"})),
        Err(crate::job_execution::LocalFileCopyError::OutcomeUnknown(_)) => response(json!({"ok":false,"error":"outcome_unknown"})),
        Err(crate::job_execution::LocalFileCopyError::InvalidRequest) => response(json!({"ok":false,"error":"invalid_request"})),
        Err(crate::job_execution::LocalFileCopyError::Local(_)) => response(json!({"ok":false,"error":"local_operation_failed"})),
        Err(crate::job_execution::LocalFileCopyError::Store(_)) => response(json!({"ok":false,"error":"job_store_unavailable"})),
    }
}

/// Execute one selected conflict replacement after snapshot revalidation.
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_run_local_scenario_conflict_replace(
    database_path: *const c_char, registry_path: *const c_char,
    conflict_id: *const c_char, job_id: *const c_char, consent: bool,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(conflict_id), Some(job_id)) = (
        input(database_path), input(registry_path), input(conflict_id), input(job_id),
    ) else { return response(json!({"ok":false,"error":"invalid_request"})); };
    match crate::job_execution::run_local_scenario_conflict_replace(
        Path::new(database_path), Path::new(registry_path), conflict_id, job_id, consent,
    ) {
        Ok(crate::job_execution::LocalFileCopyStatus::Completed) => response(json!({"ok":true,"data":{"status":"completed"}})),
        Ok(crate::job_execution::LocalFileCopyStatus::Existing(state)) => response(json!({"ok":true,"data":{"status":format!("{:?}", state).to_ascii_lowercase(),"existing":true}})),
        Err(crate::job_execution::LocalFileCopyError::ConflictStale) => response(json!({"ok":false,"error":"conflict_stale"})),
        Err(crate::job_execution::LocalFileCopyError::ConflictUnavailable) => response(json!({"ok":false,"error":"conflict_unavailable"})),
        Err(crate::job_execution::LocalFileCopyError::ConsentRequired) => response(json!({"ok":false,"error":"consent_required"})),
        Err(crate::job_execution::LocalFileCopyError::OutcomeUnknown(_)) => response(json!({"ok":false,"error":"outcome_unknown"})),
        Err(_) => response(json!({"ok":false,"error":"conflict_replace_failed"})),
    }
}

/// Read-only reconciliation for one UNKNOWN Local Filesystem directory-create
/// Job. This never starts another mkdir; Rust requires the exact durable root
/// identity, connection, operation and directory metadata before completion.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_reconcile_local_directory_create(
    database_path: *const c_char,
    registry_path: *const c_char,
    local_root: *const c_char,
    connection_id: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (
        Some(database_path),
        Some(registry_path),
        Some(local_root),
        Some(connection_id),
        Some(job_id),
    ) = (
        input(database_path),
        input(registry_path),
        input(local_root),
        input(connection_id),
        input(job_id),
    )
    else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::reconcile_local_directory_create(
        Path::new(database_path),
        Path::new(registry_path),
        Path::new(local_root),
        connection_id,
        job_id,
    ) {
        Ok(crate::job_execution::LocalDirectoryReconciliationStatus::Completed) => {
            response(json!({"ok": true, "data": {"status": "completed", "reconciled": true}}))
        }
        Ok(crate::job_execution::LocalDirectoryReconciliationStatus::StillUnknown) => {
            response(json!({"ok": true, "data": {"status": "unknown", "reconciled": false}}))
        }
        Ok(crate::job_execution::LocalDirectoryReconciliationStatus::Existing(state)) => response(
            json!({"ok": true, "data": {"status": format!("{:?}", state).to_ascii_lowercase(), "existing": true, "reconciled": false}}),
        ),
        Err(crate::job_execution::LocalDirectoryCreateError::InvalidRequest) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::ConnectionUnavailable) => {
            response(json!({"ok": false, "error": "local_connection_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Store(
            crate::jobs::JobStoreError::NotFound,
        )) => response(json!({"ok": false, "error": "not_found"})),
        // Root or target inspection errors never become successful recovery.
        Err(crate::job_execution::LocalDirectoryCreateError::Local(_))
        | Err(crate::job_execution::LocalDirectoryCreateError::VerificationFailed) => {
            response(json!({"ok": false, "error": "reconciliation_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "reconciliation_failed"})),
    }
}

/// List existing UNKNOWN Local Filesystem directory-create Jobs that are
/// durably bound to the supplied root and enabled connection. This inventory
/// operation never inspects a target path, completes a Job, or retries an
/// effect. JobStore retains its normal restart-to-UNKNOWN rule.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_list_local_directory_reconciliations(
    database_path: *const c_char,
    registry_path: *const c_char,
    local_root: *const c_char,
    connection_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(registry_path), Some(local_root), Some(connection_id)) = (
        input(database_path),
        input(registry_path),
        input(local_root),
        input(connection_id),
    ) else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::local_directory_reconciliation_candidates(
        Path::new(database_path),
        Path::new(registry_path),
        Path::new(local_root),
        connection_id,
    ) {
        Ok(candidates) => response(json!({
            "ok": true,
            "data": candidates.into_iter().map(|candidate| json!({"job_id": candidate.job_id})).collect::<Vec<_>>()
        })),
        Err(crate::job_execution::LocalDirectoryCreateError::InvalidRequest) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::ConnectionUnavailable) => {
            response(json!({"ok": false, "error": "local_connection_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Local(_)) => {
            response(json!({"ok": false, "error": "reconciliation_unavailable"}))
        }
        Err(crate::job_execution::LocalDirectoryCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "reconciliation_failed"})),
    }
}

/// Read-only reconciliation for one app-owned UNKNOWN G1 mock Job.  It never
/// starts another create operation; a durable expectation and exact metadata
/// match are required before the existing attempt can be completed.
///
/// # Safety
/// Arguments must be valid, NUL-terminated strings for the duration of call.
#[no_mangle]
pub unsafe extern "C" fn qadren_jobs_reconcile_mock_create(
    database_path: *const c_char,
    storage_root: *const c_char,
    job_id: *const c_char,
) -> *mut c_char {
    let (Some(database_path), Some(storage_root), Some(job_id)) =
        (input(database_path), input(storage_root), input(job_id))
    else {
        return response(json!({"ok": false, "error": "invalid_request"}));
    };
    match crate::job_execution::reconcile_mock_create(
        Path::new(database_path),
        Path::new(storage_root),
        job_id,
    ) {
        Ok(crate::job_execution::MockReconciliationStatus::Completed) => {
            response(json!({"ok": true, "data": {"status": "completed", "reconciled": true}}))
        }
        Ok(crate::job_execution::MockReconciliationStatus::StillUnknown) => {
            response(json!({"ok": true, "data": {"status": "unknown", "reconciled": false}}))
        }
        Ok(crate::job_execution::MockReconciliationStatus::Existing(state)) => response(
            json!({"ok": true, "data": {"status": format!("{:?}", state).to_ascii_lowercase(), "existing": true, "reconciled": false}}),
        ),
        Err(crate::job_execution::MockCreateError::InvalidJobId) => {
            response(json!({"ok": false, "error": "invalid_request"}))
        }
        Err(crate::job_execution::MockCreateError::Store(crate::jobs::JobStoreError::NotFound)) => {
            response(json!({"ok": false, "error": "not_found"}))
        }
        // A root mismatch or unavailable storage never becomes a successful
        // reconciliation and leaves the durable Job untouched.
        Err(crate::job_execution::MockCreateError::OccupiedTarget)
        | Err(crate::job_execution::MockCreateError::Io(_)) => {
            response(json!({"ok": false, "error": "reconciliation_unavailable"}))
        }
        Err(crate::job_execution::MockCreateError::Store(_)) => {
            response(json!({"ok": false, "error": "job_store_unavailable"}))
        }
        Err(_) => response(json!({"ok": false, "error": "reconciliation_failed"})),
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
    fn reconciliation_completes_a_durably_unknown_mock_attempt_through_the_c_abi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-reconcile-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let database_path = base.join("jobs.sqlite3");
        let storage_path = base.join("storage");
        fs::create_dir(&storage_path).unwrap();
        let job_id = "unknown-job";
        let target = "unknown-job.qadren-g1-test";
        let content = b"QADREN G1 mock create artifact\n";
        fs::write(storage_path.join(target), content).unwrap();
        let mut store = JobStore::open(&database_path).unwrap();
        store.enqueue(job_id, None).unwrap();
        store
            .advance(
                job_id,
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(crate::confirmation::EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt(
                job_id,
                "unknown-job-attempt-1",
                "mock_create_new",
                crate::job_execution::MOCK_CONNECTION_ID,
                target,
                content.len() as u64,
                &format!("{:x}", md5::compute(content)),
            )
            .unwrap();
        drop(store);

        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let storage = CString::new(storage_path.to_str().unwrap()).unwrap();
        let job = CString::new(job_id).unwrap();
        let response = unsafe {
            qadren_jobs_reconcile_mock_create(database.as_ptr(), storage.as_ptr(), job.as_ptr())
        };
        let message = unsafe { CStr::from_ptr(response) }.to_str().unwrap();
        assert!(message.contains("\"reconciled\":true"));
        unsafe { qadren_jobs_free_string(response) };
        let store = JobStore::open(&database_path).unwrap();
        assert_eq!(store.state(job_id).unwrap(), Some(JobState::Completed));
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

    #[test]
    fn local_directory_command_executes_through_the_c_abi() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-ffi-local-directory-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir(&base).unwrap();
        let database = CString::new(base.join("jobs.sqlite3").to_str().unwrap()).unwrap();
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let connection_id = registry.add("local_filesystem", "Test Local").unwrap();
        let registry = CString::new(registry_path.to_str().unwrap()).unwrap();
        let root = CString::new(base.to_str().unwrap()).unwrap();
        let connection = CString::new(connection_id).unwrap();
        let relative = CString::new("created-by-ffi").unwrap();
        let job = CString::new("local-mkdir-ffi").unwrap();
        let response = unsafe {
            qadren_jobs_run_local_directory_create(
                database.as_ptr(),
                registry.as_ptr(),
                root.as_ptr(),
                connection.as_ptr(),
                relative.as_ptr(),
                job.as_ptr(),
            )
        };
        assert!(unsafe { CStr::from_ptr(response) }
            .to_str()
            .unwrap()
            .contains("completed"));
        unsafe { qadren_jobs_free_string(response) };
        assert!(base.join("created-by-ffi").is_dir());
    }

    #[test]
    fn local_directory_reconciliation_requires_the_same_root_through_the_c_abi() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-ffi-local-reconcile-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir(&base).unwrap();
        let database_path = base.join("jobs.sqlite3");
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let connection_id = registry.add("local_filesystem", "Test Local").unwrap();
        let local = crate::local_fs::LocalFilesystem::open(&base).unwrap();
        let identity = local.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let job_id = "local-mkdir-ffi-reconcile";
        let mut store = JobStore::open(&database_path).unwrap();
        store.enqueue(job_id, None).unwrap();
        store
            .advance(
                job_id,
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(crate::confirmation::EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                job_id,
                "local-mkdir-ffi-reconcile-attempt-1",
                "local_mkdir_new",
                &connection_id,
                "created-by-reconciliation",
                0,
                "d41d8cd98f00b204e9800998ecf8427e",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::create_dir(base.join("created-by-reconciliation")).unwrap();

        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let registry = CString::new(registry_path.to_str().unwrap()).unwrap();
        let root = CString::new(base.to_str().unwrap()).unwrap();
        let connection = CString::new(connection_id).unwrap();
        let job = CString::new(job_id).unwrap();
        let candidates = unsafe {
            qadren_jobs_list_local_directory_reconciliations(
                database.as_ptr(),
                registry.as_ptr(),
                root.as_ptr(),
                connection.as_ptr(),
            )
        };
        assert!(unsafe { CStr::from_ptr(candidates) }
            .to_str()
            .unwrap()
            .contains("local-mkdir-ffi-reconcile"));
        unsafe { qadren_jobs_free_string(candidates) };
        let response = unsafe {
            qadren_jobs_reconcile_local_directory_create(
                database.as_ptr(),
                registry.as_ptr(),
                root.as_ptr(),
                connection.as_ptr(),
                job.as_ptr(),
            )
        };
        assert!(unsafe { CStr::from_ptr(response) }
            .to_str()
            .unwrap()
            .contains("\"reconciled\":true"));
        unsafe { qadren_jobs_free_string(response) };
        let store = JobStore::open(&database_path).unwrap();
        assert_eq!(store.state(job_id).unwrap(), Some(JobState::Completed));
    }

    #[test]
    fn local_copy_reconciliation_completes_exact_unknown_attempt_through_the_c_abi() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-ffi-local-copy-reconcile-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir(&base).unwrap();
        let database_path = base.join("jobs.sqlite3");
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let connection_id = registry.add("local_filesystem", "Test Local").unwrap();
        let local = crate::local_fs::LocalFilesystem::open(&base).unwrap();
        let identity = local.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let job_id = "local-copy-ffi-reconcile";
        let mut store = JobStore::open(&database_path).unwrap();
        store.enqueue(job_id, None).unwrap();
        store
            .advance(
                job_id,
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(crate::confirmation::EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                job_id,
                "local-copy-ffi-reconcile-attempt-1",
                "local_copy_new",
                &connection_id,
                "copied-by-reconciliation.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::write(base.join("copied-by-reconciliation.txt"), b"payload").unwrap();

        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let registry = CString::new(registry_path.to_str().unwrap()).unwrap();
        let root = CString::new(base.to_str().unwrap()).unwrap();
        let connection = CString::new(connection_id).unwrap();
        let job = CString::new(job_id).unwrap();
        let candidates = unsafe {
            qadren_jobs_list_local_file_copy_reconciliations(
                database.as_ptr(),
                registry.as_ptr(),
                root.as_ptr(),
                connection.as_ptr(),
            )
        };
        assert!(unsafe { CStr::from_ptr(candidates) }
            .to_str()
            .unwrap()
            .contains("local-copy-ffi-reconcile"));
        unsafe { qadren_jobs_free_string(candidates) };
        let response = unsafe {
            qadren_jobs_reconcile_local_file_copy(
                database.as_ptr(),
                registry.as_ptr(),
                root.as_ptr(),
                connection.as_ptr(),
                job.as_ptr(),
            )
        };
        assert!(unsafe { CStr::from_ptr(response) }
            .to_str()
            .unwrap()
            .contains("\"reconciled\":true"));
        unsafe { qadren_jobs_free_string(response) };
        let store = JobStore::open(&database_path).unwrap();
        assert_eq!(store.state(job_id).unwrap(), Some(JobState::Completed));
    }

    #[test]
    fn local_file_replace_executes_through_the_c_abi() {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
            "qadren-ffi-local-replace-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let source_root = base.join("source");
        let destination_root = base.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"new contents").unwrap();
        fs::write(destination_root.join("destination.txt"), b"old contents").unwrap();
        let database_path = base.join("jobs.sqlite3");
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let source_connection = registry.add("local_filesystem", "Source Local").unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination Local").unwrap();
        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let registry = CString::new(registry_path.to_str().unwrap()).unwrap();
        let source = CString::new(source_root.to_str().unwrap()).unwrap();
        let destination = CString::new(destination_root.to_str().unwrap()).unwrap();
        let source_connection = CString::new(source_connection).unwrap();
        let destination_connection = CString::new(destination_connection).unwrap();
        let source_relative = CString::new("source.txt").unwrap();
        let destination_relative = CString::new("destination.txt").unwrap();
        let job = CString::new("local-replace-ffi").unwrap();

        let response = unsafe {
            qadren_jobs_run_local_file_replace(
                database.as_ptr(), registry.as_ptr(), source.as_ptr(), source_connection.as_ptr(),
                source_relative.as_ptr(), destination.as_ptr(), destination_connection.as_ptr(),
                destination_relative.as_ptr(), job.as_ptr(), true,
            )
        };
        let message = unsafe { CStr::from_ptr(response) }.to_str().unwrap();
        assert!(message.contains("\"completed\""), "{message}");
        unsafe { qadren_jobs_free_string(response) };
        assert_eq!(fs::read(destination_root.join("destination.txt")).unwrap(), b"new contents");
        assert_eq!(JobStore::open(&database_path).unwrap().state("local-replace-ffi").unwrap(), Some(JobState::Completed));
    }

    #[test]
    fn scenario_file_replace_executes_through_the_c_abi() {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
            "qadren-ffi-scenario-replace-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let source_root = base.join("source");
        let destination_root = base.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"new contents").unwrap();
        fs::write(destination_root.join("report.txt"), b"old contents").unwrap();
        let database_path = base.join("jobs.sqlite3");
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let source_connection = registry.add("local_filesystem", "Source Local").unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination Local").unwrap();
        let scenario = registry
            .create_scenario(
                "Replace report",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let registry = CString::new(registry_path.to_str().unwrap()).unwrap();
        let scenario = CString::new(scenario).unwrap();
        let source = CString::new(source_root.to_str().unwrap()).unwrap();
        let destination = CString::new(destination_root.to_str().unwrap()).unwrap();
        let source_connection = CString::new(source_connection).unwrap();
        let destination_connection = CString::new(destination_connection).unwrap();
        let relative = CString::new("report.txt").unwrap();
        let job = CString::new("scenario-replace-ffi").unwrap();

        let response = unsafe {
            qadren_jobs_run_local_scenario_file_replace(
                database.as_ptr(), registry.as_ptr(), scenario.as_ptr(), 1,
                source.as_ptr(), source_connection.as_ptr(), relative.as_ptr(),
                destination.as_ptr(), destination_connection.as_ptr(), relative.as_ptr(),
                job.as_ptr(), true,
            )
        };
        let message = unsafe { CStr::from_ptr(response) }.to_str().unwrap();
        assert!(message.contains("\"completed\""), "{message}");
        unsafe { qadren_jobs_free_string(response) };
        assert_eq!(fs::read(destination_root.join("report.txt")).unwrap(), b"new contents");
        assert_eq!(
            JobStore::open(&database_path).unwrap().state("scenario-replace-ffi").unwrap(),
            Some(JobState::Completed)
        );
    }

    #[test]
    fn selected_conflict_replace_executes_through_the_c_abi() {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
            "qadren-ffi-conflict-replace-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let source_root = base.join("source");
        let destination_root = base.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"new contents").unwrap();
        fs::write(destination_root.join("report.txt"), b"old contents").unwrap();
        let database_path = base.join("jobs.sqlite3");
        let registry_path = base.join("connections.sqlite3");
        let registry = crate::registry::Registry::open(&registry_path).unwrap();
        let source_connection = registry.add("local_filesystem", "Source Local").unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination Local").unwrap();
        let scenario_id = registry.create_scenario(
            "Conflict report", &source_connection, source_root.to_str().unwrap(),
            &destination_connection, destination_root.to_str().unwrap(), "left_to_right",
        ).unwrap();
        let source = crate::local_fs::LocalFilesystem::open(&source_root).unwrap()
            .stat(Path::new("report.txt")).unwrap();
        let destination = crate::local_fs::LocalFilesystem::open(&destination_root).unwrap()
            .stat(Path::new("report.txt")).unwrap();
        let task = registry.queue_conflict_task(
            &crate::model::ScenarioReference { scenario_id, revision: 1 }, "report.txt", "content", None,
            Some(&crate::model::ConfirmedFileVersion { object_revision: source.object_revision, size: source.size, checksum_md5: None }),
            Some(&crate::model::ConfirmedFileVersion { object_revision: destination.object_revision, size: destination.size, checksum_md5: None }),
        ).unwrap();
        registry.select_conflict_task(&task, Some(true), "replace_from_source").unwrap();
        let database = CString::new(database_path.to_str().unwrap()).unwrap();
        let registry_path = CString::new(registry_path.to_str().unwrap()).unwrap();
        let task = CString::new(task).unwrap();
        let job = CString::new("conflict-replace-ffi").unwrap();

        let response = unsafe {
            qadren_jobs_run_local_scenario_conflict_replace(
                database.as_ptr(), registry_path.as_ptr(), task.as_ptr(), job.as_ptr(), true,
            )
        };
        let message = unsafe { CStr::from_ptr(response) }.to_str().unwrap();
        assert!(message.contains("\"completed\""), "{message}");
        unsafe { qadren_jobs_free_string(response) };
        assert_eq!(fs::read(destination_root.join("report.txt")).unwrap(), b"new contents");
    }
}
