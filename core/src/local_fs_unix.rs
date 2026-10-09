//! Descriptor-relative, symlink-resistant non-overwriting file creation.
//! Unix only. Every intermediate directory is opened with O_NOFOLLOW.
use super::{LocalEntry, LocalError, Result};
use std::ffi::{c_char, c_int, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
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
        return Err(LocalError::Io(io::Error::last_os_error()));
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
