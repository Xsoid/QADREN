//! Descriptor-relative, symlink-resistant non-overwriting file creation.
//! Unix only. Every intermediate directory is opened with O_NOFOLLOW.
use super::{LocalEntry, LocalError, LocalFileIdentity, Result};
use std::ffi::{c_char, c_int, CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

#[cfg(target_os = "macos")]
const DIRECTORY: c_int = 0x100000;
#[cfg(target_os = "macos")]
const NOFOLLOW: c_int = 0x100;
#[cfg(target_os = "macos")]
const CLOEXEC: c_int = 0x1000000;
#[cfg(target_os = "linux")]
const DIRECTORY: c_int = 0x10000;
#[cfg(target_os = "linux")]
const NOFOLLOW: c_int = 0x20000;
#[cfg(target_os = "linux")]
const CLOEXEC: c_int = 0x80000;
const RDONLY: c_int = 0;
#[cfg(target_os = "linux")]
const NONBLOCK: c_int = 0x800;
const WRONLY: c_int = 1;
#[cfg(target_os = "macos")]
const CREAT: c_int = 0x200;
#[cfg(target_os = "macos")]
const EXCL: c_int = 0x800;
#[cfg(target_os = "linux")]
const CREATE_FLAG: c_int = 0x40;
#[cfg(target_os = "linux")]
const EXCLUSIVE_FLAG: c_int = 0x80;
#[cfg(target_os = "macos")]
const CREATE_FLAG: c_int = CREAT;
#[cfg(target_os = "macos")]
const EXCLUSIVE_FLAG: c_int = EXCL;

unsafe extern "C" {
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn openat(fd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    fn linkat(
        from_fd: c_int,
        from: *const c_char,
        to_fd: c_int,
        to: *const c_char,
        flags: c_int,
    ) -> c_int;
    fn unlinkat(fd: c_int, path: *const c_char, flags: c_int) -> c_int;
    fn mkdirat(fd: c_int, path: *const c_char, mode: u32) -> c_int;
}

/// Classify syscall failure without treating an interrupted rename as
/// safely retryable: a namespace mutation may already have occurred.
/// Do not rename or publish an alias of a known macOS dataless
/// placeholder. Metadata does not establish that other File Provider files
/// are fully local, so this is only a conservative negative guard.
#[cfg(target_os = "macos")]
fn reject_dataless_mutation(info: &libc::stat) -> Result<()> {
    const SF_DATALESS: u32 = 0x4000_0000;
    if info.st_flags & SF_DATALESS != 0 {
        return Err(LocalError::UnsafeEntry);
    }
    Ok(())
}

fn rename_failure(error: io::Error) -> LocalError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        LocalError::AlreadyExists
    } else if error.kind() == io::ErrorKind::Interrupted {
        LocalError::OutcomeUnknown(error)
    } else {
        LocalError::Io(error)
    }
}

/// Rename a regular file without replacing the destination. This moves
/// a directory entry atomically, but path-to-inode races still require
/// post-operation reconciliation before any job reports success.
pub fn rename_file_noreplace(root: &File, from: &Path, to: &Path) -> Result<()> {
    let (source_dir, source_name) = parent(root, from)?;
    let (target_dir, target_name) = parent(root, to)?;
    let source = file_identity(root, from)?;
    let mut source_info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            source_info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let source_info = unsafe { source_info.assume_init() };
    #[cfg(target_os = "macos")]
    reject_dataless_mutation(&source_info)?;
    if source_info.st_nlink != 1
        || source_info.st_dev as u64 != source.device
        || source_info.st_ino != source.inode
        || source_info.st_size.max(0) as u64 != source.size
    {
        return Err(LocalError::UnsafeEntry);
    }
    #[cfg(target_os = "linux")]
    let outcome = unsafe {
        libc::renameat2(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let outcome = unsafe {
        libc::renameatx_np(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if outcome != 0 {
        let error = io::Error::last_os_error();
        return Err(rename_failure(error));
    }
    let current = file_identity(root, to);
    if !matches!(current, Ok(identity) if identity == source) {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "renamed destination identity differs from preflight",
        )));
    }
    // A recreated source path is not proof that the rename failed; report
    // ambiguity and leave both paths untouched for explicit reconciliation.
    let mut remaining = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            remaining.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 || io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "source pathname was not confirmed absent after rename",
        )));
    }
    sync(&source_dir).map_err(LocalError::OutcomeUnknown)?;
    sync(&target_dir).map_err(LocalError::OutcomeUnknown)?;
    Ok(())
}

/// Atomically replace one regular destination entry with a checked regular
/// source entry. The caller owns the replacement policy and must persist the
/// attempt before this effect. This primitive never follows a destination
/// symlink and reports a post-publication identity mismatch as UNKNOWN.
pub fn rename_file_replace(root: &File, from: &Path, to: &Path) -> Result<()> {
    replace_checked(root, from, to, 1)
}

/// Like `rename_file_replace`, for a destination that has exactly one more
/// name: the Trash link that `link_file_into_trash` made. After the rename the
/// previous version exists only in the Trash.
#[cfg(target_os = "macos")]
pub fn rename_file_replace_linked(root: &File, from: &Path, to: &Path) -> Result<()> {
    replace_checked(root, from, to, 2)
}

fn replace_checked(root: &File, from: &Path, to: &Path, target_links: u64) -> Result<()> {
    let (source_dir, source_name) = parent(root, from)?;
    let (target_dir, target_name) = parent(root, to)?;
    let source = file_identity(root, from)?;
    let mut source_info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            source_info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let source_info = unsafe { source_info.assume_init() };
    #[cfg(target_os = "macos")]
    reject_dataless_mutation(&source_info)?;
    if source_info.st_mode & libc::S_IFMT != libc::S_IFREG
        || source_info.st_nlink != 1
        || source_info.st_dev as u64 != source.device
        || source_info.st_ino != source.inode
        || source_info.st_size.max(0) as u64 != source.size
    {
        return Err(LocalError::UnsafeEntry);
    }
    let mut target_info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            target_info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let target_info = unsafe { target_info.assume_init() };
    #[cfg(target_os = "macos")]
    reject_dataless_mutation(&target_info)?;
    if target_info.st_mode & libc::S_IFMT != libc::S_IFREG
        || target_info.st_nlink as u64 != target_links
    {
        return Err(LocalError::UnsafeEntry);
    }
    #[cfg(target_os = "linux")]
    let outcome = unsafe {
        libc::renameat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
        )
    };
    #[cfg(target_os = "macos")]
    let outcome = unsafe {
        libc::renameatx_np(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            0,
        )
    };
    if outcome != 0 {
        return Err(rename_failure(io::Error::last_os_error()));
    }
    let replaced = if inode_follows_name(&target_dir) {
        // The inode number on this volume names the path, not the file.
        // Size and modification time of the staged file are the evidence.
        let mut after = std::mem::MaybeUninit::<libc::stat>::uninit();
        let found = unsafe {
            libc::fstatat(
                target_dir.as_raw_fd(),
                target_name.as_ptr(),
                after.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        found && {
            let after = unsafe { after.assume_init() };
            after.st_mode & libc::S_IFMT == libc::S_IFREG
                && after.st_size == source_info.st_size
                && after.st_mtime == source_info.st_mtime
                && after.st_mtime_nsec == source_info.st_mtime_nsec
        }
    } else {
        matches!(file_identity(root, to), Ok(identity) if identity == source)
    };
    if !replaced {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "replaced destination identity differs from staged source",
        )));
    }
    Ok(())
}

/// True on smbfs. There the inode number belongs to the path name: a file
/// renamed over another name gets the inode number of that name (checked on
/// 2026-10-25 on an SMB share). Inode checks then cannot tell one file from
/// another at the same name, so callers use other evidence.
#[cfg(target_os = "macos")]
pub(super) fn inode_follows_name(directory: &File) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(directory.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return false;
    }
    let info = unsafe { info.assume_init() };
    unsafe { CStr::from_ptr(info.f_fstypename.as_ptr()) }.to_bytes() == b"smbfs"
}

#[cfg(not(target_os = "macos"))]
pub(super) fn inode_follows_name(_directory: &File) -> bool {
    false
}

fn name(path: &std::ffi::OsStr) -> Result<CString> {
    CString::new(path.as_bytes()).map_err(|_| LocalError::InvalidPath)
}

/// NFC form of a name. Name comparisons use it, so one name in NFC and in
/// NFD (as smbfs lists names) is one name.
pub(super) fn nfc(name: &str) -> String {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
        .normalize(name)
        .into_owned()
}

/// Flush a file or directory to stable storage. On macOS `sync_all` uses
/// `F_FULLFSYNC`, which smbfs refuses with ENOTSUP (checked on 2026-10-25 on
/// an SMB share). Then plain `fsync` is the strongest flush the volume offers.
fn sync(file: &File) -> io::Result<()> {
    match file.sync_all() {
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EOPNOTSUPP)) => {
            if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        other => other,
    }
}

fn opened(fd: c_int) -> Result<File> {
    if fd < 0 {
        let error = io::Error::last_os_error();
        // O_NOFOLLOW rejects final symlinks with ELOOP. O_DIRECTORY may
        // report ENOTDIR for a symlink in an intermediate component.
        // ELOOP is 40 on Linux and 62 on macOS.
        // Both cases must remain a hard failure without traversal.
        if matches!(error.raw_os_error(), Some(20 | 40 | 62)) {
            return Err(LocalError::UnsafeEntry);
        }
        return Err(LocalError::Io(error));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub fn root(path: &Path) -> Result<File> {
    // O_NOFOLLOW protects only the final component of a pathname.
    // Walk every component relative to a pinned descriptor, including the
    // ancestors of the configured root, to reject intermediate symlinks.
    let anchor = if path.is_absolute() { "/" } else { "." };
    let anchor = name(std::ffi::OsStr::new(anchor))?;
    let mut current =
        opened(unsafe { open(anchor.as_ptr(), RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC) })?;
    for component in path.components() {
        let part = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(part) => part,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(LocalError::InvalidPath);
            }
        };
        let part = name(part)?;
        current = opened(unsafe {
            openat(
                current.as_raw_fd(),
                part.as_ptr(),
                RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
            )
        })?;
    }
    Ok(current)
}

/// Detect whether two pinned roots are identical or nested without
/// opening file contents or relying on canonical pathname comparisons.
pub fn roots_overlap(first: &File, second: &File) -> Result<bool> {
    fn ancestor_contains(root: &File, target: (u64, u64)) -> Result<bool> {
        let mut current = root.try_clone()?;
        let dotdot = CString::new("..").map_err(|_| LocalError::InvalidPath)?;
        for _ in 0..1024 {
            let metadata = current.metadata()?;
            let identity = (metadata.dev(), metadata.ino());
            if identity == target {
                return Ok(true);
            }
            let parent = opened(unsafe {
                openat(
                    current.as_raw_fd(),
                    dotdot.as_ptr(),
                    RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
                )
            })?;
            let higher = parent.metadata()?;
            if (higher.dev(), higher.ino()) == identity {
                return Ok(false);
            }
            current = parent;
        }
        Err(LocalError::UnsafeEntry)
    }
    let a = first.metadata()?;
    let b = second.metadata()?;
    Ok(ancestor_contains(first, (b.dev(), b.ino()))?
        || ancestor_contains(second, (a.dev(), a.ino()))?)
}

fn parent(root: &File, relative: &Path) -> Result<(File, CString)> {
    let mut components = relative.components().peekable();
    let mut current = root.try_clone()?;
    while let Some(component) = components.next() {
        let Component::Normal(part) = component else {
            return Err(LocalError::InvalidPath);
        };
        let part = name(part)?;
        if components.peek().is_none() {
            return Ok((current, part));
        }
        current = opened(unsafe {
            openat(
                current.as_raw_fd(),
                part.as_ptr(),
                RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
            )
        })?;
    }
    Err(LocalError::InvalidPath)
}

/// Open the requested directory without resolving an untrusted pathname.
fn directory(root: &File, relative: &Path) -> Result<File> {
    if relative.as_os_str().is_empty() {
        // dup/try_clone shares a directory offset with root_fd. Reopen "."
        // relative to the pinned descriptor to give each scan its own offset.
        let dot = CString::new(".").map_err(|_| LocalError::InvalidPath)?;
        return opened(unsafe {
            openat(
                root.as_raw_fd(),
                dot.as_ptr(),
                RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
            )
        });
    }
    let (parent, leaf) = parent(root, relative)?;
    opened(unsafe {
        openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
        )
    })
}

/// Read leaf metadata without opening file content. This avoids triggering
/// placeholder materialization through metadata-only list/stat calls.
///
/// The revision deliberately omits ctime. macOS changes ctime when it
/// downloads or evicts a File Provider placeholder (iCloud Drive and similar),
/// which is not a content change. With ctime, every eviction would look like
/// an edit and the Scenario baseline would never stay confirmed.
fn local_object_revision(info: &libc::stat) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return Some(format!(
            "local:{}:{}:{:x}",
            info.st_dev,
            info.st_ino,
            crate::local_fs::provider_stable_mtime(info.st_mtime, info.st_mtime_nsec),
        ));
    }
    #[cfg(target_os = "linux")]
    {
        return Some(format!(
            "local:{}:{}:{:x}",
            info.st_dev,
            info.st_ino,
            crate::local_fs::provider_stable_mtime(info.st_mtim.tv_sec, info.st_mtim.tv_nsec),
        ));
    }
    #[allow(unreachable_code)]
    None
}

/// Read leaf metadata without opening file content. This avoids triggering
/// placeholder materialization through metadata-only list/stat calls.
fn leaf_metadata(directory: &File, leaf: &CStr) -> Result<(bool, u64, Option<String>)> {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    let kind = info.st_mode & libc::S_IFMT;
    if kind != libc::S_IFREG && kind != libc::S_IFDIR {
        return Err(LocalError::UnsafeEntry);
    }
    Ok((
        kind == libc::S_IFDIR,
        info.st_size.max(0) as u64,
        local_object_revision(&info),
    ))
}

/// Remove one checked regular entry relative to the pinned root. The caller
/// must persist an attempt first. EINTR is UNKNOWN because the namespace may
/// already have changed.
pub fn delete_file(root: &File, target: &Path) -> Result<()> {
    let (directory, leaf) = parent(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalError::UnsafeEntry);
    }
    #[cfg(target_os = "macos")]
    reject_dataless_mutation(&info)?;
    let status = unsafe { unlinkat(directory.as_raw_fd(), leaf.as_ptr(), 0) };
    if status != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Err(LocalError::OutcomeUnknown(error))
        } else {
            Err(LocalError::Io(error))
        };
    }
    Ok(())
}

/// The system Trash directory for files on device `device`, or `None` when
/// that volume has no usable Trash. This never creates `.Trashes` itself:
/// only the per-user directory inside an existing `.Trashes` or the home
/// `.Trash`. The result is a pinned directory owned by this user.
#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
pub fn system_trash_directory(near: &File, device: u64) -> Result<Option<File>> {
    let uid = unsafe { libc::getuid() };
    let owned_on_device = |directory: &File| -> Result<bool> {
        let metadata = directory.metadata()?;
        Ok(metadata.is_dir() && metadata.dev() == device && metadata.uid() == uid)
    };
    let child = |parent: &File, leaf: &str, create: bool| -> Result<Option<File>> {
        let leaf = name(std::ffi::OsStr::new(leaf))?;
        let open = || unsafe {
            openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
            )
        };
        let mut fd = open();
        if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            if !create {
                return Ok(None);
            }
            if unsafe { mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0
                && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
            {
                return Err(LocalError::Io(io::Error::last_os_error()));
            }
            fd = open();
        }
        opened(fd).map(Some)
    };

    // The home Trash serves the volume of the home folder.
    let mut buffer = vec![0 as c_char; 16 * 1024];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            entry.as_mut_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status == 0 && !found.is_null() {
        let home = unsafe { CStr::from_ptr((*found).pw_dir) };
        let home = Path::new(std::ffi::OsStr::from_bytes(home.to_bytes()));
        if let Ok(home) = root(home) {
            if home.metadata()?.dev() == device {
                return match child(&home, ".Trash", true)? {
                    Some(trash) if owned_on_device(&trash)? => Ok(Some(trash)),
                    _ => Ok(None),
                };
            }
        }
    }

    // Another volume uses <mount point>/.Trashes/<uid>.
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(near.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    let mount = unsafe { CStr::from_ptr(info.f_mntonname.as_ptr()) };
    let mount = root(Path::new(std::ffi::OsStr::from_bytes(mount.to_bytes())))?;
    let Some(trashes) = child(&mount, ".Trashes", false)? else {
        return Ok(None);
    };
    match child(&trashes, &uid.to_string(), true)? {
        Some(trash) if owned_on_device(&trash)? => Ok(Some(trash)),
        _ => Ok(None),
    }
}

/// Candidate names in a Trash directory: `a.txt`, `a 2.txt`, `a 3.txt`, ...
fn trash_name(leaf: &CStr, attempt: usize) -> Result<CString> {
    if attempt == 0 {
        return Ok(leaf.to_owned());
    }
    let bytes = leaf.to_bytes();
    let split = bytes
        .iter()
        .rposition(|byte| *byte == b'.')
        .filter(|index| *index > 0)
        .unwrap_or(bytes.len());
    let mut candidate = bytes[..split].to_vec();
    candidate.extend_from_slice(format!(" {}", attempt + 1).as_bytes());
    candidate.extend_from_slice(&bytes[split..]);
    CString::new(candidate).map_err(|_| LocalError::InvalidPath)
}

/// Move one checked regular entry into `trash`, which must be on the same
/// volume. A name in the Trash is never replaced. The caller must persist an
/// attempt first. An interrupted or unconfirmed move is UNKNOWN.
#[cfg(target_os = "macos")]
pub fn trash_file_into(root: &File, target: &Path, trash: &File) -> Result<()> {
    let (directory, leaf) = parent(root, target)?;
    let source = file_identity(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    reject_dataless_mutation(&info)?;
    if info.st_mode & libc::S_IFMT != libc::S_IFREG
        || info.st_dev as u64 != source.device
        || info.st_ino != source.inode
        || trash.metadata()?.dev() != source.device
    {
        return Err(LocalError::UnsafeEntry);
    }
    for attempt in 0..100 {
        let candidate = trash_name(&leaf, attempt)?;
        let outcome = unsafe {
            libc::renameatx_np(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                trash.as_raw_fd(),
                candidate.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if outcome != 0 {
            match rename_failure(io::Error::last_os_error()) {
                LocalError::AlreadyExists => continue,
                error => return Err(error),
            }
        }
        let mut moved = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                trash.as_raw_fd(),
                candidate.as_ptr(),
                moved.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        let unconfirmed = || {
            LocalError::OutcomeUnknown(io::Error::other(
                "the Trash entry identity differs from preflight",
            ))
        };
        if status != 0 {
            return Err(unconfirmed());
        }
        let moved = unsafe { moved.assume_init() };
        if moved.st_dev as u64 != source.device || moved.st_ino != source.inode {
            return Err(unconfirmed());
        }
        sync(&directory).map_err(LocalError::OutcomeUnknown)?;
        sync(trash).map_err(LocalError::OutcomeUnknown)?;
        return Ok(());
    }
    Err(LocalError::AlreadyExists)
}

/// Put a second name for one checked regular file into `trash`, which must
/// be on the same volume. The file stays in place, so a later atomic replace
/// of its name leaves the previous version only in the Trash. A name in the
/// Trash is never replaced. Returns `Ok(false)` before any effect when the
/// volume has no hard links. The caller must persist an attempt first.
#[cfg(target_os = "macos")]
pub fn link_file_into_trash(root: &File, target: &Path, trash: &File) -> Result<bool> {
    let (directory, leaf) = parent(root, target)?;
    let source = file_identity(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    reject_dataless_mutation(&info)?;
    if info.st_mode & libc::S_IFMT != libc::S_IFREG
        || info.st_nlink != 1
        || info.st_dev as u64 != source.device
        || info.st_ino != source.inode
        || trash.metadata()?.dev() != source.device
    {
        return Err(LocalError::UnsafeEntry);
    }
    for attempt in 0..100 {
        let candidate = trash_name(&leaf, attempt)?;
        let outcome = unsafe {
            libc::linkat(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                trash.as_raw_fd(),
                candidate.as_ptr(),
                0,
            )
        };
        if outcome != 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EEXIST) => continue,
                Some(libc::ENOTSUP | libc::EOPNOTSUPP) => return Ok(false),
                _ => return Err(LocalError::Io(error)),
            }
        }
        let mut linked = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                trash.as_raw_fd(),
                candidate.as_ptr(),
                linked.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        let unconfirmed = || {
            LocalError::OutcomeUnknown(io::Error::other(
                "the Trash link identity differs from preflight",
            ))
        };
        if status != 0 {
            return Err(unconfirmed());
        }
        let linked = unsafe { linked.assume_init() };
        if linked.st_dev as u64 != source.device || linked.st_ino != source.inode {
            return Err(unconfirmed());
        }
        sync(trash).map_err(LocalError::OutcomeUnknown)?;
        return Ok(true);
    }
    Err(LocalError::AlreadyExists)
}

/// Inspect an inode relative to the pinned root without opening content.
pub fn file_identity(root: &File, target: &Path) -> Result<LocalFileIdentity> {
    let (directory, leaf) = parent(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalError::UnsafeEntry);
    }
    Ok(LocalFileIdentity {
        device: info.st_dev as u64,
        inode: info.st_ino,
        size: info.st_size.max(0) as u64,
    })
}

/// Inspect POSIX mode bits without accessing file content. These bits
/// are not an effective-access decision (ACLs and sandbox rules also apply).
pub fn file_mode(root: &File, target: &Path) -> Result<u32> {
    let (directory, leaf) = parent(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    let kind = info.st_mode & libc::S_IFMT;
    if kind != libc::S_IFREG && kind != libc::S_IFDIR {
        return Err(LocalError::UnsafeEntry);
    }
    Ok((info.st_mode & 0o7777) as u32)
}

/// Metadata-only modification signature. Timestamps are best-effort,
/// not a cryptographic integrity proof or immutable snapshot.
pub fn file_change_stamp(root: &File, target: &Path) -> Result<(i64, i64, i64, i64)> {
    let (directory, leaf) = parent(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalError::UnsafeEntry);
    }
    Ok((
        info.st_mtime,
        info.st_mtime_nsec,
        info.st_ctime,
        info.st_ctime_nsec,
    ))
}

/// Inspect macOS SF_DATALESS without opening the file or its content.
/// No flag does not prove that the object cannot be materialized later.
#[cfg(target_os = "macos")]
pub fn is_dataless(root: &File, target: &Path) -> Result<bool> {
    const SF_DATALESS: u32 = 0x4000_0000;
    let (directory, leaf) = parent(root, target)?;
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    if info.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalError::UnsafeEntry);
    }
    Ok(info.st_flags & SF_DATALESS != 0)
}

/// Enumerate via fdopendir/readdir. On macOS /dev/fd/N is not a
/// directory pathname usable by read_dir, so enumeration must use the fd.
pub fn list(root: &File, relative: &Path) -> Result<Vec<LocalEntry>> {
    list_with_skipped(root, relative).map(|(entries, _)| entries)
}

/// Enumerate regular files and directories. Symbolic links and special files
/// (sockets, FIFOs, devices) are never followed or synchronized; their names
/// are returned separately, so callers can disclose them truthfully.
pub fn list_with_skipped(root: &File, relative: &Path) -> Result<(Vec<LocalEntry>, Vec<String>)> {
    let dir = directory(root, relative)?;
    let owned_fd = dir.try_clone()?.into_raw_fd();
    let stream = unsafe { libc::fdopendir(owned_fd) };
    if stream.is_null() {
        unsafe { libc::close(owned_fd) };
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    struct DirectoryStream(*mut libc::DIR);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = DirectoryStream(stream);
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { libc::__errno_location() };
        unsafe { *errno = 0 };
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let code = unsafe { *errno };
            if code != 0 {
                return Err(LocalError::Io(io::Error::from_raw_os_error(code)));
            }
            break;
        }
        let raw_name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = raw_name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let leaf = CString::new(bytes).map_err(|_| LocalError::InvalidPath)?;
        let name = std::str::from_utf8(bytes).map_err(|_| LocalError::UnsafeEntry)?;
        let (is_directory, size, object_revision) = match leaf_metadata(&dir, &leaf) {
            Ok(metadata) => metadata,
            // leaf_metadata reports UnsafeEntry only for an entry that is not a
            // regular file or directory, such as a symbolic link.
            Err(LocalError::UnsafeEntry) => {
                skipped.push(name.to_owned());
                continue;
            }
            Err(error) => return Err(error),
        };
        entries.push(LocalEntry {
            name: name.to_owned(),
            is_directory,
            size,
            object_revision,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    if entries.windows(2).any(|pair| pair[0].name == pair[1].name) {
        // Distinct non-UTF8 names must not silently collapse after lossy conversion.
        return Err(LocalError::UnsafeEntry);
    }
    skipped.sort();
    Ok((entries, skipped))
}

/// Check sibling names without stat-ing unrelated entries. A directory may
/// contain symlinks; they must not block creation of an independent file.
pub fn has_case_collision(root: &File, relative: &Path, leaf: &str) -> Result<bool> {
    let dir = directory(root, relative)?;
    let owned_fd = dir.try_clone()?.into_raw_fd();
    let stream = unsafe { libc::fdopendir(owned_fd) };
    if stream.is_null() {
        unsafe { libc::close(owned_fd) };
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(stream);
    let expected = nfc(leaf).to_lowercase();
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { libc::__errno_location() };
        unsafe { *errno = 0 };
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let code = unsafe { *errno };
            return if code == 0 {
                Ok(false)
            } else {
                Err(LocalError::Io(io::Error::from_raw_os_error(code)))
            };
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes).map_err(|_| LocalError::UnsafeEntry)?;
        if nfc(name).to_lowercase() == expected {
            return Ok(true);
        }
    }
}

/// Check an exact directory entry spelling without inspecting unrelated
/// symlinks. A positive result is observational, not a lock or a snapshot.
pub fn has_exact_directory(root: &File, relative: &Path, leaf: &str) -> Result<bool> {
    let dir = directory(root, relative)?;
    let owned_fd = dir.try_clone()?.into_raw_fd();
    let stream = unsafe { libc::fdopendir(owned_fd) };
    if stream.is_null() {
        unsafe { libc::close(owned_fd) };
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(stream);
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { libc::__errno_location() };
        unsafe { *errno = 0 };
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let code = unsafe { *errno };
            return if code == 0 {
                Ok(false)
            } else {
                Err(LocalError::Io(io::Error::from_raw_os_error(code)))
            };
        }
        let raw = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        // Same spelling up to the Unicode form: smbfs lists names in NFD
        // while QADREN and providers give NFC (checked on 2026-10-25).
        if raw == leaf.as_bytes() || std::str::from_utf8(raw).is_ok_and(|raw| nfc(raw) == nfc(leaf))
        {
            let leaf_name = name(std::ffi::OsStr::from_bytes(raw))?;
            let (is_directory, _, _) = leaf_metadata(&dir, leaf_name.as_c_str())?;
            return Ok(is_directory);
        }
    }
}

/// Detect revisiting the same directory while scanning an untrusted tree.
pub fn directory_identity(root: &File, relative: &Path) -> Result<(u64, u64)> {
    let directory = directory(root, relative)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Err(LocalError::UnsafeEntry);
    }
    Ok((metadata.dev(), metadata.ino()))
}

/// Metadata lookup without opening the final file or resolving symlinks.
pub fn stat(root: &File, target: &Path) -> Result<LocalEntry> {
    let (directory, name) = parent(root, target)?;
    let (is_directory, size, object_revision) = leaf_metadata(&directory, &name)?;
    let display_name = target
        .file_name()
        .ok_or(LocalError::InvalidPath)?
        .to_str()
        .ok_or(LocalError::UnsafeEntry)?
        .to_owned();
    Ok(LocalEntry {
        name: display_name,
        is_directory,
        size,
        object_revision,
    })
}

/// Open a regular file relative to the pinned directory for streaming.
/// The caller must already have durable, exact-scope consent. On macOS this
/// open can materialize a File Provider item.
pub fn open_content(root: &File, target: &Path) -> Result<File> {
    let (directory, leaf) = parent(root, target)?;
    #[cfg(target_os = "linux")]
    let flags = RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC;
    #[cfg(target_os = "macos")]
    let flags = RDONLY | NOFOLLOW | CLOEXEC;
    let input = opened(unsafe { openat(directory.as_raw_fd(), leaf.as_ptr(), flags) })?;
    if !input.metadata()?.is_file() {
        return Err(LocalError::UnsafeEntry);
    }
    Ok(input)
}

/// Creates one directory without following symlinks or replacing existing entries.
pub fn create_directory(root: &File, target: &Path) -> Result<()> {
    create_directory_with_post_mkdir_check(root, target, || Ok(()))
}

/// Test seam after mkdirat succeeds, before the parent directory is synced.
pub(super) fn create_directory_with_post_mkdir_check<F: FnOnce() -> io::Result<()>>(
    root: &File,
    target: &Path,
    post_mkdir_check: F,
) -> Result<()> {
    let (directory, destination) = parent(root, target)?;
    let parent_path = target.parent().ok_or(LocalError::InvalidPath)?;
    let leaf = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LocalError::UnsafeEntry)?;
    if has_case_collision(root, parent_path, leaf)? {
        return Err(LocalError::AlreadyExists);
    }
    let result = unsafe { mkdirat(directory.as_raw_fd(), destination.as_ptr(), 0o700) };
    if result != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::AlreadyExists {
            Err(LocalError::AlreadyExists)
        } else {
            Err(LocalError::Io(error))
        };
    }
    post_mkdir_check().map_err(LocalError::OutcomeUnknown)?;
    sync(&directory).map_err(LocalError::OutcomeUnknown)?;
    Ok(())
}

/// Publish a second hard link without reading content or deleting the source.
/// This is a non-destructive move staging primitive, NOT a completed move.
/// A racing source replacement is detected after publication as OutcomeUnknown.
pub fn link_file_new(root: &File, from: &Path, to: &Path) -> Result<()> {
    link_file_new_with_post_link_check(root, from, to, || Ok(()))
}

/// Fault injection after publication, before source identity reconciliation.
pub(super) fn link_file_new_with_post_link_check<F: FnOnce() -> io::Result<()>>(
    root: &File,
    from: &Path,
    to: &Path,
    post_link_check: F,
) -> Result<()> {
    let (source_dir, source_name) = parent(root, from)?;
    let (target_dir, target_name) = parent(root, to)?;
    let mut source_info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            source_info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    let source_info = unsafe { source_info.assume_init() };
    #[cfg(target_os = "macos")]
    reject_dataless_mutation(&source_info)?;
    if source_info.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalError::UnsafeEntry);
    }
    // A staging operation must not introduce another link to a file
    // already shared via hard links: such files require explicit conflict
    // handling, not implicit aliasing.
    if source_info.st_nlink != 1 {
        return Err(LocalError::UnsafeEntry);
    }
    if unsafe {
        linkat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            0,
        )
    } != 0
    {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::AlreadyExists {
            Err(LocalError::AlreadyExists)
        } else {
            Err(LocalError::Io(error))
        };
    }
    // Published; all subsequent failures are ambiguous and MUST NOT retry.
    post_link_check().map_err(LocalError::OutcomeUnknown)?;
    // The source pathname must still resolve to the inspected inode.
    // This is a best-effort race detector, not an atomic source guard.
    let mut current_source = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            current_source.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::OutcomeUnknown(io::Error::last_os_error()));
    }
    let current_source = unsafe { current_source.assume_init() };
    if current_source.st_dev != source_info.st_dev
        || current_source.st_ino != source_info.st_ino
        || current_source.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "source pathname changed during hard-link publication",
        )));
    }
    sync(&target_dir).map_err(LocalError::OutcomeUnknown)?;
    let mut target_info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            target_dir.as_raw_fd(),
            target_name.as_ptr(),
            target_info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(LocalError::OutcomeUnknown(io::Error::last_os_error()));
    }
    let target_info = unsafe { target_info.assume_init() };
    if target_info.st_dev != source_info.st_dev
        || target_info.st_ino != source_info.st_ino
        || target_info.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "published hard link does not match inspected source",
        )));
    }
    Ok(())
}

/// Ensure a temporary directory entry still refers to the inode opened by
/// this operation. This detects replaced names; it is not an atomic guarantee
/// against a malicious process with write access to the same directory.
fn same_temporary_inode(
    directory: &File,
    temporary: &CStr,
    output: &File,
    expected_links: u64,
) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            info.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return false;
    }
    let info = unsafe { info.assume_init() };
    let Ok(opened) = output.metadata() else {
        return false;
    };
    #[cfg(target_os = "macos")]
    let same_device = u64::try_from(info.st_dev).ok() == Some(opened.dev());
    #[cfg(target_os = "linux")]
    let same_device = info.st_dev == opened.dev();
    same_device && info.st_ino == opened.ino() && (info.st_nlink as u64) == expected_links
}

/// Publish the synced `temporary` file as the new `destination` on a volume
/// without hard links and without an exclusive rename. SMB shares can lack
/// both (ENOTSUP for `linkat` and `RENAME_EXCL`, checked on 2026-10-25),
/// while an `O_EXCL` create there still fails on an existing name, also for
/// another letter case, and a plain rename replaces silently.
///
/// So an `O_EXCL` create first reserves the name. It fails on any existing
/// file. Just before the rename, the reservation must still be our empty
/// file; only then does the rename replace it. A name that another writer
/// took is never replaced. The short time between that check and the rename
/// is the remaining race; it can affect only the reserved name.
fn publish_by_reservation(directory: &File, temporary: &CStr, destination: &CStr) -> Result<()> {
    let reservation = opened(unsafe {
        openat(
            directory.as_raw_fd(),
            destination.as_ptr(),
            WRONLY | CREATE_FLAG | EXCLUSIVE_FLAG | NOFOLLOW | CLOEXEC,
            0o600 as c_int,
        )
    })
    .map_err(|error| match error {
        LocalError::Io(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            LocalError::AlreadyExists
        }
        other => other,
    })?;
    let reserved = reservation.metadata().map_err(LocalError::OutcomeUnknown)?;
    drop(reservation);
    let still_reserved = || {
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                destination.as_ptr(),
                info.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return false;
        }
        let info = unsafe { info.assume_init() };
        info.st_ino == reserved.ino()
            && u64::try_from(info.st_dev).ok() == Some(reserved.dev())
            && info.st_size == 0
            && info.st_mode & libc::S_IFMT == libc::S_IFREG
    };
    // Remove our own empty reservation after a failure before publication.
    // If it is not ours and empty any more, the outcome is not known.
    let release = |error: io::Error| -> LocalError {
        if still_reserved()
            && unsafe { unlinkat(directory.as_raw_fd(), destination.as_ptr(), 0) } == 0
        {
            LocalError::Io(error)
        } else {
            LocalError::OutcomeUnknown(error)
        }
    };
    if !still_reserved() {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "destination reservation changed before publication",
        )));
    }
    let renamed = unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    if renamed != 0 {
        let error = io::Error::last_os_error();
        return Err(if error.kind() == io::ErrorKind::Interrupted {
            LocalError::OutcomeUnknown(error)
        } else {
            release(error)
        });
    }
    Ok(())
}

pub fn create<R: Read>(root: &File, target: &Path, input: &mut R, serial: u64) -> Result<u64> {
    create_with_post_link_check(root, target, input, serial, || Ok(()))
}

/// Fault-injection seam: invoked strictly after publishing the destination.
/// Production always passes a no-op; tests can simulate a durability failure.
pub(super) fn create_with_post_link_check<R: Read, F: FnOnce() -> io::Result<()>>(
    root: &File,
    target: &Path,
    input: &mut R,
    serial: u64,
    post_link_check: F,
) -> Result<u64> {
    let (directory, destination) = parent(root, target)?;
    // Reject an occupied destination before reading potentially large input.
    // The final linkat still performs the authoritative atomic no-overwrite
    // check, since this inspection is subject to concurrent changes.
    let mut existing = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            destination.as_ptr(),
            existing.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        return Err(LocalError::AlreadyExists);
    }
    let error = io::Error::last_os_error();
    if error.kind() != io::ErrorKind::NotFound {
        return Err(LocalError::Io(error));
    }
    let temporary = CString::new(format!(".qadren-{}-{serial}.tmp", std::process::id()))
        .map_err(|_| LocalError::InvalidPath)?;
    let mut output = opened(unsafe {
        openat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            WRONLY | CREATE_FLAG | EXCLUSIVE_FLAG | NOFOLLOW | CLOEXEC,
            0o600 as c_int,
        )
    })?;
    let mut published = false;
    // True when the staging file was renamed to the destination because the
    // volume has no hard links (see `publish_by_reservation`).
    let mut renamed = false;
    let result = (|| -> Result<u64> {
        let bytes = io::copy(input, &mut output)?;
        output.flush()?;
        sync(&output)?;
        if !same_temporary_inode(
            &directory,
            &temporary,
            &output,
            if published { 2 } else { 1 },
        ) {
            return Err(LocalError::UnsafeEntry);
        }
        let linked = unsafe {
            linkat(
                directory.as_raw_fd(),
                temporary.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
                0,
            )
        };
        if linked != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(LocalError::AlreadyExists);
            }
            if !matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EOPNOTSUPP)) {
                return Err(LocalError::Io(error));
            }
            publish_by_reservation(&directory, &temporary, &destination)?;
            renamed = true;
            published = true;
            post_link_check().map_err(LocalError::OutcomeUnknown)?;
            sync(&directory).map_err(LocalError::OutcomeUnknown)?;
            return Ok(bytes);
        }
        published = true;
        post_link_check().map_err(LocalError::OutcomeUnknown)?;
        sync(&directory).map_err(LocalError::OutcomeUnknown)?;
        Ok(bytes)
    })();
    if renamed {
        // The staging name is gone. The destination must be our only link.
        // On smbfs the inode number follows the name, so an open file and its
        // new name can report different numbers; the size is the evidence.
        let published_here = |bytes: u64| {
            if !inode_follows_name(&directory) {
                return same_temporary_inode(&directory, &destination, &output, 1);
            }
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            let found = unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    destination.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } == 0;
            found && {
                let info = unsafe { info.assume_init() };
                info.st_mode & libc::S_IFMT == libc::S_IFREG
                    && u64::try_from(info.st_size).ok() == Some(bytes)
            }
        };
        return match result {
            Ok(bytes) if published_here(bytes) => Ok(bytes),
            Ok(_) => Err(LocalError::OutcomeUnknown(io::Error::other(
                "destination file identity changed after publication",
            ))),
            Err(error) => Err(error),
        };
    }
    // An interrupted reservation rename can have moved the staging file.
    // Keep that UNKNOWN; staging cleanup handles a staging file left behind.
    if !published && matches!(result, Err(LocalError::OutcomeUnknown(_))) {
        return result;
    }
    // Even after publication, never unlink a staging pathname replaced
    // by another writer. The published destination requires reconciliation.
    if !same_temporary_inode(
        &directory,
        &temporary,
        &output,
        if published { 2 } else { 1 },
    ) {
        return if published {
            Err(LocalError::OutcomeUnknown(io::Error::other(
                "temporary file identity changed after publication",
            )))
        } else {
            Err(LocalError::UnsafeEntry)
        };
    }
    // After linkat, the destination name must still refer to our opened
    // inode. A replaced destination must never be reported as a success.
    if published && !same_temporary_inode(&directory, &destination, &output, 2) {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "destination file identity changed after publication",
        )));
    }
    let cleanup = unsafe { unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0) };
    if cleanup != 0 {
        let error = io::Error::last_os_error();
        if published {
            return Err(LocalError::OutcomeUnknown(error));
        }
        if result.is_ok() {
            return Err(LocalError::Io(error));
        }
        return result;
    }
    if published {
        sync(&directory).map_err(LocalError::OutcomeUnknown)?;
        // The staging link has been removed; the published name must now be
        // the sole link to our opened inode. A concurrent replacement after
        // cleanup cannot be reported as a confirmed successful creation.
        if !same_temporary_inode(&directory, &destination, &output, 1) {
            return Err(LocalError::OutcomeUnknown(io::Error::other(
                "destination file identity changed after staging cleanup",
            )));
        }
    }
    result
}

#[cfg(test)]
mod rename_error_tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn dataless_flag_blocks_mutating_file_operations() {
        let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
        metadata.st_flags = 0x4000_0000;
        assert!(matches!(
            reject_dataless_mutation(&metadata),
            Err(LocalError::UnsafeEntry)
        ));
        metadata.st_flags = 0;
        assert!(reject_dataless_mutation(&metadata).is_ok());
    }

    #[test]
    fn interrupted_rename_is_ambiguous_and_must_not_be_retried() {
        let classified = rename_failure(io::Error::from(io::ErrorKind::Interrupted));
        assert!(matches!(classified, LocalError::OutcomeUnknown(_)));
    }

    #[test]
    fn occupied_rename_destination_is_a_known_collision() {
        let classified = rename_failure(io::Error::from(io::ErrorKind::AlreadyExists));
        assert!(matches!(classified, LocalError::AlreadyExists));
    }

    #[test]
    fn ordinary_pre_commit_rename_error_stays_io() {
        let classified = rename_failure(io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(matches!(classified, LocalError::Io(_)));
    }
}
