//! Descriptor-relative, symlink-resistant non-overwriting file creation.
//! Unix only. Every intermediate directory is opened with O_NOFOLLOW.
use super::{LocalEntry, LocalError, Result};
use std::ffi::{c_char, c_int, CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::OsStrExt;
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
#[cfg(target_os = "macos")]
const NONBLOCK: c_int = 0x4;
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
    let raw = name(path.as_os_str())?;
    opened(unsafe { open(raw.as_ptr(), RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC) })
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
    let (parent, leaf) = parent(root, relative)?;
    opened(unsafe {
        openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC,
        )
    })
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
        let file = opened(unsafe {
            openat(
                dir.as_raw_fd(),
                leaf.as_ptr(),
                RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC,
            )
        })?;
        let meta = file.metadata()?;
        if !meta.is_file() && !meta.is_dir() {
            return Err(LocalError::UnsafeEntry);
        }
        let name = std::ffi::OsStr::from_bytes(bytes);
        entries.push(LocalEntry {
            name: name.to_string_lossy().into_owned(),
            is_directory: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Metadata from a file descriptor opened relative to the pinned parent.
/// O_NOFOLLOW rejects a final symlink, including one swapped in during lookup.
/// O_NONBLOCK avoids hanging on FIFO/device-like entries.
pub fn stat(root: &File, target: &Path) -> Result<LocalEntry> {
    let (directory, name) = parent(root, target)?;
    let entry = opened(unsafe {
        openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC,
        )
    })?;
    let metadata = entry.metadata()?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(LocalError::UnsafeEntry);
    }
    let display_name = target
        .file_name()
        .ok_or(LocalError::InvalidPath)?
        .to_string_lossy()
        .into_owned();
    Ok(LocalEntry {
        name: display_name,
        is_directory: metadata.is_dir(),
        size: metadata.len(),
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

/// Rename a regular file inside the scoped tree. The destination must not exist.
/// Directory moves, replacements and destructive deletion are deliberately excluded.
pub fn move_new(root: &File, source: &Path, destination: &Path) -> Result<()> {
    let (source_dir, source_name) = parent(root, source)?;
    let (dest_dir, dest_name) = parent(root, destination)?;
    let source_file = opened(unsafe {
        openat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC,
        )
    })?;
    if !source_file.metadata()?.is_file() {
        return Err(LocalError::UnsafeEntry);
    }
    // Link-first gives atomic no-overwrite semantics on the same filesystem.
    // If unlink fails, two names reference the same data; report UNKNOWN via Io
    // rather than retrying an unsafe move.
    let linked = unsafe {
        linkat(
            source_dir.as_raw_fd(),
            source_name.as_ptr(),
            dest_dir.as_raw_fd(),
            dest_name.as_ptr(),
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
    dest_dir.sync_all()?;
    let removed = unsafe { unlinkat(source_dir.as_raw_fd(), source_name.as_ptr(), 0) };
    if removed != 0 {
        return Err(LocalError::Io(io::Error::last_os_error()));
    }
    source_dir.sync_all()?;
    Ok(())
}

/// Creates one directory without following symlinks or replacing existing entries.
pub fn create_directory(root: &File, target: &Path) -> Result<()> {
    let (directory, destination) = parent(root, target)?;
    let result = unsafe { mkdirat(directory.as_raw_fd(), destination.as_ptr(), 0o700) };
    if result != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::AlreadyExists {
            Err(LocalError::AlreadyExists)
        } else {
            Err(LocalError::Io(error))
        };
    }
    directory.sync_all()?;
    Ok(())
}

pub fn create<R: Read>(root: &File, target: &Path, input: &mut R, serial: u64) -> Result<u64> {
    let (directory, destination) = parent(root, target)?;
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
    let result = (|| -> Result<u64> {
        let bytes = io::copy(input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
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
        directory.sync_all()?;
        Ok(bytes)
    })();
    unsafe {
        unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0);
    }
    result
}
