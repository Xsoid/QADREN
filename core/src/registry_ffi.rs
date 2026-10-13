//! Minimal C ABI for the SQLite connection registry.
//! Inputs are UTF-8 owned by the caller; JSON output is caller-freed.
//! No OAuth tokens or file contents are persisted by these methods.
use crate::model::ScenarioReference;
use crate::registry::{Registry, ScenarioUpdate};
use crate::scenario_planner::{ObservedFile, PlanAction, PlanItem, SideChange};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

const MAX_DRY_RUN_SNAPSHOT_BYTES: usize = 1_048_576;
const MAX_DRY_RUN_SNAPSHOT_ENTRIES: usize = 10_000;

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
        Err(_) => output(json!({"ok":false,"error":"registry_error"})),
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
                    "provider_account_id": entry.provider_account_id
                })
            })
            .collect();
        Ok(json!(entries))
    })
}

#[no_mangle]
pub extern "C" fn qadren_scenarios_list(path: *const c_char) -> *mut c_char {
    with_registry(path, |db| {
        Ok(json!(db
            .list_scenarios()?
            .into_iter()
            .map(|item| json!({
                "id": item.id, "name": item.name, "left_connection_id": item.left_connection_id,
                "left_root": item.left_root, "right_connection_id": item.right_connection_id,
                "right_root": item.right_root, "direction": item.direction, "enabled": item.enabled,
                "revision": item.revision
            }))
            .collect::<Vec<Value>>()))
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
        Ok(
            json!({"id": db.create_scenario(&name, &left_id, &left_root, &right_id, &right_root, &direction)?}),
        )
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
        let left_snapshot = left_snapshot
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let right_snapshot = right_snapshot
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        Ok(json!(db
            .plan_scenario_dry_run(&reference, &left_snapshot, &right_snapshot)?
            .into_iter()
            .map(plan_item_value)
            .collect::<Vec<_>>()))
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

#[no_mangle]
pub extern "C" fn qadren_registry_remove(path: *const c_char, id: *const c_char) -> *mut c_char {
    let Some(id) = input(id) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.remove(&id)?;
        Ok(json!(true))
    })
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
        unsafe { qadren_registry_free_string(result) };
        let result = qadren_registry_list(path.as_ptr());
        let entries: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(result) };
        assert_eq!(entries["data"].as_array().unwrap().len(), 1);
        assert_eq!(entries["data"][0]["id"], ids[1]);
        assert_eq!(entries["data"][0]["provider_account_id"], "shared-account");
        std::fs::remove_file(file).unwrap();
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
        let add_right = qadren_registry_add(path.as_ptr(), adapter.as_ptr(), right_name.as_ptr());
        let right: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(add_right) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(add_right) };
        let left_id = CString::new(left["data"]["id"].as_str().unwrap()).unwrap();
        let right_id = CString::new(right["data"]["id"].as_str().unwrap()).unwrap();
        let scenario_name = CString::new("Documents").unwrap();
        let root = CString::new("/").unwrap();
        let direction = CString::new("left_to_right").unwrap();
        let created = qadren_scenarios_create(
            path.as_ptr(),
            scenario_name.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            right_id.as_ptr(),
            root.as_ptr(),
            direction.as_ptr(),
        );
        let scenario: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(created) }.to_str().unwrap()).unwrap();
        unsafe { qadren_registry_free_string(created) };
        let scenario_id = CString::new(scenario["data"]["id"].as_str().unwrap()).unwrap();
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
        let update_pointer = qadren_scenarios_update(
            path.as_ptr(),
            scenario_id.as_ptr(),
            updated_name.as_ptr(),
            right_id.as_ptr(),
            root.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            direction.as_ptr(),
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
        let root = CString::new("/").unwrap();
        let direction = CString::new("left_to_right").unwrap();
        let created = qadren_scenarios_create(
            database.as_ptr(),
            name.as_ptr(),
            left_id.as_ptr(),
            root.as_ptr(),
            right_id.as_ptr(),
            root.as_ptr(),
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
