//! Minimal C ABI for the SQLite connection registry.
//! Inputs are UTF-8 owned by the caller; JSON output is caller-freed.
//! No OAuth tokens or file contents are persisted by these methods.
use crate::jobs::JobStore;
use crate::model::{DeletionMode, ScenarioReference, ScenarioScope};
use crate::registry::{Registry, RegistryError, ScenarioUpdate};
use crate::scenario_planner::{ObservedFile, PlanAction, PlanItem, SideChange};
use crate::schedule_dispatch::dispatch_pending_schedule_jobs;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

const MAX_DRY_RUN_SNAPSHOT_BYTES: usize = 1_048_576;
const MAX_DRY_RUN_SNAPSHOT_ENTRIES: usize = 10_000;
/// Upper bound for one Scenario selection. The interface selects objects in
/// one visible folder, so a real selection is far smaller.
const MAX_SELECTED_PATHS: usize = 10_000;

fn input(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(ToOwned::to_owned)
}

fn output(value: Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("JSON-escaped string never contains literal NUL")
        .into_raw()
}

#[derive(Deserialize)]
struct ObservedFileInput {
    relative_path: String,
    object_revision: Option<String>,
    size: u64,
    checksum_md5: Option<String>,
    /// A symbolic link or special file that the snapshot skipped.
    #[serde(default)]
    special: bool,
}

/// Split a snapshot into synchronizable files and skipped special paths.
fn split_snapshot(items: Vec<ObservedFileInput>) -> (Vec<ObservedFile>, Vec<String>) {
    let (special, files): (Vec<_>, Vec<_>) = items.into_iter().partition(|item| item.special);
    (
        files.into_iter().map(Into::into).collect(),
        special.into_iter().map(|item| item.relative_path).collect(),
    )
}

#[derive(Deserialize)]
struct BatchCreateInput {
    relative_path: String,
    source_is_left: bool,
}

impl From<ObservedFileInput> for ObservedFile {
    fn from(value: ObservedFileInput) -> Self {
        Self {
            relative_path: value.relative_path,
            object_revision: value.object_revision,
            size: value.size,
            checksum_md5: value.checksum_md5,
        }
    }
}

fn conflict_snapshot(
    value: Option<ObservedFileInput>,
) -> Option<crate::model::ConfirmedFileVersion> {
    value.map(|value| crate::model::ConfirmedFileVersion {
        object_revision: value.object_revision,
        size: value.size,
        checksum_md5: value.checksum_md5,
    })
}

fn side_change_value(value: SideChange) -> &'static str {
    match value {
        SideChange::Unchanged => "unchanged",
        SideChange::Added => "added",
        SideChange::Removed => "removed",
        SideChange::Changed => "changed",
        SideChange::NeedsVerification => "needs_verification",
    }
}

fn plan_action_value(value: PlanAction) -> Value {
    match value {
        PlanAction::NoAction => json!({"kind":"no_action"}),
        PlanAction::Create { from_left } => json!({"kind":"create", "from_left":from_left}),
        PlanAction::Update { from_left } => json!({"kind":"update", "from_left":from_left}),
        PlanAction::DeleteRequiresDecision { on_left } => {
            json!({"kind":"delete_requires_decision", "on_left":on_left})
        }
        PlanAction::Delete { on_left } => json!({"kind":"delete", "on_left":on_left}),
        PlanAction::Conflict { reason } => json!({"kind":"conflict", "reason":reason}),
        PlanAction::NeedsVerification { reason } => {
            json!({"kind":"needs_verification", "reason":reason})
        }
    }
}

fn plan_item_value(item: PlanItem) -> Value {
    json!({
        "relative_path": item.relative_path,
        "left": side_change_value(item.left),
        "right": side_change_value(item.right),
        "action": plan_action_value(item.action),
    })
}

/// Decode an ABI source selector: `1` is left, `0` is right, and `-1` is not
/// selected. Values outside this set are rejected.
fn source_is_left(value: i32) -> Option<Option<bool>> {
    match value {
        -1 => Some(None),
        0 => Some(Some(false)),
        1 => Some(Some(true)),
        _ => None,
    }
}

fn conflict_task_value(task: crate::registry::ScenarioConflictTask) -> Value {
    json!({
        "id": task.id,
        "scenario_id": task.scenario.scenario_id,
        "scenario_revision": task.scenario.revision,
        "relative_path": task.relative_path,
        "kind": task.kind,
        "left": task.left.map(|v| json!({"object_revision":v.object_revision,"size":v.size,"checksum_md5":v.checksum_md5})),
        "right": task.right.map(|v| json!({"object_revision":v.object_revision,"size":v.size,"checksum_md5":v.checksum_md5})),
        "source_is_left": task.source_is_left,
        "decision": task.decision,
        "state": task.state,
    })
}

/// An SMB root must be inside the QADREN mount of its connection (D-022).
/// A Local Filesystem root is not restricted here.
fn require_roots_belong(
    db: &Registry,
    path: *const c_char,
    endpoints: [(&str, &str); 2],
) -> crate::registry::Result<()> {
    let registry_path = input(path).unwrap_or_default();
    for (connection_id, root) in endpoints {
        if let Some(connection) = db.get(connection_id)? {
            if !crate::storage_access::root_belongs(
                &connection,
                Path::new(&registry_path),
                Path::new(root),
            ) {
                return Err(RegistryError::RootOutsideConnection);
            }
        }
    }
    Ok(())
}

fn with_registry<F>(path: *const c_char, action: F) -> *mut c_char
where
    F: FnOnce(&Registry) -> crate::registry::Result<Value>,
{
    let Some(path) = input(path) else {
        return output(json!({"ok":false,"error":"invalid_path"}));
    };
    if path.trim().is_empty() {
        return output(json!({"ok":false,"error":"invalid_path"}));
    }
    match Registry::open(Path::new(&path)).and_then(|registry| action(&registry)) {
        Ok(value) => output(json!({"ok":true,"data":value})),
        Err(error) => output(json!({"ok":false,"error":registry_error_code(&error)})),
    }
}

/// Stable error codes that the UI explains to the user. Other errors stay
/// generic so that no SQLite or path detail reaches the interface.
fn registry_error_code(error: &RegistryError) -> &'static str {
    match error {
        RegistryError::ScenarioRootsOverlap => "scenario_roots_overlap",
        RegistryError::StaleScenarioRevision => "scenario_revision_stale",
        RegistryError::InvalidSelection => "invalid_selection",
        RegistryError::InvalidEndpoint => "invalid_smb_address",
        RegistryError::RootOutsideConnection => "smb_root_outside_mount",
        RegistryError::UnsupportedYandexScope => "yandex_source_only",
        _ => "registry_error",
    }
}

#[no_mangle]
pub extern "C" fn qadren_registry_list(path: *const c_char) -> *mut c_char {
    with_registry(path, |db| {
        let entries: Vec<Value> = db
            .list()?
            .into_iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "adapter": entry.adapter,
                    "name": entry.name,
                    "enabled": entry.enabled,
                    "status": entry.status,
                    "credential_ref": entry.credential_ref,
                    "provider_account_id": entry.provider_account_id,
                    "endpoint_url": entry.endpoint_url
                })
            })
            .collect();
        Ok(json!(entries))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenarios_list(path: *const c_char) -> *mut c_char {
    with_registry(path, |db| {
        let mut entries = Vec::new();
        for item in db.list_scenarios()? {
            let scope = db.get_scenario_revision_scope(&item.id, item.revision)?;
            let retry_until_success = db.scenario_retry_until_success(&item.id)?;
            entries.push(json!({
                "id": item.id, "name": item.name, "left_connection_id": item.left_connection_id,
                "left_root": item.left_root, "right_connection_id": item.right_connection_id,
                "right_root": item.right_root, "direction": item.direction, "enabled": item.enabled,
                "revision": item.revision, "selected_paths": scope.selected_paths,
                "deletion_mode": scope.deletion_mode.as_str(),
                "retry_until_success": retry_until_success
            }));
        }
        Ok(json!(entries))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenarios_create(
    path: *const c_char,
    name: *const c_char,
    left_id: *const c_char,
    left_root: *const c_char,
    right_id: *const c_char,
    right_root: *const c_char,
    direction: *const c_char,
) -> *mut c_char {
    let (
        Some(name),
        Some(left_id),
        Some(left_root),
        Some(right_id),
        Some(right_root),
        Some(direction),
    ) = (
        input(name),
        input(left_id),
        input(left_root),
        input(right_id),
        input(right_root),
        input(direction),
    )
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        require_roots_belong(db, path, [(&left_id, &left_root), (&right_id, &right_root)])?;
        Ok(
            json!({"id": db.create_scenario(&name, &left_id, &left_root, &right_id, &right_root, &direction)?}),
        )
    })
}

/// The selection and deletion rule of a new revision, as JSON:
/// `{"selected_paths":["Docs"],"deletion_mode":"paranoid"|"simple"}`.
#[derive(Deserialize)]
struct ScopeInput {
    #[serde(default)]
    selected_paths: Vec<String>,
    deletion_mode: String,
}

fn scope_input(value: &str) -> Option<ScenarioScope> {
    let input = serde_json::from_str::<ScopeInput>(value).ok()?;
    if input.selected_paths.len() > MAX_SELECTED_PATHS {
        return None;
    }
    Some(ScenarioScope {
        selected_paths: input.selected_paths,
        deletion_mode: DeletionMode::parse(&input.deletion_mode)?,
    })
}

/// Like `qadren_scenarios_create`, with the selection and deletion rule of
/// the first revision (D-019).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn qadren_scenarios_create_scoped(
    path: *const c_char,
    name: *const c_char,
    left_id: *const c_char,
    left_root: *const c_char,
    right_id: *const c_char,
    right_root: *const c_char,
    direction: *const c_char,
    scope_json: *const c_char,
) -> *mut c_char {
    let (
        Some(name),
        Some(left_id),
        Some(left_root),
        Some(right_id),
        Some(right_root),
        Some(direction),
        Some(scope),
    ) = (
        input(name),
        input(left_id),
        input(left_root),
        input(right_id),
        input(right_root),
        input(direction),
        input(scope_json).as_deref().and_then(scope_input),
    )
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        require_roots_belong(db, path, [(&left_id, &left_root), (&right_id, &right_root)])?;
        Ok(json!({"id": db.create_scenario_with_scope(
            &name, &left_id, &left_root, &right_id, &right_root, &direction, &scope
        )?}))
    })
}

/// Like `qadren_scenarios_update`, with the selection and deletion rule of
/// the new revision (D-019).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn qadren_scenarios_update_scoped(
    path: *const c_char,
    id: *const c_char,
    name: *const c_char,
    left_id: *const c_char,
    left_root: *const c_char,
    right_id: *const c_char,
    right_root: *const c_char,
    direction: *const c_char,
    scope_json: *const c_char,
) -> *mut c_char {
    let (
        Some(id),
        Some(name),
        Some(left_id),
        Some(left_root),
        Some(right_id),
        Some(right_root),
        Some(direction),
        Some(scope),
    ) = (
        input(id),
        input(name),
        input(left_id),
        input(left_root),
        input(right_id),
        input(right_root),
        input(direction),
        input(scope_json).as_deref().and_then(scope_input),
    )
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        require_roots_belong(db, path, [(&left_id, &left_root), (&right_id, &right_root)])?;
        db.update_scenario_with_scope(
            &id,
            ScenarioUpdate {
                name: &name,
                left_connection_id: &left_id,
                left_root: &left_root,
                right_connection_id: &right_id,
                right_root: &right_root,
                direction: &direction,
            },
            &scope,
        )?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenarios_set_enabled(
    path: *const c_char,
    id: *const c_char,
    enabled: u8,
) -> *mut c_char {
    let Some(id) = input(id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if enabled > 1 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    with_registry(path, |db| {
        db.set_scenario_enabled(&id, enabled == 1)?;
        Ok(json!(true))
    })
}

/// Persist a confirmed Create batch before the Worker receives its receipt ID.
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
    items_json: *const c_char,
) -> *mut c_char {
    let (Some(path), Some(scenario_id), Some(items_json)) =
        (input(path), input(scenario_id), input(items_json))
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let Ok(items) = serde_json::from_str::<Vec<BatchCreateInput>>(&items_json) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let children = items
        .into_iter()
        .map(|item| (item.relative_path, item.source_is_left))
        .collect::<Vec<_>>();
    match Registry::open(Path::new(&path)).and_then(|mut db| {
        db.create_scenario_create_batch(
            &ScenarioReference {
                scenario_id,
                revision,
            },
            &children,
        )
    }) {
        Ok(receipt) => output(
            json!({"ok":true,"data":{"id":receipt.id,"cancellation_requested":receipt.cancellation_requested,"paused":receipt.paused,"children":receipt.children}}),
        ),
        Err(error) => output(json!({"ok":false,"error":registry_error_code(&error)})),
    }
}

/// Return durable Create-batch child states. This function is read-only.
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch_get(
    path: *const c_char,
    batch_id: *const c_char,
) -> *mut c_char {
    let (Some(path), Some(batch_id)) = (input(path), input(batch_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    match Registry::open(Path::new(&path)).and_then(|db| db.scenario_create_batch(&batch_id)) {
        Ok(Some(receipt)) => output(json!({
            "ok":true,
            "data":{"id":receipt.id,"cancellation_requested":receipt.cancellation_requested,"paused":receipt.paused,"children":receipt.children}
        })),
        Ok(None) => output(json!({"ok":false,"error":"not_found"})),
        Err(_) => output(json!({"ok":false,"error":"registry_error"})),
    }
}

/// Save a durable cancellation request for one confirmed Create batch. The
/// Worker reads this flag before each child, so the request takes effect after
/// the current file even while the Worker is busy and cannot answer IPC.
/// A repeated request is accepted. This function never starts or stops I/O.
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch_request_cancellation(
    path: *const c_char,
    batch_id: *const c_char,
) -> *mut c_char {
    let (Some(path), Some(batch_id)) = (input(path), input(batch_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let result = Registry::open(Path::new(&path)).and_then(|db| {
        match db.request_scenario_create_batch_cancellation(&batch_id) {
            Ok(()) => Ok(true),
            Err(RegistryError::NotFound) => db
                .scenario_create_batch(&batch_id)
                .map(|receipt| receipt.is_some_and(|receipt| receipt.cancellation_requested)),
            Err(error) => Err(error),
        }
    });
    match result {
        Ok(true) => output(json!({"ok":true,"data":{"status":"cancellation_requested"}})),
        Ok(false) => output(json!({"ok":false,"error":"not_found"})),
        Err(_) => output(json!({"ok":false,"error":"registry_error"})),
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Progress summaries of the batches of one Scenario revision that still
/// have pending children (D-024). Counts only, never the list of children.
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch_summaries(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let reference = ScenarioReference {
        scenario_id,
        revision,
    };
    with_registry(path, |db| {
        let now = unix_now();
        let summaries = db
            .pending_scenario_create_batch_ids_for(&reference)?
            .into_iter()
            .filter_map(|id| db.scenario_create_batch_summary(&id, now).transpose())
            .collect::<crate::registry::Result<Vec<_>>>()?;
        Ok(json!(summaries))
    })
}

/// Progress summary of one batch (D-024).
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch_summary(
    path: *const c_char,
    batch_id: *const c_char,
) -> *mut c_char {
    let Some(batch_id) = input(batch_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.scenario_create_batch_summary(&batch_id, unix_now())?
            .map(|summary| json!(summary))
            .ok_or(RegistryError::NotFound)
    })
}

/// Turn the "retry until success" mode of D-024 on (1) or off (0) for one
/// Scenario. The Scenario revision does not change, so a running batch
/// continues and the Worker reads the new value at the next failure.
#[no_mangle]
pub extern "C" fn qadren_scenario_set_retry_until_success(
    path: *const c_char,
    scenario_id: *const c_char,
    enabled: i32,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.set_scenario_retry_until_success(&scenario_id, enabled != 0)?;
        Ok(json!({"retry_until_success": enabled != 0}))
    })
}

/// Pause (`paused` = 1) or resume (`paused` = 0) a durable Create batch
/// (D-024). The Worker starts no new child of a paused batch; the current
/// child finishes.
#[no_mangle]
pub extern "C" fn qadren_scenario_create_batch_set_paused(
    path: *const c_char,
    batch_id: *const c_char,
    paused: i32,
) -> *mut c_char {
    let (Some(path), Some(batch_id)) = (input(path), input(batch_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    match Registry::open(Path::new(&path))
        .and_then(|db| db.set_scenario_create_batch_paused(&batch_id, paused != 0))
    {
        Ok(()) => output(json!({"ok":true,"data":{"paused":paused != 0}})),
        Err(RegistryError::NotFound) => output(json!({"ok":false,"error":"not_found"})),
        Err(_) => output(json!({"ok":false,"error":"registry_error"})),
    }
}

/// Return unfinished durable Create batches for one exact Scenario revision.
/// This is read-only and does not submit work to the Worker.
#[no_mangle]
pub extern "C" fn qadren_scenario_pending_create_batches(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
) -> *mut c_char {
    let (Some(path), Some(scenario_id)) = (input(path), input(scenario_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    match Registry::open(Path::new(&path)).and_then(|db| {
        db.pending_scenario_create_batches(&ScenarioReference {
            scenario_id,
            revision,
        })
    }) {
        Ok(receipts) => output(json!({
            "ok":true,
            "data":receipts.into_iter().map(|receipt| json!({
                "id":receipt.id,
                "cancellation_requested":receipt.cancellation_requested,"paused":receipt.paused,
                "children":receipt.children,
            })).collect::<Vec<_>>(),
        })),
        Err(_) => output(json!({"ok":false,"error":"registry_error"})),
    }
}

/// Compose a metadata-only dry run for one exact persisted Scenario revision.
/// Both snapshots are caller-supplied JSON arrays; this function never scans
/// adapters, reads content, creates a Job, advances FileSyncState, or records
/// a policy decision.
#[no_mangle]
pub extern "C" fn qadren_scenarios_dry_run(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
    left_snapshot_json: *const c_char,
    right_snapshot_json: *const c_char,
) -> *mut c_char {
    let (Some(scenario_id), Some(left_json), Some(right_json)) = (
        input(scenario_id),
        input(left_snapshot_json),
        input(right_snapshot_json),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if scenario_id.is_empty() || revision == 0 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    if left_json.len() > MAX_DRY_RUN_SNAPSHOT_BYTES || right_json.len() > MAX_DRY_RUN_SNAPSHOT_BYTES
    {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let (Ok(left_snapshot), Ok(right_snapshot)) = (
        serde_json::from_str::<Vec<ObservedFileInput>>(&left_json),
        serde_json::from_str::<Vec<ObservedFileInput>>(&right_json),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if left_snapshot.len().saturating_add(right_snapshot.len()) > MAX_DRY_RUN_SNAPSHOT_ENTRIES {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let reference = ScenarioReference {
        scenario_id,
        revision,
    };
    with_registry(path, |db| {
        let (left_snapshot, skipped_left) = split_snapshot(left_snapshot);
        let (right_snapshot, skipped_right) = split_snapshot(right_snapshot);
        Ok(json!(db
            .plan_scenario_dry_run_with_skipped(
                &reference,
                &left_snapshot,
                &right_snapshot,
                &skipped_left,
                &skipped_right,
            )?
            .into_iter()
            .map(plan_item_value)
            .collect::<Vec<_>>()))
    })
}

/// Add one durable, metadata-only Scenario conflict task. `source_is_left` is
/// `1` for left, `0` for right, or `-1` when the user has not chosen a source.
/// This function does not create or advance a Job.
#[no_mangle]
pub extern "C" fn qadren_scenario_conflicts_queue(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
    relative_path: *const c_char,
    kind: *const c_char,
    source_is_left_value: i32,
    left_snapshot_json: *const c_char,
    right_snapshot_json: *const c_char,
) -> *mut c_char {
    let (
        Some(scenario_id),
        Some(relative_path),
        Some(kind),
        Some(source_is_left),
        Some(left_json),
        Some(right_json),
    ) = (
        input(scenario_id),
        input(relative_path),
        input(kind),
        source_is_left(source_is_left_value),
        input(left_snapshot_json),
        input(right_snapshot_json),
    )
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if scenario_id.is_empty() || revision == 0 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let (Ok(left), Ok(right)) = (
        serde_json::from_str::<Option<ObservedFileInput>>(&left_json),
        serde_json::from_str::<Option<ObservedFileInput>>(&right_json),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let left = conflict_snapshot(left);
    let right = conflict_snapshot(right);
    let reference = ScenarioReference {
        scenario_id,
        revision,
    };
    with_registry(path, |db| {
        Ok(json!({"id": db.queue_conflict_task(
            &reference,
            &relative_path,
            &kind,
            source_is_left,
            left.as_ref(),
            right.as_ref(),
        )?}))
    })
}

/// List the durable conflict tasks belonging to one exact Scenario revision.
/// This function is read-only and never scans an adapter or queries JobStore.
#[no_mangle]
pub extern "C" fn qadren_scenario_conflicts_list(
    path: *const c_char,
    scenario_id: *const c_char,
    revision: u64,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if scenario_id.is_empty() || revision == 0 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let reference = ScenarioReference {
        scenario_id,
        revision,
    };
    with_registry(path, |db| {
        Ok(json!(db
            .list_conflict_tasks(&reference)?
            .into_iter()
            .map(conflict_task_value)
            .collect::<Vec<_>>()))
    })
}

/// Save the user's conflict decision. A source is mandatory: use `1` for
/// left or `0` for right. The selected decision is only queue metadata; this
/// function does not execute a transfer or delete a file.
#[no_mangle]
pub extern "C" fn qadren_scenario_conflicts_select(
    path: *const c_char,
    id: *const c_char,
    source_is_left_value: i32,
    decision: *const c_char,
) -> *mut c_char {
    let (Some(id), Some(source_is_left), Some(decision)) = (
        input(id),
        source_is_left(source_is_left_value),
        input(decision),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if source_is_left.is_none() {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    with_registry(path, |db| {
        db.select_conflict_task(&id, source_is_left, &decision)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_list(
    path: *const c_char,
    scenario_id: *const c_char,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        Ok(json!(db
            .list_scenario_schedules(&scenario_id)?
            .into_iter()
            .map(|item| json!({
                "id": item.id, "scenario_id": item.scenario_id, "timezone": item.timezone,
                "weekday": item.weekday, "hour": item.hour, "minute": item.minute,
                "enabled": item.enabled
            }))
            .collect::<Vec<Value>>()))
    })
}

/// Read durable automatic-trigger state for one Scenario. This is registry
/// metadata only; it neither evaluates schedules nor inspects or changes Jobs.
#[no_mangle]
pub extern "C" fn qadren_scenario_automation_get(
    path: *const c_char,
    scenario_id: *const c_char,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        let state = db.scenario_automation_snapshot(&scenario_id)?;
        Ok(json!({
            "locally_paused": state.locally_paused,
            "pending_automatic_run": state.pending_automatic_run
        }))
    })
}

/// Change only the local automatic-trigger pause state for one Scenario.
/// Pending automatic work is cleared when pausing; Jobs remain owned by JobStore.
#[no_mangle]
pub extern "C" fn qadren_scenario_automation_set_paused(
    path: *const c_char,
    scenario_id: *const c_char,
    paused: u8,
) -> *mut c_char {
    let Some(scenario_id) = input(scenario_id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if paused > 1 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    with_registry(path, |db| {
        db.set_scenario_locally_paused(&scenario_id, paused == 1)?;
        Ok(json!(true))
    })
}

/// Evaluate schedule candidates over a caller-supplied UTC interval. This is
/// observation only: it does not claim occurrences or submit Jobs.
#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_due(
    path: *const c_char,
    after: *const c_char,
    through: *const c_char,
) -> *mut c_char {
    let (Some(after), Some(through)) = (input(after), input(through)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let (Ok(after), Ok(through)) = (
        DateTime::parse_from_rfc3339(&after),
        DateTime::parse_from_rfc3339(&through),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let after = after.with_timezone(&Utc);
    let through = through.with_timezone(&Utc);
    with_registry(path, |db| {
        Ok(json!(db
            .due_schedule_occurrences(after, through)?
            .into_iter()
            .map(|item| json!({
                "scenario_id": item.scenario_id,
                "schedule_id": item.schedule_id,
                "occurrence_key": item.occurrence_key.0
            }))
            .collect::<Vec<Value>>()))
    })
}

/// Stage due automatic schedule slots and submit their durable outbox through
/// the existing JobStore. This only creates queued, revision-pinned Job
/// records: it never runs a Scenario, touches adapters, reads content, or
/// grants consent for a transfer.
#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_dispatch(
    registry_path: *const c_char,
    job_store_path: *const c_char,
    after: *const c_char,
    through: *const c_char,
) -> *mut c_char {
    let (Some(registry_path), Some(job_store_path), Some(after), Some(through)) = (
        input(registry_path),
        input(job_store_path),
        input(after),
        input(through),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if registry_path.trim().is_empty() || job_store_path.trim().is_empty() {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let (Ok(after), Ok(through)) = (
        DateTime::parse_from_rfc3339(&after),
        DateTime::parse_from_rfc3339(&through),
    ) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let after = after.with_timezone(&Utc);
    let through = through.with_timezone(&Utc);
    if through < after {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    let result = (|| {
        let registry = Registry::open(Path::new(&registry_path))?;
        let staged = registry.stage_due_schedule_occurrences(after, through)?;
        let mut jobs = JobStore::open(Path::new(&job_store_path))
            .map_err(|_| crate::registry::RegistryError::IntegrityFailed)?;
        let dispatched = dispatch_pending_schedule_jobs(&registry, &mut jobs)
            .map_err(|_| crate::registry::RegistryError::IntegrityFailed)?;
        Ok::<_, crate::registry::RegistryError>(json!({
            "staged": staged,
            "dispatched": dispatched,
        }))
    })();
    match result {
        Ok(value) => output(json!({"ok":true,"data":value})),
        Err(_) => output(json!({"ok":false,"error":"scheduler_unavailable"})),
    }
}

#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_create(
    path: *const c_char,
    scenario_id: *const c_char,
    timezone: *const c_char,
    weekday: u8,
    hour: u8,
    minute: u8,
) -> *mut c_char {
    let (Some(scenario_id), Some(timezone)) = (input(scenario_id), input(timezone)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        Ok(json!({"id": db.create_scenario_schedule(
            &scenario_id, &timezone, weekday, hour, minute
        )?}))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_set_enabled(
    path: *const c_char,
    scenario_id: *const c_char,
    schedule_id: *const c_char,
    enabled: u8,
) -> *mut c_char {
    let (Some(scenario_id), Some(schedule_id)) = (input(scenario_id), input(schedule_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if enabled > 1 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    with_registry(path, |db| {
        db.set_scenario_schedule_enabled(&scenario_id, &schedule_id, enabled == 1)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenario_schedules_remove(
    path: *const c_char,
    scenario_id: *const c_char,
    schedule_id: *const c_char,
) -> *mut c_char {
    let (Some(scenario_id), Some(schedule_id)) = (input(scenario_id), input(schedule_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.remove_scenario_schedule(&scenario_id, &schedule_id)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenarios_update(
    path: *const c_char,
    id: *const c_char,
    name: *const c_char,
    left_id: *const c_char,
    left_root: *const c_char,
    right_id: *const c_char,
    right_root: *const c_char,
    direction: *const c_char,
) -> *mut c_char {
    let (
        Some(id),
        Some(name),
        Some(left_id),
        Some(left_root),
        Some(right_id),
        Some(right_root),
        Some(direction),
    ) = (
        input(id),
        input(name),
        input(left_id),
        input(left_root),
        input(right_id),
        input(right_root),
        input(direction),
    )
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        require_roots_belong(db, path, [(&left_id, &left_root), (&right_id, &right_root)])?;
        db.update_scenario(
            &id,
            ScenarioUpdate {
                name: &name,
                left_connection_id: &left_id,
                left_root: &left_root,
                right_connection_id: &right_id,
                right_root: &right_root,
                direction: &direction,
            },
        )?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_add(
    path: *const c_char,
    adapter: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(adapter), Some(name)) = (input(adapter), input(name)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| Ok(json!({"id":db.add(&adapter, &name)?})))
}

/// Add one SMB share connection (D-021). The address must not carry a login
/// or a password; the platform keeps them in its credential store.
#[no_mangle]
pub extern "C" fn qadren_registry_add_smb_share(
    path: *const c_char,
    name: *const c_char,
    url: *const c_char,
) -> *mut c_char {
    let (Some(name), Some(url)) = (input(name), input(url)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| Ok(json!({"id": db.add_smb_share(&name, &url)?})))
}

#[no_mangle]
pub extern "C" fn qadren_registry_set_smb_endpoint(
    path: *const c_char,
    id: *const c_char,
    url: *const c_char,
) -> *mut c_char {
    let (Some(id), Some(url)) = (input(id), input(url)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.set_smb_endpoint(&id, &url)?;
        Ok(json!(true))
    })
}

/// Return the ID of the single "This Mac" connection, created when missing
/// (D-020). The product never adds a second Local Filesystem connection.
#[no_mangle]
pub extern "C" fn qadren_registry_ensure_local_computer(
    path: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let Some(name) = input(name) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        Ok(json!({"id": db.ensure_local_computer(&name)?}))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_remove(path: *const c_char, id: *const c_char) -> *mut c_char {
    let (Some(_path), Some(_id)) = (input(path), input(id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    // Keep this legacy ABI symbol for callers that can decode its explicit
    // failure. A direct Registry deletion bypasses ConnectionRemoval's
    // disable and unresolved-Job gates, so it must not mutate local state.
    output(json!({"ok":false,"error":"registry_remove_requires_connection_workflow"}))
}

/// Remove one disabled connection only after a fresh JobStore safety gate.
/// This changes local Registry state only. The macOS removal workflow owns any
/// credential cleanup that must happen before this final local deletion.
///
/// # Safety
/// Arguments must point to valid NUL-terminated UTF-8 strings for this call.
#[no_mangle]
pub unsafe extern "C" fn qadren_registry_remove_after_job_gate(
    registry_path: *const c_char,
    jobs_path: *const c_char,
    id: *const c_char,
) -> *mut c_char {
    let (Some(registry_path), Some(jobs_path), Some(id)) =
        (input(registry_path), input(jobs_path), input(id))
    else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let registry = match Registry::open(Path::new(&registry_path)) {
        Ok(registry) => registry,
        Err(_) => return output(json!({"ok":false,"error":"registry_unavailable"})),
    };
    let connection = match registry.get(&id) {
        Ok(Some(connection)) => connection,
        Ok(None) => return output(json!({"ok":false,"error":"connection_not_found"})),
        Err(_) => return output(json!({"ok":false,"error":"registry_unavailable"})),
    };
    if connection.enabled {
        return output(json!({"ok":false,"error":"connection_not_disabled"}));
    }
    let jobs = match JobStore::open(Path::new(&jobs_path)) {
        Ok(jobs) => jobs,
        Err(_) => return output(json!({"ok":false,"error":"job_store_unavailable"})),
    };
    match jobs.connection_has_unresolved_attempts(&id) {
        Ok(true) => output(json!({"ok":false,"error":"connection_has_unresolved_jobs"})),
        Ok(false) => match registry.remove(&id) {
            Ok(()) => output(json!({"ok":true,"data":true})),
            Err(RegistryError::ConnectionReferencedByScenario) => {
                output(json!({"ok":false,"error":"connection_referenced_by_scenario"}))
            }
            Err(_) => output(json!({"ok":false,"error":"registry_unavailable"})),
        },
        Err(_) => output(json!({"ok":false,"error":"job_store_unavailable"})),
    }
}

#[no_mangle]
pub extern "C" fn qadren_registry_rename(
    path: *const c_char,
    id: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(id), Some(name)) = (input(id), input(name)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.rename(&id, &name)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_set_enabled(
    path: *const c_char,
    id: *const c_char,
    enabled: u8,
) -> *mut c_char {
    let Some(id) = input(id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    if enabled > 1 {
        return output(json!({"ok":false,"error":"invalid_input"}));
    }
    with_registry(path, |db| {
        db.set_enabled(&id, enabled == 1)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_finalize_disconnect(
    path: *const c_char,
    id: *const c_char,
) -> *mut c_char {
    let Some(id) = input(id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.finalize_disconnect(&id)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_set_credential(
    path: *const c_char,
    id: *const c_char,
    credential_ref: *const c_char,
) -> *mut c_char {
    let Some(id) = input(id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    let reference = input(credential_ref);
    with_registry(path, |db| {
        db.set_credential_ref(&id, reference.as_deref())?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_set_status(
    path: *const c_char,
    id: *const c_char,
    status: *const c_char,
    at: i64,
) -> *mut c_char {
    let (Some(id), Some(status)) = (input(id), input(status)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.mark_checked(&id, &status, at, None)?;
        Ok(json!(true))
    })
}

#[no_mangle]
pub extern "C" fn qadren_registry_set_provider_account(
    path: *const c_char,
    id: *const c_char,
    account_id: *const c_char,
) -> *mut c_char {
    let (Some(id), Some(account_id)) = (input(id), input(account_id)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.set_provider_account_id(&id, Some(&account_id))?;
        Ok(json!(true))
    })
}

/// # Safety
/// `ptr` must be a pointer returned by this module and not already freed.
#[no_mangle]
pub unsafe extern "C" fn qadren_registry_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_batch_receipt_states_round_trip_through_ffi() {
        let file = std::env::temp_dir().join(format!(
            "qadren-batch-receipt-ffi-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut registry = Registry::open(&file).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario("Receipt", &left, "/left", &right, "/right", "left_to_right")
            .unwrap();
        let receipt = registry
            .create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: scenario.clone(),
                    revision: 1,
                },
                &[("report.txt".into(), true)],
            )
            .unwrap();
        drop(registry);
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let scenario_id = CString::new(scenario.clone()).unwrap();
        let pointer =
            qadren_scenario_pending_create_batches(path.as_ptr(), scenario_id.as_ptr(), 1);
        let response: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(pointer) };
        assert_eq!(response["data"][0]["id"], receipt.id);

        let registry = Registry::open(&file).unwrap();
        registry
            .set_scenario_create_batch_child_state(&receipt.id, "report.txt", "completed")
            .unwrap();
        drop(registry);
        let id = CString::new(receipt.id).unwrap();
        let pointer = qadren_scenario_create_batch_get(path.as_ptr(), id.as_ptr());
        let response: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(pointer) };
        assert_eq!(
            response["data"]["children"][0]["relative_path"],
            "report.txt"
        );
        assert_eq!(response["data"]["children"][0]["state"], "completed");
        let pointer =
            qadren_scenario_pending_create_batches(path.as_ptr(), scenario_id.as_ptr(), 1);
        let response: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(pointer) };
        assert!(response["data"].as_array().unwrap().is_empty());

        for _ in 0..2 {
            let pointer =
                qadren_scenario_create_batch_request_cancellation(path.as_ptr(), id.as_ptr());
            let response: Value =
                serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
            unsafe { qadren_registry_free_string(pointer) };
            assert_eq!(response["data"]["status"], "cancellation_requested");
        }
        assert!(
            Registry::open(&file)
                .unwrap()
                .scenario_create_batch(id.to_str().unwrap())
                .unwrap()
                .unwrap()
                .cancellation_requested
        );
        let missing = CString::new("missing-batch").unwrap();
        let pointer =
            qadren_scenario_create_batch_request_cancellation(path.as_ptr(), missing.as_ptr());
        let response: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(pointer) };
        assert_eq!(response["error"], "not_found");
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn same_provider_connections_persist_independently_through_ffi() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file =
            std::env::temp_dir().join(format!("qadren-ffi-{}-{nanos}.sqlite3", std::process::id()));
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let adapter = CString::new("yandex_disk").unwrap();
        let name = CString::new("Personal").unwrap();
        let account = CString::new("shared-account").unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let result = qadren_registry_add(path.as_ptr(), adapter.as_ptr(), name.as_ptr());
            let json: Value =
                serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
            unsafe { qadren_registry_free_string(result) };
            ids.push(json["data"]["id"].as_str().unwrap().to_owned());
        }
        assert_ne!(ids[0], ids[1]);
        let a = CString::new(ids[0].as_str()).unwrap();
        let b = CString::new(ids[1].as_str()).unwrap();
        for id in [&a, &b] {
            let result =
                qadren_registry_set_provider_account(path.as_ptr(), id.as_ptr(), account.as_ptr());
            unsafe { qadren_registry_free_string(result) };
        }
        let result = qadren_registry_remove(path.as_ptr(), a.as_ptr());
        let rejection: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(result) };
        assert_eq!(
            rejection,
            json!({"ok":false,"error":"registry_remove_requires_connection_workflow"})
        );
        let jobs_path = file.with_file_name("jobs.sqlite3");
        JobStore::open(&jobs_path).unwrap();
        let jobs = CString::new(jobs_path.to_str().unwrap()).unwrap();
        let enabled_response = unsafe {
            qadren_registry_remove_after_job_gate(path.as_ptr(), jobs.as_ptr(), a.as_ptr())
        };
        let enabled: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(enabled_response) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        unsafe { qadren_registry_free_string(enabled_response) };
        assert_eq!(
            enabled,
            json!({"ok":false,"error":"connection_not_disabled"})
        );
        let registry = Registry::open(&file).unwrap();
        registry.set_enabled(&ids[0], false).unwrap();
        drop(registry);
        let removed_pointer = unsafe {
            qadren_registry_remove_after_job_gate(path.as_ptr(), jobs.as_ptr(), a.as_ptr())
        };
        let removed: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(removed_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(removed_pointer) };
        assert_eq!(removed, json!({"ok":true,"data":true}));
        let result = qadren_registry_list(path.as_ptr());
        let entries: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(result) };
        let entries = entries["data"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let listed_ids = entries
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(listed_ids.contains(&ids[1].as_str()));
        assert!(entries
            .iter()
            .all(|entry| entry["provider_account_id"] == "shared-account"));
        std::fs::remove_file(file).unwrap();
        std::fs::remove_file(jobs_path).unwrap();
    }

    #[test]
    fn disconnect_finalization_round_trips_through_ffi() {
        let file = std::env::temp_dir().join(format!(
            "qadren-disconnect-ffi-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let database = CString::new(file.to_str().unwrap()).unwrap();
        let adapter = CString::new("yandex_disk").unwrap();
        let name = CString::new("Personal").unwrap();
        let added_pointer = qadren_registry_add(database.as_ptr(), adapter.as_ptr(), name.as_ptr());
        let added: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(added_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(added_pointer) };
        let id = CString::new(added["data"]["id"].as_str().unwrap()).unwrap();
        let disabled = qadren_registry_set_enabled(database.as_ptr(), id.as_ptr(), 0);
        unsafe { qadren_registry_free_string(disabled) };
        let result_pointer = qadren_registry_finalize_disconnect(database.as_ptr(), id.as_ptr());
        let result: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result_pointer) }.to_str().unwrap())
                .unwrap();
        assert_eq!(result["data"], true);
        unsafe { qadren_registry_free_string(result_pointer) };
        let listed_pointer = qadren_registry_list(database.as_ptr());
        let listed: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(listed_pointer) }.to_str().unwrap())
                .unwrap();
        assert_eq!(listed["data"][0]["enabled"], true);
        assert_eq!(listed["data"][0]["status"], "disconnected");
        assert!(listed["data"][0]["credential_ref"].is_null());
        unsafe { qadren_registry_free_string(listed_pointer) };
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn scenario_configuration_round_trips_through_ffi() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = std::env::temp_dir().join(format!(
            "qadren-scenario-ffi-{}-{nanos}.sqlite3",
            std::process::id()
        ));
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let adapter = CString::new("yandex_disk").unwrap();
        let left_name = CString::new("Source").unwrap();
        let right_name = CString::new("Destination").unwrap();
        let add_left = qadren_registry_add(path.as_ptr(), adapter.as_ptr(), left_name.as_ptr());
        let left: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(add_left) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(add_left) };
        // D-023: Yandex Disk is only the source of a one-way Scenario.
        let local = CString::new("local_filesystem").unwrap();
        let add_right = qadren_registry_add(path.as_ptr(), local.as_ptr(), right_name.as_ptr());
        let right: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(add_right) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(add_right) };
        let left_id = CString::new(left["data"]["id"].as_str().unwrap()).unwrap();
        let right_id = CString::new(right["data"]["id"].as_str().unwrap()).unwrap();
        let scenario_name = CString::new("Documents").unwrap();
        // A Yandex Disk root starts with `disk:/`.
        let root = CString::new("disk:/").unwrap();
        let destination_root = CString::new("/archive").unwrap();
        let direction = CString::new("left_to_right").unwrap();
        let created = qadren_scenarios_create(
            path.as_ptr(),
            scenario_name.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            right_id.as_ptr(),
            destination_root.as_ptr(),
            direction.as_ptr(),
        );
        let scenario: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(created) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(created) };
        let scenario_id = CString::new(scenario["data"]["id"].as_str().unwrap()).unwrap();
        let initial_automation_pointer =
            qadren_scenario_automation_get(path.as_ptr(), scenario_id.as_ptr());
        let initial_automation: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(initial_automation_pointer) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(initial_automation["data"]["locally_paused"], false);
        assert_eq!(initial_automation["data"]["pending_automatic_run"], false);
        unsafe { qadren_registry_free_string(initial_automation_pointer) };
        let timezone = CString::new("Europe/Samara").unwrap();
        let created_schedule = qadren_scenario_schedules_create(
            path.as_ptr(),
            scenario_id.as_ptr(),
            timezone.as_ptr(),
            0,
            9,
            30,
        );
        let schedule: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(created_schedule) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        unsafe { qadren_registry_free_string(created_schedule) };
        let schedule_id = CString::new(schedule["data"]["id"].as_str().unwrap()).unwrap();
        let listed_schedules = qadren_scenario_schedules_list(path.as_ptr(), scenario_id.as_ptr());
        let schedules: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(listed_schedules) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        unsafe { qadren_registry_free_string(listed_schedules) };
        assert_eq!(schedules["data"][0]["timezone"], "Europe/Samara");
        let after = CString::new("2026-10-12T04:00:00Z").unwrap();
        let through = CString::new("2026-10-12T06:00:00Z").unwrap();
        let due_pointer =
            qadren_scenario_schedules_due(path.as_ptr(), after.as_ptr(), through.as_ptr());
        let due: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(due_pointer) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(due_pointer) };
        assert_eq!(due["data"].as_array().unwrap().len(), 1);
        assert_eq!(due["data"][0]["scenario_id"], scenario_id.to_str().unwrap());
        assert_eq!(due["data"][0]["schedule_id"], schedule_id.to_str().unwrap());
        let paused_pointer =
            qadren_scenario_automation_set_paused(path.as_ptr(), scenario_id.as_ptr(), 1);
        let paused: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(paused_pointer) }.to_str().unwrap())
                .unwrap();
        assert_eq!(paused["data"], true);
        unsafe { qadren_registry_free_string(paused_pointer) };
        let paused_due_pointer =
            qadren_scenario_schedules_due(path.as_ptr(), after.as_ptr(), through.as_ptr());
        let paused_due: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(paused_due_pointer) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert!(paused_due["data"].as_array().unwrap().is_empty());
        unsafe { qadren_registry_free_string(paused_due_pointer) };
        let disabled_schedule = qadren_scenario_schedules_set_enabled(
            path.as_ptr(),
            scenario_id.as_ptr(),
            schedule_id.as_ptr(),
            0,
        );
        unsafe { qadren_registry_free_string(disabled_schedule) };
        let removed_schedule = qadren_scenario_schedules_remove(
            path.as_ptr(),
            scenario_id.as_ptr(),
            schedule_id.as_ptr(),
        );
        let removed: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(removed_schedule) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(removed["data"], true);
        unsafe { qadren_registry_free_string(removed_schedule) };
        let disabled = qadren_scenarios_set_enabled(path.as_ptr(), scenario_id.as_ptr(), 0);
        unsafe { qadren_registry_free_string(disabled) };
        let updated_name = CString::new("Updated documents").unwrap();
        // The sides swap; Yandex Disk stays the source.
        let reversed = CString::new("right_to_left").unwrap();
        let update_pointer = qadren_scenarios_update(
            path.as_ptr(),
            scenario_id.as_ptr(),
            updated_name.as_ptr(),
            right_id.as_ptr(),
            destination_root.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            reversed.as_ptr(),
        );
        let update: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(update_pointer) }.to_str().unwrap())
                .unwrap();
        assert_eq!(update["data"], true);
        unsafe { qadren_registry_free_string(update_pointer) };
        let listed = qadren_scenarios_list(path.as_ptr());
        let scenarios: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(listed) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(listed) };
        assert_eq!(scenarios["data"][0]["name"], "Updated documents");
        assert_eq!(scenarios["data"][0]["revision"], 2);
        assert_eq!(scenarios["data"][0]["enabled"], false);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn scenario_dry_run_is_revision_bound_and_read_only_through_ffi() {
        let file = std::env::temp_dir().join(format!(
            "qadren-dry-run-ffi-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let database = CString::new(file.to_str().unwrap()).unwrap();
        let adapter = CString::new("local_filesystem").unwrap();
        let left_name = CString::new("Left").unwrap();
        let right_name = CString::new("Right").unwrap();
        let left_pointer =
            qadren_registry_add(database.as_ptr(), adapter.as_ptr(), left_name.as_ptr());
        let left: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(left_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(left_pointer) };
        let right_pointer =
            qadren_registry_add(database.as_ptr(), adapter.as_ptr(), right_name.as_ptr());
        let right: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(right_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(right_pointer) };
        let left_id = CString::new(left["data"]["id"].as_str().unwrap()).unwrap();
        let right_id = CString::new(right["data"]["id"].as_str().unwrap()).unwrap();
        let name = CString::new("Dry run").unwrap();
        let root = CString::new("/left").unwrap();
        let right_root = CString::new("/right").unwrap();
        let direction = CString::new("left_to_right").unwrap();
        let created = qadren_scenarios_create(
            database.as_ptr(),
            name.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            right_id.as_ptr(),
            right_root.as_ptr(),
            direction.as_ptr(),
        );
        let scenario: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(created) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(created) };
        let scenario_id = CString::new(scenario["data"]["id"].as_str().unwrap()).unwrap();
        let left_snapshot = CString::new(
            r#"[{"relative_path":"new.txt","object_revision":"left-v1","size":3,"checksum_md5":null}]"#,
        ).unwrap();
        let right_snapshot = CString::new("[]").unwrap();
        let plan_pointer = qadren_scenarios_dry_run(
            database.as_ptr(),
            scenario_id.as_ptr(),
            1,
            left_snapshot.as_ptr(),
            right_snapshot.as_ptr(),
        );
        let plan: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(plan_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(plan_pointer) };
        assert_eq!(plan["data"][0]["relative_path"], "new.txt");
        assert_eq!(plan["data"][0]["action"]["kind"], "create");
        assert_eq!(plan["data"][0]["action"]["from_left"], true);

        let listed_pointer = qadren_scenarios_list(database.as_ptr());
        let listed: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(listed_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(listed_pointer) };
        assert_eq!(listed["data"][0]["revision"], 1);

        let malformed = CString::new("not-json").unwrap();
        let invalid_pointer = qadren_scenarios_dry_run(
            database.as_ptr(),
            scenario_id.as_ptr(),
            1,
            malformed.as_ptr(),
            right_snapshot.as_ptr(),
        );
        let invalid: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(invalid_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(invalid_pointer) };
        assert_eq!(invalid["error"], "invalid_input");

        let oversized = CString::new(" ".repeat(MAX_DRY_RUN_SNAPSHOT_BYTES + 1)).unwrap();
        let oversized_pointer = qadren_scenarios_dry_run(
            database.as_ptr(),
            scenario_id.as_ptr(),
            1,
            oversized.as_ptr(),
            right_snapshot.as_ptr(),
        );
        let oversized: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(oversized_pointer) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        unsafe { qadren_registry_free_string(oversized_pointer) };
        assert_eq!(oversized["error"], "invalid_input");
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn conflict_queue_requires_a_source_before_a_decision_through_ffi() {
        let file = std::env::temp_dir().join(format!(
            "qadren-conflict-ffi-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = Registry::open(&file).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Documents",
                &left,
                "/left",
                &right,
                "/right",
                "bidirectional",
            )
            .unwrap();
        drop(registry);
        let database = CString::new(file.to_str().unwrap()).unwrap();
        let scenario_id = CString::new(scenario_id).unwrap();
        let relative_path = CString::new("report.pdf").unwrap();
        let content = CString::new("content").unwrap();
        let left_snapshot = CString::new(r#"{"relative_path":"report.pdf","object_revision":"left-v1","size":3,"checksum_md5":null}"#).unwrap();
        let right_snapshot = CString::new(r#"{"relative_path":"report.pdf","object_revision":"right-v1","size":3,"checksum_md5":null}"#).unwrap();
        let queued_pointer = qadren_scenario_conflicts_queue(
            database.as_ptr(),
            scenario_id.as_ptr(),
            1,
            relative_path.as_ptr(),
            content.as_ptr(),
            -1,
            left_snapshot.as_ptr(),
            right_snapshot.as_ptr(),
        );
        let queued: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(queued_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(queued_pointer) };
        let task_id = CString::new(queued["data"]["id"].as_str().unwrap()).unwrap();

        let listed_pointer =
            qadren_scenario_conflicts_list(database.as_ptr(), scenario_id.as_ptr(), 1);
        let listed: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(listed_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(listed_pointer) };
        assert_eq!(listed["data"][0]["source_is_left"], Value::Null);
        assert_eq!(listed["data"][0]["state"], "pending");

        let replace = CString::new("replace_from_source").unwrap();
        let invalid_pointer = qadren_scenario_conflicts_select(
            database.as_ptr(),
            task_id.as_ptr(),
            -1,
            replace.as_ptr(),
        );
        let invalid: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(invalid_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(invalid_pointer) };
        assert_eq!(invalid["error"], "invalid_input");

        let selected_pointer = qadren_scenario_conflicts_select(
            database.as_ptr(),
            task_id.as_ptr(),
            0,
            replace.as_ptr(),
        );
        let selected: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(selected_pointer) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        unsafe { qadren_registry_free_string(selected_pointer) };
        assert_eq!(selected["data"], true);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn schedule_dispatch_stages_and_queues_a_pinned_job_through_ffi() {
        let base = std::env::temp_dir().join(format!(
            "qadren-schedule-dispatch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let registry_path = base.join("registry.sqlite3");
        let job_path = base.join("jobs.sqlite3");
        let registry = Registry::open(&registry_path).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        registry
            .create_scenario_schedule(&scenario, "UTC", 0, 9, 30)
            .unwrap();
        drop(registry);

        let registry_path = CString::new(registry_path.to_str().unwrap()).unwrap();
        let job_path = CString::new(job_path.to_str().unwrap()).unwrap();
        let after = CString::new("2026-10-12T09:29:00Z").unwrap();
        let through = CString::new("2026-10-12T09:30:00Z").unwrap();
        let first_pointer = qadren_scenario_schedules_dispatch(
            registry_path.as_ptr(),
            job_path.as_ptr(),
            after.as_ptr(),
            through.as_ptr(),
        );
        let first: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(first_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(first_pointer) };
        assert_eq!(first["data"]["staged"], 1);
        assert_eq!(first["data"]["dispatched"], 1);
        let second_pointer = qadren_scenario_schedules_dispatch(
            registry_path.as_ptr(),
            job_path.as_ptr(),
            after.as_ptr(),
            through.as_ptr(),
        );
        let second: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(second_pointer) }.to_str().unwrap())
                .unwrap();
        unsafe { qadren_registry_free_string(second_pointer) };
        assert_eq!(second["data"]["staged"], 0);
        assert_eq!(second["data"]["dispatched"], 0);
        let jobs = JobStore::open(Path::new(job_path.to_str().unwrap())).unwrap();
        let jobs = jobs.list_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].scenario_id.as_deref(), Some(scenario.as_str()));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn removal_gate_reports_a_connection_referenced_by_a_scenario() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "qadren-ffi-removal-reference-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let registry_path = base.join("connections.sqlite3");
        let jobs_path = base.join("jobs.sqlite3");
        let registry = Registry::open(&registry_path).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        registry
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        registry.set_enabled(&left, false).unwrap();
        drop(registry);
        JobStore::open(&jobs_path).unwrap();

        let registry_path = CString::new(registry_path.to_str().unwrap()).unwrap();
        let jobs_path = CString::new(jobs_path.to_str().unwrap()).unwrap();
        let connection = CString::new(left).unwrap();
        let response = unsafe {
            qadren_registry_remove_after_job_gate(
                registry_path.as_ptr(),
                jobs_path.as_ptr(),
                connection.as_ptr(),
            )
        };
        let result: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(response) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(response) };
        assert_eq!(
            result,
            json!({"ok":false,"error":"connection_referenced_by_scenario"})
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_ffi_inputs_have_stable_json_errors() {
        let add = qadren_registry_add(std::ptr::null(), std::ptr::null(), std::ptr::null());
        let json: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(add) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(add) };
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "invalid_input");

        let list = qadren_registry_list(std::ptr::null());
        let json: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(list) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(list) };
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "invalid_path");

        let due =
            qadren_scenario_schedules_due(std::ptr::null(), std::ptr::null(), std::ptr::null());
        let json: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(due) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(due) };
        assert_eq!(json["error"], "invalid_input");

        unsafe { qadren_registry_free_string(std::ptr::null_mut()) };
    }

    #[test]
    fn rejects_missing_path_without_dereferencing_null_pointer() {
        let ptr = qadren_registry_list(std::ptr::null());
        let data = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { qadren_registry_free_string(ptr) };
        assert!(data.contains("invalid_path"));
    }
}
