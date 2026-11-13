//! C ABI for the ISPmanager connection check (D-027). Call off the Swift
//! main thread: the check makes network requests.
use crate::{ispmanager, ispmanager_sftp};
use serde_json::json;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};

fn input(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller passes a valid NUL-terminated string.
    let value = unsafe { CStr::from_ptr(pointer) };
    value.to_str().ok().map(ToOwned::to_owned)
}

/// The passwords that the user entered for database users whose password
/// the panel hides: a JSON object from MySQL user name to password. A null
/// or invalid value is an empty map.
fn manual_passwords(pointer: *const c_char) -> HashMap<String, String> {
    input(pointer)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn access_failure(error: &ispmanager::DatabaseAccessError) -> serde_json::Value {
    json!({"ok": false, "error": error.code, "user": error.user})
}

/// The MySQL user of one database and where its password comes from, for
/// the Scenario editor. Panel requests only: no MySQL sign-in. The password
/// is never returned.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_database_access(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
    database: *const c_char,
    passwords: *const c_char,
) -> *mut c_char {
    let manual = manual_passwords(passwords);
    let value = match (
        input(url).and_then(|url| ispmanager::PanelUrl::parse(url.trim()).ok()),
        input(login),
        input(password),
        input(database),
    ) {
        (Some(url), Some(login), Some(password), Some(database)) => {
            match ispmanager::Session::new(url, login, password)
                .map_err(|_| ispmanager::DatabaseAccessError::from("ispmanager_unavailable"))
                .and_then(|mut session| {
                    ispmanager::mysql_access(&mut session, &database, &|user| {
                        manual.get(user).cloned()
                    })
                }) {
                Ok((access, source)) => {
                    json!({"ok": true, "data": {"user": access.user, "source": source}})
                }
                Err(error) => access_failure(&error),
            }
        }
        _ => json!({"ok": false, "error": "invalid_input"}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL")
        .into_raw()
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

/// The DNS zones (`category` `dns`) or mail domains (`mail`) of the
/// account, for the Scenario editor (D-027 section 10). It reads one panel
/// list only.
#[no_mangle]
pub extern "C" fn qadren_ispmanager_object_names(
    url: *const c_char,
    login: *const c_char,
    password: *const c_char,
    category: *const c_char,
) -> *mut c_char {
    let value = match (
        input(url).and_then(|url| ispmanager::PanelUrl::parse(url.trim()).ok()),
        input(login),
        input(password),
        input(category),
    ) {
        (Some(url), Some(login), Some(password), Some(category)) => {
            match ispmanager::Session::new(url, login, password)
                .and_then(|mut session| ispmanager::object_names(&mut session, &category))
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

/// The sites of the account with their root folders and databases, for the
/// site options of the Scenario editor (D-027 section 7c).
#[no_mangle]
pub extern "C" fn qadren_ispmanager_sites(
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
                .and_then(|mut session| ispmanager::sites_overview(&mut session))
            {
                Ok(sites) => json!({"ok": true, "data": sites}),
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
    passwords: *const c_char,
) -> *mut c_char {
    let manual = manual_passwords(passwords);
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
                .map_err(|_| ispmanager::DatabaseAccessError::from("ispmanager_unavailable"))
                .and_then(|mut session| {
                    ispmanager::mysql_access(&mut session, &database, &|user| {
                        manual.get(user).cloned()
                    })
                });
            match access {
                Err(error) => access_failure(&error),
                Ok((access, source)) => match ispmanager_sftp::SftpConnection::open(
                    &host, ssh_port, &login, &password, &host_key,
                ) {
                    Err(error) => json!({
                        "ok": false,
                        "error": "sftp",
                        "transport": ispmanager::FileTransport::from(error)
                    }),
                    Ok(connection) => match connection.mysql_tables(&access) {
                        Ok(tables) => json!({"ok": true, "data": tables}),
                        Err(ispmanager_sftp::MysqlDumpError::Refused) => json!({
                            "ok": false,
                            "error": match source {
                                ispmanager::PasswordSource::Manual => "ispmanager_database_manual_refused",
                                ispmanager::PasswordSource::Panel => "ispmanager_database_refused",
                            },
                            "user": access.user,
                        }),
                        Err(_) => {
                            json!({"ok": false, "error": "ispmanager_database_tables_failed"})
                        }
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
