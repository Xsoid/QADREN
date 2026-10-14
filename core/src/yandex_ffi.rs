//! C ABI for metadata-only Yandex Disk requests. Call off the Swift main thread.
use crate::yandex::{YandexError, YandexMetadataClient};
use serde_json::{json, Value};
use std::ffi::{c_char, CStr, CString};

fn safe_error(error: YandexError) -> &'static str {
    match error {
        YandexError::Unauthorized => "unauthorized",
        YandexError::Forbidden => "forbidden",
        YandexError::RateLimited => "rate_limited",
        YandexError::NotFound => "not_found",
        YandexError::InvalidToken => "invalid_token",
        YandexError::InvalidPath => "invalid_path",
        YandexError::InvalidAccountIdentity => "invalid_account_identity",
        YandexError::UnsafeTransferUrl => "unsafe_transfer_url",
        YandexError::UploadRejected(_) => "upload_rejected",
        YandexError::UploadOutcomeUnknown => "upload_outcome_unknown",
        YandexError::Transport(_) => "network_error",
        YandexError::Provider(_) => "provider_error",
    }
}

fn export(value: Value) -> *mut c_char {
    let bytes = value.to_string();
    CString::new(bytes)
        .expect("JSON cannot contain NUL")
        .into_raw()
}

fn parse_token(token: *const c_char) -> Option<String> {
    if token.is_null() {
        return None;
    }
    let token = unsafe { CStr::from_ptr(token) };
    token.to_str().ok().map(ToOwned::to_owned)
}

#[no_mangle]
pub extern "C" fn qadren_yandex_disk_info(token: *const c_char) -> *mut c_char {
    let Some(token) = parse_token(token) else {
        return export(json!({"ok": false, "error": "invalid_token"}));
    };
    let result = YandexMetadataClient::new()
        .map_err(safe_error)
        .and_then(|client| client.disk_info(&token).map_err(safe_error));
    match result {
        Ok(info) => export(json!({"ok": true, "data": info})),
        Err(error) => export(json!({"ok": false, "error": error})),
    }
}

#[no_mangle]
pub extern "C" fn qadren_yandex_list_root(token: *const c_char) -> *mut c_char {
    let Some(token) = parse_token(token) else {
        return export(json!({"ok": false, "error": "invalid_token"}));
    };
    let result = YandexMetadataClient::new()
        .map_err(safe_error)
        .and_then(|client| {
            client
                .list_folder(&token, "disk:/", 0, 50)
                .map_err(safe_error)
        });
    match result {
        Ok(folder) => export(json!({"ok": true, "data": folder})),
        Err(error) => export(json!({"ok": false, "error": error})),
    }
}

/// Lists one Yandex Disk directory's metadata. This never requests file bytes.
#[no_mangle]
pub extern "C" fn qadren_yandex_list_folder(
    input: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    let (Some(input_value), Some(path)) = (parse_token(input), parse_token(path)) else {
        return export(json!({"ok": false, "error": "invalid_request"}));
    };
    let result = YandexMetadataClient::new()
        .map_err(safe_error)
        .and_then(|client| {
            client
                .list_folder(&input_value, &path, 0, 50)
                .map_err(safe_error)
        });
    match result {
        Ok(folder) => export(json!({"ok": true, "data": folder})),
        Err(error) => export(json!({"ok": false, "error": error})),
    }
}

/// Lists one bounded metadata page from a Yandex Disk directory. This never
/// requests file bytes and does not persist the access token or listing.
#[no_mangle]
pub extern "C" fn qadren_yandex_list_folder_page(
    input: *const c_char,
    path: *const c_char,
    offset: u64,
) -> *mut c_char {
    let (Some(input_value), Some(path)) = (parse_token(input), parse_token(path)) else {
        return export(json!({"ok": false, "error": "invalid_request"}));
    };
    let result = YandexMetadataClient::new()
        .map_err(safe_error)
        .and_then(|client| {
            client
                .list_folder(&input_value, &path, offset, 50)
                .map_err(safe_error)
        });
    match result {
        Ok(folder) => export(json!({"ok": true, "data": folder})),
        Err(error) => export(json!({"ok": false, "error": error})),
    }
}

/// # Safety
/// Pointer must have been returned by this module and not previously freed.
#[no_mangle]
pub unsafe extern "C" fn qadren_yandex_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_and_free_response(ptr: *mut c_char) -> String {
        let response = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { qadren_yandex_free_string(ptr) };
        response
    }

    #[test]
    fn missing_token_returns_non_secret_error() {
        let ptr = qadren_yandex_disk_info(std::ptr::null());
        assert!(decode_and_free_response(ptr).contains("invalid_token"));
    }

    #[test]
    fn folder_listing_rejects_missing_inputs_without_network_access() {
        let ptr = qadren_yandex_list_folder(std::ptr::null(), std::ptr::null());
        assert!(decode_and_free_response(ptr).contains("invalid_request"));
    }

    #[test]
    fn folder_page_rejects_missing_inputs_without_network_access() {
        let ptr = qadren_yandex_list_folder_page(std::ptr::null(), std::ptr::null(), 50);
        assert!(decode_and_free_response(ptr).contains("invalid_request"));
    }
}
