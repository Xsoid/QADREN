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
    if source_info.st_nlink != 1 {
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
        return if error.kind() == io::ErrorKind::AlreadyExists {
            Err(LocalError::AlreadyExists)
        } else if error.kind() == io::ErrorKind::Interrupted {
            // A syscall interrupted around commit may have changed the namespace.
            // Never retry a potentially destructive rename without inspection.
            Err(LocalError::OutcomeUnknown(error))
        } else {
            Err(LocalError::Io(error))
        };
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
    source_dir.sync_all().map_err(LocalError::OutcomeUnknown)?;
    target_dir.sync_all().map_err(LocalError::OutcomeUnknown)?;
    Ok(())
}

fn name(path: &std::ffi::OsStr) -> Result<CString> {
    CString::new(path.as_bytes()).map_err(|_| LocalError::InvalidPath)
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
fn leaf_metadata(directory: &File, leaf: &CStr) -> Result<(bool, u64)> {
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
    Ok((kind == libc::S_IFDIR, info.st_size.max(0) as u64))
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
        let (is_directory, size) = leaf_metadata(&dir, &leaf)?;
        let name = std::str::from_utf8(bytes).map_err(|_| LocalError::UnsafeEntry)?;
        entries.push(LocalEntry {
            name: name.to_owned(),
            is_directory,
            size,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    if entries.windows(2).any(|pair| pair[0].name == pair[1].name) {
        // Distinct non-UTF8 names must not silently collapse after lossy conversion.
        return Err(LocalError::UnsafeEntry);
    }
    Ok(entries)
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
    let expected = leaf.to_lowercase();
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
        if name.to_lowercase() == expected {
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
        if raw == leaf.as_bytes() {
            let leaf_name = name(std::ffi::OsStr::from_bytes(raw))?;
            let (is_directory, _) = leaf_metadata(&dir, leaf_name.as_c_str())?;
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
    let (is_directory, size) = leaf_metadata(&directory, &name)?;
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
    })
}

/// Open a regular file relative to the pinned directory for streaming.
/// macOS deliberately fails closed: opening an iCloud placeholder might trigger
/// materialization; a trusted resident-file check must precede implementation.
pub fn open_content(root: &File, target: &Path) -> Result<File> {
    #[cfg(target_os = "macos")]
    {
        let _ = (root, target);
        Err(LocalError::UnsafeEntry)
    }
    #[cfg(target_os = "linux")]
    {
        let (directory, leaf) = parent(root, target)?;
        let input = opened(unsafe {
            openat(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC,
            )
        })?;
        if !input.metadata()?.is_file() {
            return Err(LocalError::UnsafeEntry);
        }
        Ok(input)
    }
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
    directory.sync_all().map_err(LocalError::OutcomeUnknown)?;
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
    target_dir.sync_all().map_err(LocalError::OutcomeUnknown)?;
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
    let result = (|| -> Result<u64> {
        let bytes = io::copy(input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
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
            return if error.kind() == io::ErrorKind::AlreadyExists {
                Err(LocalError::AlreadyExists)
            } else {
                Err(LocalError::Io(error))
            };
        }
        published = true;
        post_link_check().map_err(LocalError::OutcomeUnknown)?;
        directory.sync_all().map_err(LocalError::OutcomeUnknown)?;
        Ok(bytes)
    })();
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
        directory.sync_all().map_err(LocalError::OutcomeUnknown)?;
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
