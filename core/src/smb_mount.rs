//! The Worker mounts an SMB share on demand (D-025 section 7, issue #32).
//!
//! The mount uses the NetFS framework with no user interface, at the QADREN
//! mount directory of the connection (D-022 rule 3), as the app does. The
//! login and the password go to NetFS in memory only: never in argv, a log,
//! or a URL. The Core mount gate (`storage_access::is_smb_mount`) stays the
//! last check before each effect.

use std::path::Path;

/// The result of one mount attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOutcome {
    Mounted,
    /// macOS has this share mounted at another place, for example by Finder
    /// or Time Machine. macOS mounts one share only once. QADREN never
    /// unmounts a mount that it did not make.
    MountedElsewhere(String),
    /// EAUTH or EACCES: the server refused the login.
    LoginRefused,
    /// No network or no server. A later attempt can work.
    Offline,
    /// The share does not exist, or the address is not a valid SMB address.
    NotFound,
    Failed(i32),
}

/// The percent-encoded `smb://host[:port]/share[/folder]` URL for NetFS, or
/// `None` for an address with a login, a domain, a query, or no share.
pub fn mount_url(address: &str) -> Option<String> {
    let trimmed = address.trim();
    let rest = trimmed
        .get(..6)
        .filter(|scheme| scheme.eq_ignore_ascii_case("smb://"))
        .map(|_| &trimmed[6..])?;
    if rest.contains(['@', ';', '?', '#', '\\']) {
        return None;
    }
    let (authority, path) = rest.split_once('/')?;
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if authority.is_empty()
        || segments.is_empty()
        || segments.iter().any(|part| *part == "." || *part == "..")
    {
        return None;
    }
    let encoded: Vec<String> = segments.iter().map(|part| encode(part, "")).collect();
    Some(format!(
        "smb://{}/{}",
        encode(authority, ":[]"),
        encoded.join("/")
    ))
}

/// Percent-encode everything except unreserved characters and `keep`.
fn encode(value: &str, keep: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || b"-._~".contains(&byte)
            || keep.as_bytes().contains(&byte)
        {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let digits = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(digits, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Host and share of an address or of an `smbfs` mount source
/// (`//user@host/share`), lowercased, without a trailing dot on the host.
fn host_and_share(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("//").or_else(|| {
        value
            .get(6..)
            .filter(|_| value[..6].eq_ignore_ascii_case("smb://"))
    })?;
    let (authority, path) = rest.split_once('/')?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host
        .rsplit_once(':')
        .filter(|(name, port)| !name.contains(':') && port.parse::<u16>().is_ok())
        .map_or(host, |(name, _)| name);
    let share = path.split('/').find(|part| !part.is_empty())?;
    let host = decode(host).to_lowercase();
    Some((
        host.trim_end_matches('.').to_owned(),
        decode(share).to_lowercase(),
    ))
}

/// The mount point where macOS has the share of `address` mounted now,
/// other than `own_dir`.
#[cfg(target_os = "macos")]
pub fn mounted_elsewhere(address: &str, own_dir: &Path) -> Option<String> {
    use std::ffi::CStr;
    let wanted = host_and_share(address)?;
    let own = std::fs::canonicalize(own_dir).ok();
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count <= 0 {
        return None;
    }
    let mut mounts: Vec<libc::statfs> = Vec::with_capacity(count as usize + 4);
    let size = (mounts.capacity() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
    let filled = unsafe { libc::getfsstat(mounts.as_mut_ptr(), size, libc::MNT_NOWAIT) };
    if filled <= 0 {
        return None;
    }
    // SAFETY: getfsstat initialized `filled` entries.
    unsafe { mounts.set_len(filled as usize) };
    mounts.iter().find_map(|mount| {
        let kind = unsafe { CStr::from_ptr(mount.f_fstypename.as_ptr()) };
        if kind.to_bytes() != b"smbfs" {
            return None;
        }
        let from = unsafe { CStr::from_ptr(mount.f_mntfromname.as_ptr()) }.to_string_lossy();
        let on = unsafe { CStr::from_ptr(mount.f_mntonname.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        (host_and_share(&from).as_ref() == Some(&wanted) && own.as_deref() != Some(Path::new(&on)))
            .then_some(on)
    })
}

/// Mount the share of `address` exactly at `dir` with no user interface.
/// The caller checks first that `dir` is not mounted and that the share is
/// not mounted elsewhere.
#[cfg(target_os = "macos")]
pub fn mount(address: &str, dir: &Path, login: &str, password: &str) -> MountOutcome {
    use cf::*;
    use std::os::unix::ffi::OsStrExt;
    let Some(url) = mount_url(address) else {
        return MountOutcome::NotFound;
    };
    // SAFETY: every CoreFoundation object below is created here, checked for
    // null, and released once after the call.
    unsafe {
        let url = CFURLCreateWithBytes(
            std::ptr::null(),
            url.as_ptr(),
            url.len() as isize,
            UTF8,
            std::ptr::null(),
        );
        let dir_bytes = dir.as_os_str().as_bytes();
        let dir = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            dir_bytes.as_ptr(),
            dir_bytes.len() as isize,
            1,
        );
        let user = string(login);
        let secret = string(password);
        let open = dictionary(&[("UIOption", Value::Text("NoUI"))]);
        // kNetFSMountAtMountDirKey: mount exactly at `dir`.
        // kNetFSAllowSubMountsKey: an address with a folder mounts that folder.
        let options = dictionary(&[
            ("MountAtMountDir", Value::True),
            ("AllowSubMounts", Value::True),
        ]);
        let objects = [url, dir, user, secret, open, options];
        let status = if objects.iter().any(|object| object.is_null()) {
            libc::ENOMEM
        } else {
            let mut mountpoints: *const std::ffi::c_void = std::ptr::null();
            let status = NetFSMountURLSync(url, dir, user, secret, open, options, &mut mountpoints);
            if !mountpoints.is_null() {
                CFRelease(mountpoints);
            }
            status
        };
        for object in objects.into_iter().filter(|object| !object.is_null()) {
            CFRelease(object);
        }
        match status {
            0 => MountOutcome::Mounted,
            libc::EEXIST => MountOutcome::MountedElsewhere(String::new()),
            libc::EAUTH | libc::EACCES => MountOutcome::LoginRefused,
            libc::ENOENT => MountOutcome::NotFound,
            libc::EHOSTUNREACH
            | libc::ENETUNREACH
            | libc::ETIMEDOUT
            | libc::ECONNREFUSED
            | libc::EHOSTDOWN
            | libc::ENETDOWN => MountOutcome::Offline,
            other => MountOutcome::Failed(other),
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn mounted_elsewhere(_address: &str, _own_dir: &Path) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn mount(_address: &str, _dir: &Path, _login: &str, _password: &str) -> MountOutcome {
    MountOutcome::Failed(libc::ENOTSUP)
}

/// A Time Machine mount is never touched.
pub fn is_time_machine(mount_point: &str) -> bool {
    mount_point.starts_with("/Volumes/.timemachine/")
}

#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;

    pub type Ref = *const c_void;
    pub const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        static kCFBooleanTrue: Ref;
        fn CFStringCreateWithBytes(
            allocator: Ref,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        pub fn CFURLCreateWithBytes(
            allocator: Ref,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            base: Ref,
        ) -> Ref;
        pub fn CFURLCreateFromFileSystemRepresentation(
            allocator: Ref,
            bytes: *const u8,
            length: isize,
            directory: u8,
        ) -> Ref;
        fn CFDictionaryCreateMutable(
            allocator: Ref,
            capacity: isize,
            keys: *const c_void,
            values: *const c_void,
        ) -> Ref;
        fn CFDictionarySetValue(dictionary: Ref, key: Ref, value: Ref);
        pub fn CFRelease(object: Ref);
    }

    #[link(name = "NetFS", kind = "framework")]
    extern "C" {
        pub fn NetFSMountURLSync(
            url: Ref,
            mount_path: Ref,
            user: Ref,
            password: Ref,
            open_options: Ref,
            mount_options: Ref,
            mountpoints: *mut Ref,
        ) -> i32;
    }

    pub enum Value {
        Text(&'static str),
        True,
    }

    pub unsafe fn string(value: &str) -> Ref {
        CFStringCreateWithBytes(
            std::ptr::null(),
            value.as_ptr(),
            value.len() as isize,
            UTF8,
            0,
        )
    }

    pub unsafe fn dictionary(entries: &[(&str, Value)]) -> Ref {
        let dictionary = CFDictionaryCreateMutable(
            std::ptr::null(),
            0,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        if dictionary.is_null() {
            return dictionary;
        }
        for (key, value) in entries {
            let key = string(key);
            let value = match value {
                Value::Text(text) => string(text),
                Value::True => kCFBooleanTrue,
            };
            if !key.is_null() && !value.is_null() {
                CFDictionarySetValue(dictionary, key, value);
            }
            if !key.is_null() {
                CFRelease(key);
            }
            if !value.is_null() && value != kCFBooleanTrue {
                CFRelease(value);
            }
        }
        dictionary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_url_encodes_names_and_refuses_credentials() {
        assert_eq!(
            mount_url("smb://My NAS._smb._tcp.local/Общая папка").as_deref(),
            Some("smb://My%20NAS._smb._tcp.local/%D0%9E%D0%B1%D1%89%D0%B0%D1%8F%20%D0%BF%D0%B0%D0%BF%D0%BA%D0%B0")
        );
        assert_eq!(
            mount_url("SMB://nas:445/Media/Photos/").as_deref(),
            Some("smb://nas:445/Media/Photos")
        );
        for refused in [
            "smb://user@nas/share",
            "smb://dom;user@nas/share",
            "smb://nas",
            "smb://nas/",
            "smb://nas/../etc",
            "afp://nas/share",
        ] {
            assert_eq!(mount_url(refused), None, "{refused}");
        }
    }

    #[test]
    fn mount_source_matches_the_address_without_login_case_or_port() {
        let address = host_and_share("smb://NAS.local./Media/Photos");
        assert_eq!(address, Some(("nas.local".into(), "media".into())));
        assert_eq!(host_and_share("//alice@nas.local/media"), address);
        assert_eq!(host_and_share("//;WORK;alice@NAS.local:445/Media"), address);
        assert_eq!(
            host_and_share("//alice@My%20NAS._smb._tcp.local/%D0%9E%D0%B1%D1%89%D0%B0%D1%8F"),
            Some(("my nas._smb._tcp.local".into(), "общая".into()))
        );
        assert_ne!(host_and_share("//alice@nas.local/other"), address);
    }

    #[test]
    fn time_machine_mounts_are_recognized() {
        assert!(is_time_machine("/Volumes/.timemachine/nas/backup"));
        assert!(!is_time_machine("/Volumes/Media"));
    }
}
