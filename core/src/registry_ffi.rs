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
pub extern "C" fn qadren_registry_set_credential(
    path: *const c_char,
    id: *const c_char,
    credential_ref: *const c_char,
) -> *mut c_char {
    let (Some(id), Some(reference)) = (input(id), input(credential_ref)) else {
        return output(json!({"ok":false,"error":"invalid_input"}));
    };
    with_registry(path, |db| {
        db.set_credential_ref(&id, Some(&reference))?;
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
    fn rejects_missing_path_without_dereferencing_null_pointer() {
        let ptr = qadren_registry_list(std::ptr::null());
        let data = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { qadren_registry_free_string(ptr) };
        assert!(data.contains("invalid_path"));
    }
}
