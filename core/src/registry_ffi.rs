//! Minimal C ABI for the SQLite connection registry.
//! Inputs are UTF-8 owned by the caller; JSON output is caller-freed.
//! No OAuth tokens or file contents are persisted by these methods.
use crate::registry::Registry;
use serde_json::{json, Value};
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

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
