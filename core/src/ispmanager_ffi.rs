//! C ABI for the ISPmanager connection check (D-027). Call off the Swift
//! main thread: the check makes network requests.
use crate::{ispmanager, ispmanager_sftp};
use serde_json::json;
use std::ffi::{c_char, CStr, CString};

fn input(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller passes a valid NUL-terminated string.
    let value = unsafe { CStr::from_ptr(pointer) };
    value.to_str().ok().map(ToOwned::to_owned)
}

/// Signs in with the login and the password, which stay in memory only, and
/// returns the capability report. `ssh_port` and `host_key` (empty when not
/// confirmed) set the SFTP check. Nothing on the server changes.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_check(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
    ssh_port: u16,
    host_key: *const c_char,
) -> *mut c_char {
    let host_key = input(host_key).filter(|key| !key.is_empty());
    let value = match (input(url), input(login), input(password)) {
        (Some(url), Some(login), Some(password)) if ssh_port > 0 => {
            let sftp = ispmanager::SftpTarget {
                port: ssh_port,
                host_key_sha256: host_key.as_deref(),
            };
            json!({"ok": true, "data": ispmanager::check(&url, &login, &password, sftp)})
        }
        _ => json!({"ok": false, "error": "invalid_input"}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL")
        .into_raw()
}

/// Lists one folder through SFTP (D-027 section 7a). `path` is absolute, or
/// empty for the home folder. `host_key` must be the confirmed fingerprint.
/// A failure returns the SFTP reason in the same form as the check report.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_sftp_list(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
    ssh_port: u16,
    host_key: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    let value = match (
        input(url).and_then(|url| ispmanager::PanelUrl::parse(url.trim()).ok()),
        input(login),
        input(password),
        input(host_key).filter(|key| !key.is_empty()),
        input(path),
    ) {
        (Some(url), Some(login), Some(password), Some(host_key), Some(path)) if ssh_port > 0 => {
            let path = (!path.is_empty()).then_some(path.as_str());
            match ispmanager_sftp::list_dir(
                url.host(),
                ssh_port,
                &login,
                &password,
                &host_key,
                path,
            ) {
                Ok(listing) => json!({"ok": true, "data": listing}),
                Err(error) => {
                    json!({"ok": false, "error": "sftp", "transport": ispmanager::FileTransport::from(error)})
                }
            }
        }
        _ => json!({"ok": false, "error": "invalid_input"}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL")
        .into_raw()
}

/// The MySQL database names of the account, for the Scenario editor
/// (D-027 section 7b). It reads the `db` list only.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_databases(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    let value = match (
        input(url).and_then(|url| ispmanager::PanelUrl::parse(url.trim()).ok()),
        input(login),
        input(password),
    ) {
        (Some(url), Some(login), Some(password)) => {
            match ispmanager::Session::new(url, login, password)
                .and_then(|mut session| ispmanager::database_names(&mut session))
            {
                Ok(names) => json!({"ok": true, "data": names}),
                Err(error) => json!({"ok": false, "error": error.to_string()}),
            }
        }
        _ => json!({"ok": false, "error": "invalid_input"}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL")
        .into_raw()
}

/// The base tables of one MySQL database with their estimated sizes, for
/// the table rules of the Scenario editor (D-027 section 7b.10). The access
/// facts come from the panel and stay in memory. The query reads
/// `information_schema` only.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_database_tables(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
    ssh_port: u16,
    host_key: *const c_char,
    database: *const c_char,
) -> *mut c_char {
    let value = match (
        input(url).and_then(|url| ispmanager::PanelUrl::parse(url.trim()).ok()),
        input(login),
        input(password),
        input(host_key).filter(|key| !key.is_empty()),
        input(database),
    ) {
        (Some(url), Some(login), Some(password), Some(host_key), Some(database))
            if ssh_port > 0 =>
        {
            let host = url.host().to_owned();
            let access = ispmanager::Session::new(url, login.clone(), password.clone())
                .map_err(|_| "ispmanager_unavailable")
                .and_then(|mut session| ispmanager::mysql_access(&mut session, &database));
            match access {
                Err(error) => json!({"ok": false, "error": error}),
                Ok(access) => match ispmanager_sftp::SftpConnection::open(
                    &host, ssh_port, &login, &password, &host_key,
                ) {
                    Err(error) => json!({
                        "ok": false,
                        "error": "sftp",
                        "transport": ispmanager::FileTransport::from(error)
                    }),
                    Ok(connection) => match connection.mysql_tables(&access) {
                        Ok(tables) => json!({"ok": true, "data": tables}),
                        Err(ispmanager_sftp::MysqlDumpError::Refused) => {
                            json!({"ok": false, "error": "ispmanager_database_refused"})
                        }
                        Err(_) => json!({"ok": false, "error": "ispmanager_database_tables_failed"}),
                    },
                },
            }
        }
        _ => json!({"ok": false, "error": "invalid_input"}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL")
        .into_raw()
}

/// # Safety
/// `pointer` must come from `qadren_ispmanager_check` and be freed one time.
#[no_mangle]
pub unsafe extern "C" fn qadren_ispmanager_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(CString::from_raw(pointer));
    }
}
