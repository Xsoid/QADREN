//! Which connections can reach files through the local filesystem path, and
//! when. Release 1 executes file effects through `LocalFilesystem` only.
//!
//! An SMB share connection (D-021, D-022) reaches its files through one macOS
//! mount that QADREN owns: `<mounts root>/<connection_id>`. macOS refuses a
//! network mount inside `~/Library` (EPERM), so for the default data root in
//! `~/Library/Application Support/QADREN` the mounts root is `~/.qadren/mounts`.
//! For another data root (tests, `QADREN_DATA_ROOT`) it is `<data root>/mounts`. The platform mounts the share there with the Keychain login.
//! A Scenario root of an SMB connection is an absolute path inside that mount
//! directory, so the existing local executor, durable attempts, root
//! identity, and reconciliation apply unchanged.
//!
//! When the share is not mounted, the mount directory is an ordinary local
//! directory. A plan or an effect there would act on the wrong storage, so
//! every gate below refuses an SMB connection unless the directory is the
//! mount point of an `smbfs` filesystem now.
use crate::registry::{Registry, StorageConnection};
use std::path::{Component, Path, PathBuf};

pub const LOCAL_FILESYSTEM: &str = "local_filesystem";
pub const SMB_SHARE: &str = "smb_share";
pub const YANDEX_DISK: &str = "yandex_disk";

/// A Yandex Disk Scenario root: `disk:/` or a path below it, without `.` or
/// `..` segments or control characters (D-023).
pub fn valid_yandex_root(root: &str) -> bool {
    root.starts_with("disk:/")
        && !root.contains(|ch: char| ch.is_control())
        && root["disk:/".len()..]
            .split('/')
            .all(|segment| segment != "." && segment != "..")
}

/// A Yandex Disk connection can be the source of a one-way Scenario only,
/// and its other side must be on this Mac (D-023, stage 1).
pub fn yandex_scope_allowed(left_adapter: &str, right_adapter: &str, direction: &str) -> bool {
    let on_this_mac = |adapter: &str| matches!(adapter, LOCAL_FILESYSTEM | SMB_SHARE);
    match (left_adapter == YANDEX_DISK, right_adapter == YANDEX_DISK) {
        (false, false) => true,
        (true, false) => direction == "left_to_right" && on_this_mac(right_adapter),
        (false, true) => direction == "right_to_left" && on_this_mac(left_adapter),
        (true, true) => false,
    }
}

/// The directory that holds the SMB mounts. It depends only on the Registry
/// path, so the app, the FFI, and the Worker compute the same directory.
pub fn smb_mounts_root(registry_path: &Path) -> PathBuf {
    let data = registry_path.parent().unwrap_or_else(|| Path::new("."));
    if data.ends_with("Library/Application Support/QADREN") {
        if let Some(home) = data.parent().and_then(Path::parent).and_then(Path::parent) {
            return home.join(".qadren").join("mounts");
        }
    }
    data.join("mounts")
}

/// The mount directory of one SMB share connection.
pub fn smb_mount_dir(registry_path: &Path, connection_id: &str) -> PathBuf {
    smb_mounts_root(registry_path).join(connection_id)
}

/// True when `root` is the mount directory or a path inside it, without `..`.
pub fn root_inside(mount_dir: &Path, root: &Path) -> bool {
    root.is_absolute()
        && !root
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        && root.starts_with(mount_dir)
}

/// True when `dir` is the mount point of an `smbfs` filesystem now.
#[cfg(target_os = "macos")]
pub fn is_smb_mount(dir: &Path) -> bool {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    let Ok(canonical) = std::fs::canonicalize(dir) else {
        return false;
    };
    let Ok(path) = CString::new(canonical.as_os_str().as_bytes()) else {
        return false;
    };
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `info` is a valid output buffer.
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: statfs returned 0, so the buffer is initialized.
    let info = unsafe { info.assume_init() };
    // SAFETY: the kernel writes NUL-terminated names into both fields.
    let kind = unsafe { CStr::from_ptr(info.f_fstypename.as_ptr()) };
    let mounted_on = unsafe { CStr::from_ptr(info.f_mntonname.as_ptr()) };
    kind.to_bytes() == b"smbfs" && mounted_on.to_bytes() == canonical.as_os_str().as_bytes()
}

#[cfg(not(target_os = "macos"))]
pub fn is_smb_mount(_dir: &Path) -> bool {
    false
}

/// The connection is enabled and its files are reachable now: a Local
/// Filesystem connection, or an SMB share whose QADREN mount is present.
pub fn connection_ready(connection: &StorageConnection, registry_path: &Path) -> bool {
    connection.enabled
        && match connection.adapter.as_str() {
            LOCAL_FILESYSTEM => true,
            SMB_SHARE => is_smb_mount(&smb_mount_dir(registry_path, &connection.id)),
            _ => false,
        }
}

/// Like `connection_ready`, and the root belongs to the connection: an SMB
/// root must be inside the mount directory of that connection.
pub fn root_ready(connection: &StorageConnection, registry_path: &Path, root: &Path) -> bool {
    connection_ready(connection, registry_path) && root_belongs(connection, registry_path, root)
}

/// A Scenario root can belong to the connection. A Local Filesystem root is
/// any absolute path; an SMB root is inside the mount directory; a Yandex
/// Disk root is a `disk:/` path.
pub fn root_belongs(connection: &StorageConnection, registry_path: &Path, root: &Path) -> bool {
    match connection.adapter.as_str() {
        LOCAL_FILESYSTEM => true,
        SMB_SHARE => root_inside(&smb_mount_dir(registry_path, &connection.id), root),
        YANDEX_DISK => root.to_str().is_some_and(valid_yandex_root),
        _ => false,
    }
}

/// Look up a connection and apply `connection_ready`.
pub fn connection_id_ready(registry: &Registry, registry_path: &Path, connection_id: &str) -> bool {
    matches!(
        registry.get(connection_id),
        Ok(Some(connection)) if connection_ready(&connection, registry_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smb_root_must_stay_inside_its_mount_directory() {
        let registry = Path::new("/data/QADREN/connections.sqlite3");
        let mount = smb_mount_dir(registry, "abc");
        assert_eq!(mount, Path::new("/data/QADREN/mounts/abc"));
        assert_eq!(
            smb_mount_dir(
                Path::new("/Users/u/Library/Application Support/QADREN/connections.sqlite3"),
                "abc"
            ),
            Path::new("/Users/u/.qadren/mounts/abc")
        );
        assert!(root_inside(&mount, Path::new("/data/QADREN/mounts/abc")));
        assert!(root_inside(
            &mount,
            Path::new("/data/QADREN/mounts/abc/Photos")
        ));
        assert!(!root_inside(&mount, Path::new("/data/QADREN/mounts/abcd")));
        assert!(!root_inside(
            &mount,
            Path::new("/data/QADREN/mounts/abc/../x")
        ));
        assert!(!root_inside(&mount, Path::new("mounts/abc")));
    }

    #[test]
    fn unmounted_smb_connection_is_not_ready() {
        let dir = std::env::temp_dir().join(format!("qadren-smb-gate-{}", std::process::id()));
        let registry_path = dir.join("connections.sqlite3");
        std::fs::create_dir_all(dir.join("mounts")).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let id = registry.add_smb_share("NAS", "smb://nas/share").unwrap();
        // An ordinary local directory in place of the mount is refused.
        std::fs::create_dir_all(smb_mount_dir(&registry_path, &id)).unwrap();
        assert!(!connection_id_ready(&registry, &registry_path, &id));
        let local = registry.add(LOCAL_FILESYSTEM, "Mac").unwrap();
        assert!(connection_id_ready(&registry, &registry_path, &local));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
