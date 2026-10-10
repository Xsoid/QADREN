//! Local filesystem adapter, scoped to one existing canonical directory.
//! Initial safe slice: metadata and non-overwriting atomic creation only.
//! Content reads on macOS are intentionally not exposed until iCloud placeholder
//! detection and per-transfer consent can be enforced.
use crate::capabilities::{CapabilityDeclaration, MetadataConfidence, Operations, Platform};
#[cfg(any(not(unix), test))]
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(any(not(unix), test))]
use std::path::PathBuf;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
#[path = "local_fs_unix.rs"]
mod secure;

#[derive(Debug)]
pub enum LocalError {
    InvalidPath,
    UnsafeEntry,
    NotDirectory,
    AlreadyExists,
    /// Destination may already exist after a failed durability/cleanup step.
    /// Caller must reconcile state before any retry.
    OutcomeUnknown(io::Error),
    Io(io::Error),
}
impl From<io::Error> for LocalError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
pub type Result<T> = std::result::Result<T, LocalError>;

/// Caller-provided decision for this exact content-reading operation.
/// Never infer consent from a successful metadata query or a previous job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentReadConsent {
    Denied,
    GrantedForThisOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
}

/// Identity for detecting renamed or substituted directory entries.
/// Identifiers are meaningful only within the same live filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalFileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
}

/// Metadata-only indication of local content residency.
/// NotDatalessFlagged is NOT authorization to read or a hydration guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalContentResidency {
    DatalessPlaceholder,
    NotDatalessFlagged,
    Unknown,
}

/// Metadata-only file record for a preflight inspection.
/// Identity and residency are observations, never content-read permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalContentInspection {
    pub identity: LocalFileIdentity,
    pub residency: LocalContentResidency,
    /// Modification and status-change timestamps (seconds, nanoseconds).
    /// Observational: these do not guarantee content immutability.
    pub change_stamp: (i64, i64, i64, i64),
}

/// Filesystem capacity reported by the OS for the pinned local root.
/// This is informational: other processes may consume space immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalStorageSpace {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[cfg(target_os = "linux")]
fn validate_copy_source(
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
    bytes: u64,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    // Compare the opened inode, not the source pathname. Nanosecond mtime
    // and ctime catch more same-length mutations than modified() alone.
    // This remains best-effort detection, not an immutable snapshot.
    if bytes != before.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(LocalError::OutcomeUnknown(io::Error::other(
            "source metadata changed during local copy",
        )));
    }
    Ok(())
}

/// Reject a short or oversized stream before the atomic writer publishes it.
struct ExactSizeReader<'a, R: io::Read> {
    inner: &'a mut R,
    remaining: u64,
}

impl<R: io::Read> io::Read for ExactSizeReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut extra = [0u8; 1];
            return match self.inner.read(&mut extra)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source exceeds declared content length",
                )),
            };
        }
        let max = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let limit = output.len().min(max);
        let read = self.inner.read(&mut output[..limit])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "source is shorter than declared content length",
            ));
        }
        self.remaining -= read as u64;
        Ok(read)
    }
}

pub struct LocalFilesystem {
    #[cfg(not(unix))]
    root: PathBuf,
    #[cfg(unix)]
    root_fd: File,
}

impl LocalFilesystem {
    /// Describe only implemented operations; successful opening establishes
    /// the local root prerequisite, not consent for any content transfer.
    pub fn capabilities(&self) -> CapabilityDeclaration {
        let _ = self;
        CapabilityDeclaration {
            supported_platforms: vec![
                #[cfg(target_os = "macos")]
                Platform::MacOS,
                #[cfg(target_os = "linux")]
                Platform::Linux,
            ],
            operations: Operations {
                list_metadata: cfg!(unix),
                read_content: cfg!(target_os = "linux"),
                write_content: cfg!(unix),
                streaming: cfg!(unix),
                create_directory: cfg!(unix),
                ..Operations::default()
            },
            prerequisites_verified: cfg!(unix),
            metadata_confidence: MetadataConfidence::LocallyVerified,
        }
    }
    /// Detect identical or nested local roots using pinned directory inodes.
    /// Cross-root jobs must reject overlap unless specifically designed for it.
    pub fn overlaps_root(&self, other: &Self) -> Result<bool> {
        #[cfg(unix)]
        {
            secure::roots_overlap(&self.root_fd, &other.root_fd)
        }
        #[cfg(not(unix))]
        {
            let _ = other;
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Return POSIX mode bits without reading content. This is advisory:
    /// ACLs, effective user identity and filesystem policy can still deny access.
    pub fn mode_bits(&self, path: &Path) -> Result<u32> {
        Self::relative(path)?;
        #[cfg(unix)]
        {
            secure::file_mode(&self.root_fd, path)
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Query filesystem capacity without reading any file contents.
    /// This must not be interpreted as a reservation of free space.
    pub fn storage_space(&self) -> Result<LocalStorageSpace> {
        #[cfg(unix)]
        {
            let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            let status = unsafe { libc::fstatvfs(self.root_fd.as_raw_fd(), info.as_mut_ptr()) };
            if status != 0 {
                return Err(LocalError::Io(io::Error::last_os_error()));
            }
            let info = unsafe { info.assume_init() };
            let block_size = info.f_frsize as u128;
            let total = (info.f_blocks as u128).saturating_mul(block_size);
            let available = (info.f_bavail as u128).saturating_mul(block_size);
            Ok(LocalStorageSpace {
                total_bytes: u64::try_from(total).unwrap_or(u64::MAX),
                available_bytes: u64::try_from(available).unwrap_or(u64::MAX),
            })
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Enforce a conservative capacity preflight before starting a transfer.
    /// This is not a space reservation; the write path must still handle ENOSPC.
    pub fn require_free_space(&self, incoming_bytes: u64, reserve_bytes: u64) -> Result<()> {
        let required = incoming_bytes
            .checked_add(reserve_bytes)
            .ok_or(LocalError::UnsafeEntry)?;
        let space = self.storage_space()?;
        if space.available_bytes < required {
            return Err(LocalError::Io(io::Error::new(
                io::ErrorKind::StorageFull,
                "insufficient free space for requested transfer and reserve",
            )));
        }
        Ok(())
    }

    pub fn open(root: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            // Pin the requested directory immediately. Checking a pathname,
            // canonicalizing it and opening it later leaves a root-swap race.
            let root_fd = secure::root(root)?;
            if !root_fd.metadata()?.is_dir() {
                return Err(LocalError::NotDirectory);
            }
            Ok(Self { root_fd })
        }
        #[cfg(not(unix))]
        {
            let meta = fs::symlink_metadata(root)?;
            if meta.file_type().is_symlink() {
                return Err(LocalError::UnsafeEntry);
            }
            if !meta.is_dir() {
                return Err(LocalError::NotDirectory);
            }
            let canonical = fs::canonicalize(root)?;
            Ok(Self { root: canonical })
        }
    }

    fn relative(path: &Path) -> Result<()> {
        if path.as_os_str().is_empty() {
            return Err(LocalError::InvalidPath);
        }
        if !path.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(LocalError::InvalidPath);
        }
        Ok(())
    }

    // No symlink traversal, including intermediate components.
    #[cfg(not(unix))]
    fn checked(&self, path: &Path) -> Result<PathBuf> {
        Self::relative(path)?;
        let mut current = self.root.clone();
        for component in path.components() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => return Err(LocalError::UnsafeEntry),
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(LocalError::Io(err)),
            }
        }
        Ok(current)
    }

    pub fn stat(&self, path: &Path) -> Result<LocalEntry> {
        Self::relative(path)?;
        #[cfg(unix)]
        {
            secure::stat(&self.root_fd, path)
        }
        #[cfg(not(unix))]
        {
            let path = self.checked(path)?;
            let m = fs::symlink_metadata(&path)?;
            if m.file_type().is_symlink() {
                return Err(LocalError::UnsafeEntry);
            }
            Ok(LocalEntry {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                is_directory: m.is_dir(),
                size: m.len(),
            })
        }
    }

    pub fn list(&self, path: &Path) -> Result<Vec<LocalEntry>> {
        if !path.as_os_str().is_empty() {
            Self::relative(path)?;
        }
        #[cfg(unix)]
        {
            secure::list(&self.root_fd, path)
        }
        #[cfg(not(unix))]
        {
            let dir = if path.as_os_str().is_empty() {
                self.root.clone()
            } else {
                self.checked(path)?
            };
            if !fs::symlink_metadata(&dir)?.is_dir() {
                return Err(LocalError::NotDirectory);
            }
            let mut items = Vec::new();
            for item in fs::read_dir(dir)? {
                let item = item?;
                let meta = fs::symlink_metadata(item.path())?;
                if meta.file_type().is_symlink() {
                    return Err(LocalError::UnsafeEntry);
                }
                items.push(LocalEntry {
                    name: item.file_name().to_string_lossy().into_owned(),
                    is_directory: meta.is_dir(),
                    size: meta.len(),
                });
            }
            items.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(items)
        }
    }

    /// Traverse a local subtree using metadata only, with a hard entry limit.
    /// Paths are relative to the adapter root. No file content is opened.
    /// A symlink or unsafe entry anywhere in the subtree fails closed.
    pub fn list_tree(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Result<Vec<(std::path::PathBuf, LocalEntry)>> {
        if !path.as_os_str().is_empty() {
            Self::relative(path)?;
        }
        let mut pending = vec![(path.to_path_buf(), 0usize)];
        let mut result = Vec::new();
        // An inode can be reached via more than one directory entry on some
        // filesystems. Never revisit it and accidentally scan a cycle.
        #[cfg(unix)]
        let mut visited = std::collections::HashSet::new();
        // A malicious or unexpectedly deep tree should never trigger
        // unbounded traversal work even if the entry limit is very high.
        const MAX_DIRECTORY_DEPTH: usize = 256;
        while let Some((directory, depth)) = pending.pop() {
            #[cfg(unix)]
            {
                let identity = secure::directory_identity(&self.root_fd, &directory)?;
                if !visited.insert(identity) {
                    return Err(LocalError::UnsafeEntry);
                }
            }
            let entries = self.list(&directory)?;
            for entry in entries {
                if result.len() >= max_entries {
                    return Err(LocalError::UnsafeEntry);
                }
                let relative = directory.join(&entry.name);
                Self::relative(&relative)?;
                if entry.is_directory {
                    let next_depth = depth.checked_add(1).ok_or(LocalError::UnsafeEntry)?;
                    if next_depth > MAX_DIRECTORY_DEPTH {
                        return Err(LocalError::UnsafeEntry);
                    }
                    pending.push((relative.clone(), next_depth));
                }
                result.push((relative, entry));
            }
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }

    /// Stable lexical pagination over a snapshot of directory metadata.
    /// Callers must rescan if the directory changes between page requests.
    pub fn list_page(&self, path: &Path, offset: usize, limit: usize) -> Result<Vec<LocalEntry>> {
        if !path.as_os_str().is_empty() {
            Self::relative(path)?;
        }
        // Even an empty page must validate the directory and its confinement.
        // Otherwise callers can mistake a missing or unsafe path for success.
        let entries = self.list(path)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        Ok(entries.into_iter().skip(offset).take(limit).collect())
    }

    /// Stream content only after explicit per-operation authorization.
    /// macOS remains disabled until trustworthy placeholder detection exists.
    pub fn read_to_writer<W: io::Write>(
        &self,
        path: &Path,
        output: &mut W,
        consent: ContentReadConsent,
    ) -> Result<u64> {
        Self::relative(path)?;
        if consent != ContentReadConsent::GrantedForThisOperation {
            return Err(LocalError::UnsafeEntry);
        }
        #[cfg(unix)]
        {
            let mut input = secure::open_content(&self.root_fd, path)?;
            #[cfg(target_os = "linux")]
            let before = input.metadata()?;
            // Streaming may have already written to the destination when it
            // fails. Never classify that as safely retryable without inspection.
            let bytes = io::copy(&mut input, output).map_err(LocalError::OutcomeUnknown)?;
            #[cfg(target_os = "linux")]
            {
                let after = input.metadata().map_err(LocalError::OutcomeUnknown)?;
                validate_copy_source(&before, &after, bytes)?;
            }
            Ok(bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = output;
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Copy a local regular file to a new destination without overwriting.
    /// Linux uses the pinned source descriptor and atomic no-overwrite publish.
    /// On macOS this fails closed: opening a File Provider placeholder could
    /// materialize cloud content even when an operation has user consent.
    pub fn copy_file_new(
        &self,
        from: &Path,
        to: &Path,
        consent: ContentReadConsent,
    ) -> Result<u64> {
        Self::relative(from)?;
        Self::relative(to)?;
        if consent != ContentReadConsent::GrantedForThisOperation {
            return Err(LocalError::UnsafeEntry);
        }
        #[cfg(target_os = "linux")]
        {
            let mut source = secure::open_content(&self.root_fd, from)?;
            let before = source.metadata()?;
            let bytes = self.create_from_reader(to, &mut source)?;
            let after = source.metadata().map_err(LocalError::OutcomeUnknown)?;
            // This detects a source changing during transfer, but is not a
            // transactional snapshot: a same-size rewrite may evade metadata.
            // The destination may exist, so do not retry automatically.
            validate_copy_source(&before, &after, bytes)?;
            Ok(bytes)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Linux-only safe-scope copy with free-space reserve and exact-length
    /// publication. macOS stays closed to avoid File Provider hydration.
    pub fn copy_file_new_with_reserve(
        &self,
        from: &Path,
        to: &Path,
        reserve_bytes: u64,
        consent: ContentReadConsent,
    ) -> Result<u64> {
        Self::relative(from)?;
        Self::relative(to)?;
        if consent != ContentReadConsent::GrantedForThisOperation {
            return Err(LocalError::UnsafeEntry);
        }
        #[cfg(target_os = "linux")]
        {
            let mut source = secure::open_content(&self.root_fd, from)?;
            let before = source.metadata()?;
            let bytes =
                self.create_with_space_reserve(to, &mut source, before.len(), reserve_bytes)?;
            let after = source.metadata().map_err(LocalError::OutcomeUnknown)?;
            validate_copy_source(&before, &after, bytes)?;
            Ok(bytes)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = reserve_bytes;
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Read-only preview for a proposed no-overwrite move. This does not
    /// authorize execution: a different process may change either entry
    /// immediately after inspection, so move_file_new stays fail-closed.
    pub fn preview_move_new(&self, from: &Path, to: &Path) -> Result<LocalEntry> {
        Self::relative(from)?;
        Self::relative(to)?;
        if from == to {
            return Err(LocalError::AlreadyExists);
        }
        let source = self.stat(from)?;
        if source.is_directory {
            // Recursive directory moves need separate cycle checks.
            return Err(LocalError::UnsafeEntry);
        }
        match self.stat(to) {
            Ok(_) => Err(LocalError::AlreadyExists),
            Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => {
                // Also verify the destination parent exists and is a real
                // directory, without opening file contents.
                let parent = to.parent().ok_or(LocalError::InvalidPath)?;
                if !parent.as_os_str().is_empty() {
                    let entry = self.stat(parent)?;
                    if !entry.is_directory {
                        return Err(LocalError::NotDirectory);
                    }
                }
                #[cfg(unix)]
                {
                    let leaf = to
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(LocalError::UnsafeEntry)?;
                    if secure::has_case_collision(&self.root_fd, parent, leaf)? {
                        return Err(LocalError::AlreadyExists);
                    }
                }
                Ok(source)
            }
            Err(error) => Err(error),
        }
    }

    /// Return the stable inode identity of a local file for reconciliation.
    /// Does not read file contents. Unix only; not an authorization to move.
    pub fn file_identity(&self, path: &Path) -> Result<LocalFileIdentity> {
        Self::relative(path)?;
        #[cfg(unix)]
        {
            secure::file_identity(&self.root_fd, path)
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Query content-residency hints without opening content. macOS reports
    /// SF_DATALESS; other OSes return Unknown rather than assuming availability.
    /// Read permission remains independent of this observation.
    pub fn content_residency(&self, path: &Path) -> Result<LocalContentResidency> {
        Self::relative(path)?;
        #[cfg(target_os = "macos")]
        {
            if secure::is_dataless(&self.root_fd, path)? {
                Ok(LocalContentResidency::DatalessPlaceholder)
            } else {
                Ok(LocalContentResidency::NotDatalessFlagged)
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = self.file_identity(path)?;
            Ok(LocalContentResidency::Unknown)
        }
    }

    /// Inspect identity and residency without opening content. An inode
    /// replacement between the two queries is reported as unsafe.
    pub fn inspect_content(&self, path: &Path) -> Result<LocalContentInspection> {
        Self::relative(path)?;
        let before = self.file_identity(path)?;
        #[cfg(unix)]
        let prior_stamp = secure::file_change_stamp(&self.root_fd, path)?;
        let residency = self.content_residency(path)?;
        #[cfg(unix)]
        let change_stamp = secure::file_change_stamp(&self.root_fd, path)?;
        let after = self.file_identity(path)?;
        #[cfg(not(unix))]
        let change_stamp = (0, 0, 0, 0);
        if before != after {
            return Err(LocalError::UnsafeEntry);
        }
        #[cfg(unix)]
        if prior_stamp != change_stamp {
            return Err(LocalError::UnsafeEntry);
        }
        Ok(LocalContentInspection {
            identity: after,
            residency,
            change_stamp,
        })
    }

    /// Inspect several local files in one metadata-only batch. A failure
    /// aborts the batch instead of returning an incomplete success list.
    pub fn inspect_content_batch(&self, paths: &[&Path]) -> Result<Vec<LocalContentInspection>> {
        let mut seen_paths = std::collections::HashSet::new();
        let mut seen_inodes = std::collections::HashSet::new();
        let mut inspected = Vec::with_capacity(paths.len());
        for path in paths {
            Self::relative(path)?;
            // Reject duplicate requests even before inspecting their metadata.
            // Fold only ASCII: non-ASCII filesystem case rules are platform-
            // specific, so inode identity below remains the decisive guard.
            let folded = path.to_string_lossy().to_ascii_lowercase();
            if !seen_paths.insert(folded) {
                return Err(LocalError::AlreadyExists);
            }
            let item = self.inspect_content(path)?;
            if !seen_inodes.insert((item.identity.device, item.identity.inode)) {
                return Err(LocalError::AlreadyExists);
            }
            inspected.push(item);
        }
        Ok(inspected)
    }

    /// Recheck a previously observed file without reading its content.
    /// This is a point-in-time observation, never authorization to hydrate.
    pub fn revalidate_content_inspection(
        &self,
        path: &Path,
        expected: LocalContentInspection,
    ) -> Result<bool> {
        match self.inspect_content(path) {
            Ok(current) => Ok(current == expected),
            Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Revalidate a whole batch against earlier observations. An index
    /// corresponds to the same index in paths; mismatched lengths fail.
    /// Each item is checked again and no file content is opened.
    pub fn revalidate_content_batch(
        &self,
        paths: &[&Path],
        expected: &[LocalContentInspection],
    ) -> Result<bool> {
        if paths.len() != expected.len() {
            return Err(LocalError::InvalidPath);
        }
        // Inspect every entry even after a mismatch; failures must propagate.
        let mut all_match = true;
        for (path, previous) in paths.iter().zip(expected.iter()) {
            if !self.revalidate_content_inspection(path, *previous)? {
                all_match = false;
            }
        }
        Ok(all_match)
    }

    /// Confirm that a previewed regular-file inode is still at its original
    /// path and that the destination remains absent. This is observational:
    /// no filesystem mutation is authorized by this method.
    pub fn revalidate_move_new(
        &self,
        from: &Path,
        to: &Path,
        expected: LocalFileIdentity,
    ) -> Result<()> {
        Self::relative(from)?;
        Self::relative(to)?;
        if from == to {
            return Err(LocalError::AlreadyExists);
        }
        if self.file_identity(from)? != expected {
            return Err(LocalError::UnsafeEntry);
        }
        match self.preview_move_new(from, to) {
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Atomically relocate a regular file only when the destination is absent.
    /// The operation never overwrites another entry. A concurrent source swap
    /// may still lead to OutcomeUnknown; callers MUST reconcile rather than retry.
    pub fn rename_file_new(&self, from: &Path, to: &Path) -> Result<()> {
        Self::relative(from)?;
        Self::relative(to)?;
        if from == to {
            return Err(LocalError::AlreadyExists);
        }
        #[cfg(unix)]
        {
            self.preview_move_new(from, to)?;
            secure::rename_file_noreplace(&self.root_fd, from, to)
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Checked rename for a previously previewed source. Refuses an already
    /// stale preview, but concurrent writers may still replace the source
    /// after the check. Not exposed to automatic jobs.
    pub fn rename_file_new_checked(
        &self,
        from: &Path,
        to: &Path,
        expected: LocalFileIdentity,
    ) -> Result<()> {
        self.revalidate_move_new(from, to, expected)?;
        self.rename_file_new(from, to)
    }

    /// Recheck the source inode and destination vacancy before a proposed
    /// move. This does not authorize deletion or close the rename race.
    pub fn preview_move_identity(&self, from: &Path, to: &Path) -> Result<LocalFileIdentity> {
        self.preview_move_new(from, to)?;
        self.file_identity(from)
    }

    /// Observe the outcome of an ambiguous rename without changing files.
    /// Returns true only when the source name is missing and destination has
    /// the expected inode. This is not proof against subsequent mutations.
    pub fn reconcile_renamed_file(
        &self,
        from: &Path,
        to: &Path,
        expected: LocalFileIdentity,
    ) -> Result<bool> {
        Self::relative(from)?;
        Self::relative(to)?;
        if from == to {
            return Err(LocalError::AlreadyExists);
        }
        let current = match self.file_identity(to) {
            Ok(identity) => identity,
            Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if current != expected {
            return Ok(false);
        }
        match self.file_identity(from) {
            Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Ok(_) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Non-destructive hard-link staging for a future move. The source stays
    /// untouched; do not confuse this with a completed move or delete.
    pub fn stage_move_link_new(&self, from: &Path, to: &Path) -> Result<()> {
        Self::relative(from)?;
        Self::relative(to)?;
        if from == to {
            return Err(LocalError::AlreadyExists);
        }
        #[cfg(unix)]
        {
            let source = self.file_identity(from)?;
            let parent = to.parent().ok_or(LocalError::InvalidPath)?;
            let leaf = to
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(LocalError::UnsafeEntry)?;
            if secure::has_case_collision(&self.root_fd, parent, leaf)? {
                return Err(LocalError::AlreadyExists);
            }
            secure::link_file_new(&self.root_fd, from, to)?;
            match self.file_identity(to) {
                Ok(found) if found.device == source.device && found.inode == source.inode => Ok(()),
                _ => Err(LocalError::OutcomeUnknown(io::Error::other(
                    "staged hard link requires reconciliation",
                ))),
            }
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Inspect an ambiguous hard-link publication without changing any files.
    /// A matching inode indicates both names currently reference the same file;
    /// it does not prove a completed move or permit source deletion.
    pub fn reconcile_staged_link(
        &self,
        from: &Path,
        to: &Path,
        expected: LocalFileIdentity,
    ) -> Result<bool> {
        Self::relative(from)?;
        Self::relative(to)?;
        #[cfg(unix)]
        {
            // A missing name is a negative reconciliation result, not an
            // exceptional I/O failure. Never recreate either name here.
            let source = match self.file_identity(from) {
                Ok(identity) => identity,
                Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            };
            let staged = match self.file_identity(to) {
                Ok(identity) => identity,
                Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            };
            Ok(source == expected && staged == expected)
        }
        #[cfg(not(unix))]
        {
            let _ = expected;
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Moving by pathname cannot guarantee source identity under concurrent mutation.
    /// Reject until an identity-preserving platform implementation is available.
    pub fn move_file_new(&self, from: &Path, to: &Path) -> Result<()> {
        Self::relative(from)?;
        Self::relative(to)?;
        Err(LocalError::UnsafeEntry)
    }

    /// Creates one child directory; fails closed on unsupported platforms.
    pub fn create_directory(&self, path: &Path) -> Result<()> {
        Self::relative(path)?;
        #[cfg(unix)]
        {
            secure::create_directory(&self.root_fd, path)
        }
        #[cfg(not(unix))]
        {
            Err(LocalError::UnsafeEntry)
        }
    }

    /// Create missing directories within this root, one component at a time.
    /// Existing real directories are accepted; symlinks are never followed.
    /// If any directory was created before an error, return OutcomeUnknown:
    /// the caller must inspect partial effects rather than blindly retry.
    pub fn create_directory_tree(&self, path: &Path) -> Result<usize> {
        Self::relative(path)?;
        let mut prefix = std::path::PathBuf::new();
        let mut created = 0;
        for component in path.components() {
            prefix.push(component.as_os_str());
            match self.stat(&prefix) {
                Ok(entry) if entry.is_directory => {
                    // On case-insensitive volumes stat may resolve a different
                    // spelling. Do not silently accept that alias.
                    let parent = prefix.parent().ok_or(LocalError::InvalidPath)?;
                    let expected = prefix
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(LocalError::UnsafeEntry)?;
                    #[cfg(unix)]
                    let exact = secure::has_exact_directory(&self.root_fd, parent, expected)?;
                    #[cfg(not(unix))]
                    let exact = self.list(parent)?.iter().any(|item| item.name == expected);
                    if !exact {
                        return Err(LocalError::AlreadyExists);
                    }
                    continue;
                }
                Ok(_) => {
                    if created > 0 {
                        return Err(LocalError::OutcomeUnknown(io::Error::other(
                            "partial directory tree exists before a non-directory",
                        )));
                    }
                    return Err(LocalError::NotDirectory);
                }
                Err(LocalError::Io(ref err)) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    if created > 0 {
                        return Err(LocalError::OutcomeUnknown(io::Error::other(
                            "directory tree may be partially created",
                        )));
                    }
                    return Err(err);
                }
            }
            match self.create_directory(&prefix) {
                Ok(()) => created += 1,
                // Another writer may have created this directory after stat.
                // Accept only an actual directory within the pinned root;
                // never treat an unexpected file or symlink as success.
                Err(LocalError::AlreadyExists) => {
                    let parent = prefix.parent().ok_or(LocalError::InvalidPath)?;
                    let expected = prefix
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(LocalError::UnsafeEntry)?;
                    #[cfg(unix)]
                    let same_spelling =
                        secure::has_exact_directory(&self.root_fd, parent, expected)?;
                    #[cfg(not(unix))]
                    let same_spelling = self
                        .list(parent)?
                        .iter()
                        .any(|item| item.name == expected && item.is_directory);
                    if !same_spelling
                        || !matches!(self.stat(&prefix), Ok(entry) if entry.is_directory)
                    {
                        if created > 0 {
                            return Err(LocalError::OutcomeUnknown(io::Error::other(
                                "directory tree partially created after concurrent collision",
                            )));
                        }
                        return Err(LocalError::AlreadyExists);
                    }
                }
                Err(err) => {
                    if created > 0 {
                        return Err(LocalError::OutcomeUnknown(io::Error::other(
                            "directory tree may be partially created",
                        )));
                    }
                    return Err(err);
                }
            }
        }
        Ok(created)
    }

    /// Create a file only if the currently reported free space can cover the
    /// expected payload and the caller's reserve. Capacity is not reserved;
    /// the atomic writer still handles mid-transfer ENOSPC.
    pub fn create_with_space_reserve<R: io::Read>(
        &self,
        target: &Path,
        input: &mut R,
        expected_bytes: u64,
        reserve_bytes: u64,
    ) -> Result<u64> {
        Self::relative(target)?;
        self.require_free_space(expected_bytes, reserve_bytes)?;
        let mut exact = ExactSizeReader {
            inner: input,
            remaining: expected_bytes,
        };
        self.create_from_reader(target, &mut exact)
    }

    /// Conservative case-collision guard for user-visible file creation.
    /// Checks the destination directory before invoking the atomic no-overwrite
    /// writer. This does not replace the filesystem's final collision check.
    pub fn create_from_reader_case_guarded<R: io::Read>(
        &self,
        target: &Path,
        input: &mut R,
    ) -> Result<u64> {
        self.create_from_reader(target, input)
    }

    /// Create a new file through directory descriptors, without following
    /// symlinks or replacing an existing target. Unsupported platforms fail closed.
    /// Reading cloud-backed local placeholders remains prohibited until guarded.
    pub fn create_from_reader<R: io::Read>(&self, target: &Path, input: &mut R) -> Result<u64> {
        Self::relative(target)?;
        #[cfg(unix)]
        {
            // Every write entry point must reject case aliases before reading
            // data. The atomic linkat check remains authoritative for races.
            let parent = target.parent().ok_or(LocalError::InvalidPath)?;
            let leaf = target
                .file_name()
                .ok_or(LocalError::InvalidPath)?
                .to_str()
                .ok_or(LocalError::UnsafeEntry)?
                .to_lowercase();
            if secure::has_case_collision(&self.root_fd, parent, &leaf)? {
                return Err(LocalError::AlreadyExists);
            }
        }
        #[cfg(unix)]
        {
            let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            secure::create(&self.root_fd, target, input, serial)
        }
        #[cfg(not(unix))]
        {
            let _ = (target, input);
            Err(LocalError::UnsafeEntry)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    fn sandbox() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        // macOS commonly exposes /var as a symlink to /private/var.
        // Exercise confinement using a physical sandbox root, not that alias.
        let temp_root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let p = temp_root.join(format!(
            "qadren-local-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        p
    }

    // macOS rejects invalid UTF-8 path bytes with EILSEQ before listing.
    #[cfg(target_os = "linux")]
    #[test]
    fn lossy_name_collisions_fail_instead_of_conflating_distinct_files() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let root = sandbox();
        let first = OsStr::from_bytes(b"invalid-\xff");
        let second = OsStr::from_bytes(b"invalid-\xfe");
        fs::write(root.join(first), b"one").unwrap();
        fs::write(root.join(second), b"two").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.list(Path::new("")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.list_page(Path::new(""), 0, 10),
            Err(LocalError::UnsafeEntry)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_capabilities_do_not_advertise_unsafe_operations() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let capabilities = adapter.capabilities();
        assert!(capabilities.operations.list_metadata);
        assert!(!capabilities.operations.rename);
        assert!(!capabilities.operations.delete);
        assert_eq!(
            capabilities.operations.read_content,
            cfg!(target_os = "linux")
        );
        assert_eq!(capabilities.operations.write_content, cfg!(unix));
        assert_eq!(capabilities.operations.create_directory, cfg!(unix));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zero_limit_pagination_does_not_bypass_path_validation() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.list_page(Path::new("../escape"), 0, 0),
            Err(LocalError::InvalidPath)
        ));
        assert!(matches!(
            adapter.list_page(Path::new("/absolute"), 0, 0),
            Err(LocalError::InvalidPath)
        ));
        assert!(adapter.list_page(Path::new(""), 0, 0).unwrap().is_empty());
        assert!(adapter.list_page(Path::new("missing"), 0, 0).is_err());
        fs::write(root.join("file.txt"), b"file").unwrap();
        assert!(adapter.list_page(Path::new("file.txt"), 0, 0).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("file.txt", root.join("alias")).unwrap();
            assert!(adapter.list_page(Path::new("alias"), 0, 0).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unverified_move_is_rejected_without_changes() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        fs::write(root.join("from.txt"), b"data").unwrap();
        fs::write(root.join("occupied.txt"), b"existing").unwrap();
        assert!(matches!(
            adapter.move_file_new(Path::new("from.txt"), Path::new("occupied.txt")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.move_file_new(Path::new("from.txt"), Path::new("to.txt")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(root.join("from.txt").exists());
        assert!(!root.join("to.txt").exists());
        assert_eq!(fs::read(root.join("occupied.txt")).unwrap(), b"existing");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn root_listing_uses_pinned_directory_and_stable_pagination() {
        let root = sandbox();
        fs::write(root.join("z.txt"), b"last").unwrap();
        fs::write(root.join("a.txt"), b"first").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let entries = adapter.list(Path::new("")).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[1].name, "z.txt");
        assert_eq!(
            adapter.list_page(Path::new(""), 1, 1).unwrap(),
            vec![entries[1].clone()]
        );
        assert!(matches!(
            adapter.stat(Path::new("")),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn metadata_rejects_symlinks_without_opening_contents() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        fs::create_dir(root.join("entries")).unwrap();
        fs::write(root.join("entries").join("file"), b"metadata").unwrap();
        symlink("file", root.join("entries").join("alias")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stat(Path::new("entries/alias")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.list(Path::new("entries")),
            Err(LocalError::UnsafeEntry)
        ));
        fs::remove_file(root.join("entries").join("alias")).unwrap();
        let entries = adapter.list(Path::new("entries")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 8);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_swap_after_publish_preserves_foreign_file() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let serial = 940004;
        let staging = root.join(format!(".qadren-{}-{serial}.tmp", std::process::id()));
        let moved = root.join("original-staging");
        let result = secure::create_with_post_link_check(
            &adapter.root_fd,
            Path::new("published.bin"),
            &mut Cursor::new(b"original"),
            serial,
            || {
                fs::rename(&staging, &moved)?;
                fs::write(&staging, b"foreign")
            },
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert_eq!(fs::read(root.join("published.bin")).unwrap(), b"original");
        assert_eq!(fs::read(staging).unwrap(), b"foreign");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn destination_swap_after_publish_preserves_foreign_file() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let serial = 940005;
        let destination = root.join("published.bin");
        let displaced = root.join("displaced-published.bin");
        let result = secure::create_with_post_link_check(
            &adapter.root_fd,
            Path::new("published.bin"),
            &mut Cursor::new(b"original"),
            serial,
            || {
                fs::rename(&destination, &displaced)?;
                fs::write(&destination, b"foreign")
            },
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert_eq!(fs::read(&destination).unwrap(), b"foreign");
        assert_eq!(fs::read(&displaced).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_hardlink_injected_during_stream_blocks_publication() {
        struct HardlinkReader {
            staging: PathBuf,
            alias: PathBuf,
            done: bool,
        }
        impl io::Read for HardlinkReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.done {
                    return Ok(0);
                }
                self.done = true;
                fs::hard_link(&self.staging, &self.alias)?;
                buf[0] = b'x';
                Ok(1)
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let serial = 940006;
        let staging = root.join(format!(".qadren-{}-{serial}.tmp", std::process::id()));
        let alias = root.join("foreign-hardlink");
        let result = secure::create_with_post_link_check(
            &adapter.root_fd,
            Path::new("destination.bin"),
            &mut HardlinkReader {
                staging: staging.clone(),
                alias: alias.clone(),
                done: false,
            },
            serial,
            || Ok(()),
        );
        assert!(matches!(result, Err(LocalError::UnsafeEntry)));
        assert!(!root.join("destination.bin").exists());
        assert_eq!(fs::read(&alias).unwrap(), b"x");
        assert_eq!(fs::read(&staging).unwrap(), b"x");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_name_replacement_is_rejected_without_publishing() {
        struct SwapReader {
            root: PathBuf,
            serial: u64,
            done: bool,
        }
        impl io::Read for SwapReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.done {
                    return Ok(0);
                }
                self.done = true;
                let temporary = self.root.join(format!(
                    ".qadren-{}-{}.tmp",
                    std::process::id(),
                    self.serial
                ));
                fs::rename(&temporary, self.root.join("moved-staging")).unwrap();
                fs::write(&temporary, b"foreign contents").unwrap();
                buf[0] = b'x';
                Ok(1)
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let serial = 940003;
        let mut reader = SwapReader {
            root: root.clone(),
            serial,
            done: false,
        };
        assert!(matches!(
            secure::create_with_post_link_check(
                &adapter.root_fd,
                Path::new("destination"),
                &mut reader,
                serial,
                || Ok(())
            ),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("destination").exists());
        assert_eq!(
            fs::read(root.join(format!(".qadren-{}-{serial}.tmp", std::process::id()))).unwrap(),
            b"foreign contents"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_creates_of_same_target_never_overwrite() {
        use std::sync::{Arc, Barrier};
        let root = sandbox();
        let adapter = Arc::new(LocalFilesystem::open(&root).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .map(|payload| {
                let adapter = Arc::clone(&adapter);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    adapter.create_from_reader(Path::new("shared.bin"), &mut Cursor::new(payload))
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(LocalError::AlreadyExists)))
                .count(),
            1
        );
        let bytes = fs::read(root.join("shared.bin")).unwrap();
        assert!(bytes == b"first" || bytes == b"second");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_root_remains_pinned_after_path_is_replaced() {
        let parent = sandbox();
        let original = parent.join("original");
        let moved = parent.join("moved");
        fs::create_dir(&original).unwrap();
        fs::write(original.join("original.txt"), b"first").unwrap();
        let adapter = LocalFilesystem::open(&original).unwrap();
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("replacement.txt"), b"second").unwrap();
        let entries = adapter.list(Path::new("")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "original.txt");
        adapter
            .create_from_reader(Path::new("created.txt"), &mut Cursor::new(b"pinned"))
            .unwrap();
        assert_eq!(fs::read(moved.join("created.txt")).unwrap(), b"pinned");
        assert!(!original.join("created.txt").exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_symlink_in_root_path_is_rejected() {
        use std::os::unix::fs::symlink;
        let parent = sandbox();
        let actual = parent.join("actual");
        fs::create_dir(&actual).unwrap();
        fs::create_dir(actual.join("nested")).unwrap();
        symlink("actual", parent.join("alias")).unwrap();
        assert!(LocalFilesystem::open(&actual.join("nested")).is_ok());
        assert!(matches!(
            LocalFilesystem::open(&parent.join("alias").join("nested")),
            Err(LocalError::UnsafeEntry)
        ));
        fs::remove_dir_all(parent).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_root_is_rejected_before_any_content_access() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let parent = sandbox();
        let alias = parent.join("linked-root");
        symlink(&root, &alias).unwrap();
        assert!(matches!(
            LocalFilesystem::open(&alias),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(LocalFilesystem::open(&root).is_ok());
        fs::remove_dir_all(parent).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn temporary_name_collision_does_not_modify_existing_file() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let serial = 730001_u64;
        let temp = root.join(format!(".qadren-{}-{serial}.tmp", std::process::id()));
        fs::write(&temp, b"another operation").unwrap();
        let result = secure::create_with_post_link_check(
            &adapter.root_fd,
            Path::new("destination.bin"),
            &mut Cursor::new(b"replacement"),
            serial,
            || Ok(()),
        );
        assert!(matches!(
            result,
            Err(LocalError::AlreadyExists) | Err(LocalError::Io(_))
        ));
        assert_eq!(fs::read(&temp).unwrap(), b"another operation");
        assert!(!root.join("destination.bin").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_after_creation_failure_is_unknown_and_preserves_directory() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = secure::create_directory_with_post_mkdir_check(
            &adapter.root_fd,
            Path::new("created-before-failure"),
            || Err(io::Error::other("simulated durability failure")),
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert!(root.join("created-before-failure").is_dir());
        assert!(matches!(
            adapter.create_directory(Path::new("created-before-failure")),
            Err(LocalError::AlreadyExists)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_existing_directory_is_not_treated_as_new_success() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        adapter.create_directory(Path::new("new-folder")).unwrap();
        assert!(matches!(
            adapter.create_directory(Path::new("new-folder")),
            Err(LocalError::AlreadyExists)
        ));
        assert!(root.join("new-folder").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn disk_full_during_stream_stays_before_publication() {
        struct DiskFullAfterFirstChunk(bool);
        impl io::Read for DiskFullAfterFirstChunk {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::from_raw_os_error(libc::ENOSPC));
                }
                self.0 = true;
                buf[0] = b'a';
                Ok(1)
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = adapter.create_from_reader(
            Path::new("disk-full.bin"),
            &mut DiskFullAfterFirstChunk(false),
        );
        assert!(matches!(
            result,
            Err(LocalError::Io(ref e)) if e.raw_os_error() == Some(libc::ENOSPC)
        ));
        assert!(!root.join("disk-full.bin").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn post_publication_failure_is_unknown_and_requires_reconciliation() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = secure::create_with_post_link_check(
            &adapter.root_fd,
            Path::new("published.txt"),
            &mut Cursor::new(b"committed"),
            9001,
            || Err(io::Error::other("simulated sync failure after link")),
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert_eq!(fs::read(root.join("published.txt")).unwrap(), b"committed");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(matches!(
            adapter.create_from_reader(Path::new("published.txt"), &mut Cursor::new(b"retry")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("published.txt")).unwrap(), b"committed");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_partial_stream_removes_temp_and_never_publishes_target() {
        struct PartialThenFailure(bool);
        impl io::Read for PartialThenFailure {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected interrupted stream"));
                }
                self.0 = true;
                buf[..4].copy_from_slice(b"part");
                Ok(4)
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = adapter
            .create_from_reader(Path::new("interrupted.bin"), &mut PartialThenFailure(false));
        assert!(result.is_err());
        assert!(!root.join("interrupted.bin").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn streaming_large_file_and_retry_preserve_committed_contents() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let length = 8 * 1024 * 1024_u64 + 19;
        let bytes = adapter
            .create_from_reader(Path::new("large.bin"), &mut io::repeat(0x5a).take(length))
            .unwrap();
        assert_eq!(bytes, length);
        assert_eq!(adapter.stat(Path::new("large.bin")).unwrap().size, length);
        assert!(matches!(
            adapter.create_from_reader(Path::new("large.bin"), &mut Cursor::new(b"replacement"),),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::metadata(root.join("large.bin")).unwrap().len(), length);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn changed_copy_source_requires_reconciliation() {
        let root = sandbox();
        let source = root.join("mutable.bin");
        fs::write(&source, b"old").unwrap();
        let before = fs::metadata(&source).unwrap();
        fs::write(&source, b"new and longer").unwrap();
        let after = fs::metadata(&source).unwrap();
        assert!(matches!(
            validate_copy_source(&before, &after, 3),
            Err(LocalError::OutcomeUnknown(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn copy_source_verification_rejects_truncated_transfer() {
        let root = sandbox();
        let source = root.join("stable.bin");
        fs::write(&source, b"stable-data").unwrap();
        let metadata = fs::metadata(&source).unwrap();
        assert!(validate_copy_source(&metadata, &metadata, 11).is_ok());
        assert!(matches!(
            validate_copy_source(&metadata, &metadata, 4),
            Err(LocalError::OutcomeUnknown(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_rejects_unsafe_sources_and_destination_paths() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let consent = ContentReadConsent::GrantedForThisOperation;

        assert!(matches!(
            adapter.copy_file_new(
                Path::new("../source.bin"),
                Path::new("escaped.bin"),
                consent,
            ),
            Err(LocalError::InvalidPath)
        ));
        assert!(matches!(
            adapter.copy_file_new(
                Path::new("source.bin"),
                Path::new("../escaped.bin"),
                consent,
            ),
            Err(LocalError::InvalidPath)
        ));
        assert!(!root.join("escaped.bin").exists());
        assert!(matches!(
            adapter.copy_file_new(
                Path::new("missing.bin"),
                Path::new("missing-copy.bin"),
                consent,
            ),
            Err(LocalError::Io(_)) | Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("missing-copy.bin").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("source.bin", root.join("source-link")).unwrap();
            assert!(matches!(
                adapter.copy_file_new(
                    Path::new("source-link"),
                    Path::new("linked-copy.bin"),
                    consent,
                ),
                Err(LocalError::Io(_)) | Err(LocalError::UnsafeEntry)
            ));
            assert!(!root.join("linked-copy.bin").exists());
        }
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_stream_reports_unknown_on_source_mutation() {
        struct MutatingWriter {
            source: PathBuf,
            bytes: Vec<u8>,
            changed: bool,
        }
        impl io::Write for MutatingWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if !self.changed {
                    self.changed = true;
                    fs::write(&self.source, b"changed and longer")?;
                }
                self.bytes.extend_from_slice(buffer);
                Ok(buffer.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = sandbox();
        let source = root.join("source.bin");
        fs::write(&source, b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let mut output = MutatingWriter {
            source: source.clone(),
            bytes: Vec::new(),
            changed: false,
        };
        assert!(matches!(
            adapter.read_to_writer(
                Path::new("source.bin"),
                &mut output,
                ContentReadConsent::GrantedForThisOperation,
            ),
            Err(LocalError::OutcomeUnknown(_))
        ));
        assert!(output.changed);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reserved_creation_rejects_mismatched_length_before_publish() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        for (name, source, length) in [
            ("short.bin", b"abc".as_slice(), 4),
            ("long.bin", b"abc".as_slice(), 2),
        ] {
            let result = adapter.create_with_space_reserve(
                Path::new(name),
                &mut io::Cursor::new(source),
                length,
                0,
            );
            assert!(matches!(result, Err(LocalError::Io(_))));
            assert!(!root.join(name).exists());
        }
        let result = adapter.create_with_space_reserve(
            Path::new("exact.bin"),
            &mut io::Cursor::new(b"abc"),
            3,
            0,
        );
        assert_eq!(result.unwrap(), 3);
        assert_eq!(fs::read(root.join("exact.bin")).unwrap(), b"abc");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reserved_creation_rejects_before_reading_or_publishing() {
        struct PanicReader;
        impl io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("reader must not be called when capacity check fails")
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = adapter.create_with_space_reserve(
            Path::new("must-not-exist"),
            &mut PanicReader,
            u64::MAX,
            1,
        );
        assert!(matches!(result, Err(LocalError::UnsafeEntry)));
        assert!(!root.join("must-not-exist").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn capacity_preflight_rejects_unfulfillable_transfer() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let available = adapter.storage_space().unwrap().available_bytes;
        assert!(adapter.require_free_space(0, 0).is_ok());
        assert!(matches!(
            adapter.require_free_space(u64::MAX, 1),
            Err(LocalError::UnsafeEntry)
        ));
        if available < u64::MAX {
            assert!(matches!(
                adapter.require_free_space(available + 1, 0),
                Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::StorageFull
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn storage_space_is_metadata_only_and_uses_open_root() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let space = adapter.storage_space().unwrap();
        assert!(space.total_bytes > 0);
        assert!(space.available_bytes <= space.total_bytes);
        fs::remove_dir_all(root).unwrap();
        // The pinned descriptor remains valid even after the directory
        // pathname disappears; capacity inspection never opens content.
        let pinned = adapter.storage_space().unwrap();
        assert!(pinned.total_bytes > 0);
    }

    #[cfg(unix)]
    #[test]
    fn directory_tree_creates_missing_parents_without_replacing_entries() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter
                .create_directory_tree(Path::new("one/two/three"))
                .unwrap(),
            3
        );
        assert!(root.join("one/two/three").is_dir());
        assert_eq!(
            adapter
                .create_directory_tree(Path::new("one/two/three"))
                .unwrap(),
            0
        );
        fs::write(root.join("one/file"), b"protected").unwrap();
        assert!(matches!(
            adapter.create_directory_tree(Path::new("one/file/child")),
            Err(LocalError::NotDirectory) | Err(LocalError::UnsafeEntry)
        ));
        assert_eq!(fs::read(root.join("one/file")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_tree_rejects_symlink_and_preserves_external_directory() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let external = sandbox();
        symlink(&external, root.join("redirect")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.create_directory_tree(Path::new("redirect/subdirectory")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!external.join("subdirectory").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_tree_partial_failure_is_unknown_without_rollback() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let too_long = "x".repeat(300);
        let target = format!("first/{too_long}");
        assert!(matches!(
            adapter.create_directory_tree(Path::new(&target)),
            Err(LocalError::OutcomeUnknown(_))
        ));
        assert!(root.join("first").is_dir());
        assert!(matches!(
            adapter.create_directory_tree(Path::new("../escape")),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_streaming_read_is_unknown_after_partial_output() {
        struct PartialWriter {
            accepted: usize,
        }
        impl io::Write for PartialWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.accepted > 0 {
                    return Err(io::Error::other("downstream failed"));
                }
                self.accepted += 1;
                Ok(bytes.len().min(1))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = sandbox();
        fs::write(root.join("source.bin"), b"multiple bytes").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let mut writer = PartialWriter { accepted: 0 };
        assert!(matches!(
            adapter.read_to_writer(
                Path::new("source.bin"),
                &mut writer,
                ContentReadConsent::GrantedForThisOperation
            ),
            Err(LocalError::OutcomeUnknown(_))
        ));
        assert_eq!(writer.accepted, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reserved_copy_keeps_existing_destination_and_mac_content_closed() {
        let root = sandbox();
        fs::write(root.join("origin.bin"), b"original").unwrap();
        fs::write(root.join("occupied.bin"), b"preserved").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.copy_file_new_with_reserve(
                Path::new("origin.bin"),
                Path::new("untouched.bin"),
                0,
                ContentReadConsent::Denied,
            ),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("untouched.bin").exists());
        let result = adapter.copy_file_new_with_reserve(
            Path::new("origin.bin"),
            Path::new("occupied.bin"),
            0,
            ContentReadConsent::GrantedForThisOperation,
        );
        #[cfg(target_os = "linux")]
        assert!(matches!(result, Err(LocalError::AlreadyExists)));
        #[cfg(not(target_os = "linux"))]
        assert!(matches!(result, Err(LocalError::UnsafeEntry)));
        assert_eq!(fs::read(root.join("occupied.bin")).unwrap(), b"preserved");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn file_identity_tracks_inode_without_reading_content() {
        let root = sandbox();
        fs::write(root.join("first.bin"), b"source").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let before = adapter.file_identity(Path::new("first.bin")).unwrap();
        assert_eq!(before.size, 6);
        fs::rename(root.join("first.bin"), root.join("second.bin")).unwrap();
        let after = adapter.file_identity(Path::new("second.bin")).unwrap();
        assert_eq!(before, after);
        assert!(matches!(
            adapter.file_identity(Path::new("../outside")),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn move_revalidation_detects_replaced_source_without_mutation() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let expected = adapter.file_identity(Path::new("source.bin")).unwrap();
        assert!(adapter
            .revalidate_move_new(Path::new("source.bin"), Path::new("target.bin"), expected,)
            .is_ok());
        fs::rename(root.join("source.bin"), root.join("old.bin")).unwrap();
        fs::write(root.join("source.bin"), b"replacement").unwrap();
        assert!(matches!(
            adapter
                .revalidate_move_new(Path::new("source.bin"), Path::new("target.bin"), expected,),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("target.bin").exists());
        assert_eq!(fs::read(root.join("old.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_filename_is_never_silently_replaced() {
        use std::os::unix::ffi::OsStrExt;

        let root = sandbox();
        let invalid = std::ffi::OsStr::from_bytes(b"bad-\xff");
        if let Err(error) = fs::write(root.join(invalid), b"protected") {
            // APFS/macOS rejects malformed UTF-8 names before QADREN can
            // inspect them; Linux filesystems may permit such raw bytes.
            if cfg!(target_os = "macos") && error.raw_os_error() == Some(92) {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("failed to prepare invalid UTF-8 filename: {error}");
        }
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.list(Path::new("")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.stat(Path::new(invalid)),
            Err(LocalError::UnsafeEntry)
        ));
        assert_eq!(fs::read(root.join(invalid)).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_identity_is_stable_for_opened_directory() {
        let root = sandbox();
        fs::create_dir(root.join("nested")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let before = secure::directory_identity(&adapter.root_fd, Path::new("nested")).unwrap();
        fs::rename(root.join("nested"), root.join("renamed")).unwrap();
        let after = secure::directory_identity(&adapter.root_fd, Path::new("renamed")).unwrap();
        assert_eq!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn list_tree_rejects_unsafe_symlink_without_partial_success() {
        let root = sandbox();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/file.txt"), b"data").unwrap();
        std::os::unix::fs::symlink("../nested/file.txt", root.join("nested/alias")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.list_tree(Path::new(""), 100),
            Err(LocalError::UnsafeEntry)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn list_tree_is_metadata_only_bounded_and_nested() {
        let root = sandbox();
        fs::create_dir(root.join("nested")).unwrap();
        fs::create_dir(root.join("nested/deep")).unwrap();
        fs::write(root.join("nested/deep/file.txt"), b"payload").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let tree = adapter.list_tree(Path::new(""), 3).unwrap();
        let paths: Vec<_> = tree.iter().map(|(path, _)| path.as_path()).collect();
        assert_eq!(
            paths,
            vec![
                Path::new("nested"),
                Path::new("nested/deep"),
                Path::new("nested/deep/file.txt"),
            ]
        );
        assert!(matches!(
            adapter.list_tree(Path::new(""), 2),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.list_tree(Path::new("../outside"), 100),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn occupied_destination_is_rejected_before_streaming() {
        struct PanicReader;
        impl io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("existing destination must reject before reading")
            }
        }
        let root = sandbox();
        fs::write(root.join("occupied.bin"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.create_from_reader(Path::new("occupied.bin"), &mut PanicReader),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("occupied.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_revalidation_detects_replacement_and_missing_file() {
        let root = sandbox();
        fs::write(root.join("item.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let path = Path::new("item.bin");
        let observed = adapter.inspect_content(path).unwrap();
        assert!(adapter
            .revalidate_content_inspection(path, observed)
            .unwrap());
        fs::rename(root.join("item.bin"), root.join("retained.bin")).unwrap();
        fs::write(root.join("item.bin"), b"new").unwrap();
        assert!(!adapter
            .revalidate_content_inspection(path, observed)
            .unwrap());
        fs::remove_file(root.join("item.bin")).unwrap();
        assert!(!adapter
            .revalidate_content_inspection(path, observed)
            .unwrap());
        assert_eq!(fs::read(root.join("retained.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_revalidation_detects_stale_entry_and_length_error() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"one").unwrap();
        fs::write(root.join("two.bin"), b"two").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let paths = [Path::new("one.bin"), Path::new("two.bin")];
        let previous = adapter.inspect_content_batch(&paths).unwrap();
        assert!(adapter.revalidate_content_batch(&paths, &previous).unwrap());
        assert!(matches!(
            adapter.revalidate_content_batch(&paths, &previous[..1]),
            Err(LocalError::InvalidPath)
        ));
        fs::rename(root.join("two.bin"), root.join("old-two.bin")).unwrap();
        fs::write(root.join("two.bin"), b"replaced").unwrap();
        assert!(!adapter.revalidate_content_batch(&paths, &previous).unwrap());
        assert_eq!(fs::read(root.join("old-two.bin")).unwrap(), b"two");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_revalidation_propagates_unsafe_entry() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"one").unwrap();
        fs::write(root.join("two.bin"), b"two").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let paths = [Path::new("one.bin"), Path::new("two.bin")];
        let previous = adapter.inspect_content_batch(&paths).unwrap();
        fs::remove_file(root.join("two.bin")).unwrap();
        std::os::unix::fs::symlink("one.bin", root.join("two.bin")).unwrap();
        assert!(matches!(
            adapter.revalidate_content_batch(&paths, &previous),
            Err(LocalError::UnsafeEntry)
        ));
        assert_eq!(fs::read(root.join("one.bin")).unwrap(), b"one");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_detects_same_length_content_rewrite() {
        use std::io::{Seek, SeekFrom, Write};
        let root = sandbox();
        fs::write(root.join("item.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let path = Path::new("item.bin");
        let observed = adapter.inspect_content(path).unwrap();
        let item = root.join("item.bin");
        let mut file = fs::OpenOptions::new().write(true).open(&item).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"REPLACED").unwrap();
        file.sync_all().unwrap();
        assert_eq!(adapter.file_identity(path).unwrap(), observed.identity);
        assert!(!adapter
            .revalidate_content_inspection(path, observed)
            .unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_detects_same_size_rewrite() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"original").unwrap();
        fs::write(root.join("two.bin"), b"second__").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let paths = [Path::new("one.bin"), Path::new("two.bin")];
        let earlier = adapter.inspect_content_batch(&paths).unwrap();
        fs::write(root.join("two.bin"), b"UPDATED_").unwrap();
        assert_eq!(
            adapter.file_identity(paths[1]).unwrap(),
            earlier[1].identity
        );
        assert!(!adapter.revalidate_content_batch(&paths, &earlier).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_returns_identities_without_content_reads() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"one").unwrap();
        fs::write(root.join("two.bin"), b"two").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let paths = [Path::new("one.bin"), Path::new("two.bin")];
        let result = adapter.inspect_content_batch(&paths).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].identity.size, 3);
        assert_eq!(result[1].identity.size, 3);
        assert_ne!(result[0].identity.inode, result[1].identity.inode);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pinned_roots_detect_same_and_nested_directories() {
        let root = sandbox();
        fs::create_dir(root.join("nested")).unwrap();
        let a = LocalFilesystem::open(&root).unwrap();
        let b = LocalFilesystem::open(&root).unwrap();
        let nested = LocalFilesystem::open(&root.join("nested")).unwrap();
        assert!(a.overlaps_root(&b).unwrap());
        assert!(a.overlaps_root(&nested).unwrap());
        assert!(nested.overlaps_root(&a).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pinned_root_overlap_survives_directory_rename() {
        let root = sandbox();
        fs::create_dir(root.join("original")).unwrap();
        fs::create_dir(root.join("original/child")).unwrap();
        let parent = LocalFilesystem::open(&root.join("original")).unwrap();
        let child = LocalFilesystem::open(&root.join("original/child")).unwrap();
        fs::rename(root.join("original"), root.join("renamed")).unwrap();
        assert!(parent.overlaps_root(&child).unwrap());
        assert!(child.overlaps_root(&parent).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pinned_roots_reject_sibling_overlap_without_content_reads() {
        let root = sandbox();
        fs::create_dir(root.join("alpha")).unwrap();
        fs::create_dir(root.join("beta")).unwrap();
        let a = LocalFilesystem::open(&root.join("alpha")).unwrap();
        let b = LocalFilesystem::open(&root.join("beta")).unwrap();
        assert!(!a.overlaps_root(&b).unwrap());
        assert!(!b.overlaps_root(&a).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_rejects_duplicate_paths_and_case_aliases() {
        let root = sandbox();
        fs::write(root.join("Assets.bin"), b"data").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let duplicated = [Path::new("Assets.bin"), Path::new("Assets.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&duplicated),
            Err(LocalError::AlreadyExists)
        ));
        let aliases = [Path::new("Assets.bin"), Path::new("assets.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&aliases),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("Assets.bin")).unwrap(), b"data");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_rejects_hardlinked_duplicate_inode() {
        let root = sandbox();
        fs::write(root.join("original.bin"), b"data").unwrap();
        fs::hard_link(root.join("original.bin"), root.join("alias.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let entries = [Path::new("original.bin"), Path::new("alias.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&entries),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("alias.bin")).unwrap(), b"data");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_aborts_on_missing_file() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"one").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let paths = [Path::new("one.bin"), Path::new("missing.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&paths),
            Err(LocalError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound
        ));
        assert_eq!(fs::read(root.join("one.bin")).unwrap(), b"one");
        assert!(!root.join("missing.bin").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspection_batch_rejects_symlink_and_traversal() {
        let root = sandbox();
        fs::write(root.join("one.bin"), b"one").unwrap();
        std::os::unix::fs::symlink("one.bin", root.join("alias.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let alias = [Path::new("one.bin"), Path::new("alias.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&alias),
            Err(LocalError::UnsafeEntry)
        ));
        let traversal = [Path::new("one.bin"), Path::new("../one.bin")];
        assert!(matches!(
            adapter.inspect_content_batch(&traversal),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_copy_entry_points_reject_without_creating_destination() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("destination.bin");
        for consent in [
            ContentReadConsent::Denied,
            ContentReadConsent::GrantedForThisOperation,
        ] {
            assert!(matches!(
                adapter.copy_file_new(from, to, consent),
                Err(LocalError::UnsafeEntry)
            ));
            assert!(matches!(
                adapter.copy_file_new_with_reserve(from, to, 1024, consent),
                Err(LocalError::UnsafeEntry)
            ));
            assert!(!root.join("destination.bin").exists());
        }
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_content_inspection_is_metadata_only_and_never_enables_read() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let path = Path::new("source.bin");
        let inspected = adapter.inspect_content(path).unwrap();
        assert_eq!(inspected.identity.size, 8);
        assert_eq!(
            inspected.residency,
            LocalContentResidency::NotDatalessFlagged
        );
        let mut bytes = Vec::new();
        assert!(matches!(
            adapter.read_to_writer(
                path,
                &mut bytes,
                ContentReadConsent::GrantedForThisOperation
            ),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(bytes.is_empty());
        assert!(!adapter.capabilities().operations.read_content);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn local_residency_inspection_does_not_enable_content_reads() {
        let root = sandbox();
        fs::write(root.join("local.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter.content_residency(Path::new("local.bin")).unwrap(),
            LocalContentResidency::NotDatalessFlagged
        );
        let mut output = Vec::new();
        assert!(matches!(
            adapter.read_to_writer(
                Path::new("local.bin"),
                &mut output,
                ContentReadConsent::GrantedForThisOperation,
            ),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(output.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_content_residency_is_unknown_without_content_access() {
        let root = sandbox();
        fs::write(root.join("local.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter.content_residency(Path::new("local.bin")).unwrap(),
            LocalContentResidency::Unknown
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_reconciliation_reports_missing_names_without_recreating() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let source = Path::new("source.bin");
        let staged = Path::new("staged.bin");
        let expected = adapter.file_identity(source).unwrap();
        assert!(!adapter
            .reconcile_staged_link(source, staged, expected)
            .unwrap());
        assert!(!root.join("staged.bin").exists());
        fs::rename(root.join("source.bin"), root.join("retained.bin")).unwrap();
        assert!(!adapter
            .reconcile_staged_link(source, staged, expected)
            .unwrap());
        assert_eq!(fs::read(root.join("retained.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_reconciliation_is_read_only_and_detects_replacement() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let expected = adapter.file_identity(Path::new("source.bin")).unwrap();
        adapter
            .stage_move_link_new(Path::new("source.bin"), Path::new("staged.bin"))
            .unwrap();
        let source = Path::new("source.bin");
        let staged = Path::new("staged.bin");
        assert!(adapter
            .reconcile_staged_link(source, staged, expected)
            .unwrap());
        fs::rename(root.join("source.bin"), root.join("old.bin")).unwrap();
        fs::write(root.join("source.bin"), b"replacement").unwrap();
        assert!(!adapter
            .reconcile_staged_link(source, staged, expected)
            .unwrap());
        assert_eq!(fs::read(root.join("old.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("staged.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn metadata_mode_bits_report_file_and_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = sandbox();
        fs::write(root.join("file.bin"), b"content").unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        fs::set_permissions(root.join("file.bin"), fs::Permissions::from_mode(0o640)).unwrap();
        fs::set_permissions(root.join("folder"), fs::Permissions::from_mode(0o750)).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter.mode_bits(Path::new("file.bin")).unwrap() & 0o777,
            0o640
        );
        assert_eq!(
            adapter.mode_bits(Path::new("folder")).unwrap() & 0o777,
            0o750
        );
        assert_eq!(fs::read(root.join("file.bin")).unwrap(), b"content");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn metadata_mode_bits_reject_symlink_and_traversal() {
        let root = sandbox();
        fs::write(root.join("file.bin"), b"content").unwrap();
        std::os::unix::fs::symlink("file.bin", root.join("alias.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.mode_bits(Path::new("alias.bin")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(matches!(
            adapter.mode_bits(Path::new("../file.bin")),
            Err(LocalError::InvalidPath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_moves_regular_file_without_overwrite() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        fs::create_dir(root.join("target")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        adapter
            .rename_file_new(Path::new("source.bin"), Path::new("target/new.bin"))
            .unwrap();
        assert!(!root.join("source.bin").exists());
        assert_eq!(fs::read(root.join("target/new.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_never_overwrites_existing_destination() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("target.bin"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.rename_file_new(Path::new("source.bin"), Path::new("target.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("target.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rename_reconciliation_tracks_expected_inode_without_mutation() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("target.bin");
        let expected = adapter.file_identity(from).unwrap();
        assert!(!adapter.reconcile_renamed_file(from, to, expected).unwrap());
        adapter.rename_file_new(from, to).unwrap();
        assert!(adapter.reconcile_renamed_file(from, to, expected).unwrap());
        fs::rename(root.join("target.bin"), root.join("retained.bin")).unwrap();
        fs::write(root.join("target.bin"), b"foreign").unwrap();
        assert!(!adapter.reconcile_renamed_file(from, to, expected).unwrap());
        assert_eq!(fs::read(root.join("retained.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn move_identity_preview_does_not_mutate_file() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let source = Path::new("source.bin");
        let destination = Path::new("destination.bin");
        let observed = adapter.preview_move_identity(source, destination).unwrap();
        assert_eq!(observed, adapter.file_identity(source).unwrap());
        assert!(!root.join("destination.bin").exists());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn checked_rename_rejects_replaced_source_without_moving_it() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"first").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("destination.bin");
        let expected = adapter.preview_move_identity(from, to).unwrap();
        fs::rename(root.join("source.bin"), root.join("retained.bin")).unwrap();
        fs::write(root.join("source.bin"), b"second").unwrap();
        assert!(matches!(
            adapter.rename_file_new_checked(from, to, expected),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("destination.bin").exists());
        assert_eq!(fs::read(root.join("retained.bin")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"second");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn checked_rename_moves_unchanged_source_once() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("destination.bin");
        let expected = adapter.preview_move_identity(from, to).unwrap();
        adapter.rename_file_new_checked(from, to, expected).unwrap();
        assert!(!root.join("source.bin").exists());
        assert_eq!(fs::read(root.join("destination.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_rejects_source_symlink_without_mutation() {
        let root = sandbox();
        fs::write(root.join("real.bin"), b"protected").unwrap();
        std::os::unix::fs::symlink("real.bin", root.join("source.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(adapter
            .rename_file_new(Path::new("source.bin"), Path::new("destination.bin"))
            .is_err());
        assert_eq!(fs::read(root.join("real.bin")).unwrap(), b"protected");
        assert!(!root.join("destination.bin").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_rejects_stale_source_after_preflight() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let source = Path::new("source.bin");
        let destination = Path::new("destination.bin");
        let previous = adapter.preview_move_identity(source, destination).unwrap();
        fs::rename(root.join("source.bin"), root.join("preserved.bin")).unwrap();
        fs::write(root.join("source.bin"), b"replacement").unwrap();
        assert!(matches!(
            adapter.revalidate_move_new(source, destination, previous),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("destination.bin").exists());
        assert_eq!(fs::read(root.join("preserved.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_rejects_hardlinked_source() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::hard_link(root.join("source.bin"), root.join("alias.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.rename_file_new(Path::new("source.bin"), Path::new("target.bin")),
            Err(LocalError::UnsafeEntry)
        ));
        assert_eq!(fs::read(root.join("alias.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert!(!root.join("target.bin").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_rejects_case_collision() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("DEST.bin"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.rename_file_new(Path::new("source.bin"), Path::new("dest.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("DEST.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_rejects_parent_symlink() {
        let root = sandbox();
        let outside = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("alias")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(adapter
            .rename_file_new(Path::new("source.bin"), Path::new("alias/target.bin"))
            .is_err());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert!(!outside.join("target.bin").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rename_new_does_not_follow_destination_symlink() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("protected.bin"), b"protected").unwrap();
        std::os::unix::fs::symlink("protected.bin", root.join("target.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(adapter
            .rename_file_new(Path::new("source.bin"), Path::new("target.bin"))
            .is_err());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("protected.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn move_preview_rejects_case_alias_destination() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("TARGET.bin"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("target.bin");
        assert!(matches!(
            adapter.preview_move_new(from, to),
            Err(LocalError::AlreadyExists)
        ));
        let expected = adapter.file_identity(from).unwrap();
        assert!(matches!(
            adapter.revalidate_move_new(from, to, expected),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("TARGET.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn move_preview_rejects_symlink_parent_and_missing_parent() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink("real", root.join("alias")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        assert!(matches!(
            adapter.preview_move_new(from, Path::new("alias/new.bin")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(adapter
            .preview_move_new(from, Path::new("missing/new.bin"))
            .is_err());
        assert!(!root.join("real/new.bin").exists());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn move_preview_allows_distinct_destination_without_mutation() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let from = Path::new("source.bin");
        let to = Path::new("destination/new.bin");
        let preview = adapter.preview_move_new(from, to).unwrap();
        assert_eq!(preview.size, 6);
        let expected = adapter.file_identity(from).unwrap();
        adapter.revalidate_move_new(from, to, expected).unwrap();
        assert!(!root.join("destination/new.bin").exists());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staged_link_post_publish_io_error_keeps_both_names() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = secure::link_file_new_with_post_link_check(
            &adapter.root_fd,
            Path::new("source.bin"),
            Path::new("staged.bin"),
            || Err(io::Error::other("injected post-link failure")),
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("staged.bin")).unwrap(), b"original");
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source.bin"), Path::new("staged.bin")),
            Err(LocalError::AlreadyExists)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_reconciliation_detects_destination_inode_swap() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let source = Path::new("source.bin");
        let destination = Path::new("staged.bin");
        let expected = adapter.file_identity(source).unwrap();
        adapter.stage_move_link_new(source, destination).unwrap();
        fs::rename(root.join("staged.bin"), root.join("original-link.bin")).unwrap();
        fs::write(root.join("staged.bin"), b"foreign").unwrap();
        assert!(!adapter
            .reconcile_staged_link(source, destination, expected)
            .unwrap());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("original-link.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("staged.bin")).unwrap(), b"foreign");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_rejects_directory_source_without_new_name() {
        let root = sandbox();
        fs::create_dir(root.join("source")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source"), Path::new("staged")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("staged").exists());
        assert!(root.join("source").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_rejects_symlink_destination_without_altering_target() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("target.bin"), b"target").unwrap();
        std::os::unix::fs::symlink("target.bin", root.join("destination.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source.bin"), Path::new("destination.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("target.bin")).unwrap(), b"target");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_source_swap_after_publish_is_unknown_without_cleanup() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let result = secure::link_file_new_with_post_link_check(
            &adapter.root_fd,
            Path::new("source.bin"),
            Path::new("staged.bin"),
            || {
                fs::rename(root.join("source.bin"), root.join("original.bin"))?;
                fs::write(root.join("source.bin"), b"replacement")?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(LocalError::OutcomeUnknown(_))));
        assert_eq!(fs::read(root.join("original.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("staged.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"replacement");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_rejects_existing_hardlinked_source() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        fs::hard_link(root.join("source.bin"), root.join("shared.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source.bin"), Path::new("staged.bin")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("staged.bin").exists());
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("shared.bin")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_rejects_case_alias_without_changing_source() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        fs::write(root.join("TARGET.BIN"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source.bin"), Path::new("target.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("TARGET.BIN")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_rejects_symlink_source_without_publishing() {
        let root = sandbox();
        fs::write(root.join("real.bin"), b"protected").unwrap();
        std::os::unix::fs::symlink("real.bin", root.join("alias.bin")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("alias.bin"), Path::new("staged.bin")),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("staged.bin").exists());
        assert_eq!(fs::read(root.join("real.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stage_move_preserves_source_and_never_overwrites() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        fs::write(root.join("occupied.bin"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stage_move_link_new(Path::new("source.bin"), Path::new("occupied.bin")),
            Err(LocalError::AlreadyExists)
        ));
        adapter
            .stage_move_link_new(Path::new("source.bin"), Path::new("staged.bin"))
            .unwrap();
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("staged.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("occupied.bin")).unwrap(), b"protected");
        assert_eq!(
            adapter
                .file_identity(Path::new("source.bin"))
                .unwrap()
                .inode,
            adapter
                .file_identity(Path::new("staged.bin"))
                .unwrap()
                .inode
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_tree_accepts_existing_directory_with_unrelated_symlink() {
        let root = sandbox();
        fs::create_dir(root.join("existing")).unwrap();
        std::os::unix::fs::symlink("missing", root.join("unrelated")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter
                .create_directory_tree(Path::new("existing/child"))
                .unwrap(),
            1
        );
        assert!(root.join("existing/child").is_dir());
        assert!(fs::symlink_metadata(root.join("unrelated"))
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_rejects_case_alias_and_preserves_existing_directory() {
        let root = sandbox();
        fs::create_dir(root.join("Assets")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.create_directory(Path::new("assets")),
            Err(LocalError::AlreadyExists)
        ));
        assert!(root.join("Assets").is_dir());
        assert!(matches!(
            adapter.create_directory_tree(Path::new("assets/subdirectory")),
            Err(LocalError::AlreadyExists)
        ));
        assert!(!root.join("Assets/subdirectory").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn default_creation_allows_unrelated_symlink_in_directory() {
        let root = sandbox();
        std::os::unix::fs::symlink("missing-target", root.join("unrelated-link")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter
                .create_from_reader(Path::new("safe-file.bin"), &mut io::Cursor::new(b"data"),)
                .unwrap(),
            4
        );
        assert_eq!(fs::read(root.join("safe-file.bin")).unwrap(), b"data");
        assert!(fs::symlink_metadata(root.join("unrelated-link"))
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn default_creation_rejects_case_collision_before_reading() {
        struct PanicReader;
        impl io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("default writer must reject case alias before reading")
            }
        }
        let root = sandbox();
        fs::write(root.join("Archive.DAT"), b"original").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.create_from_reader(Path::new("archive.dat"), &mut PanicReader),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("Archive.DAT")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn guarded_create_rejects_case_collision_before_reading() {
        struct PanicReader;
        impl io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("case collision must be detected before content is read")
            }
        }
        let root = sandbox();
        fs::write(root.join("Report.TXT"), b"protected").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.create_from_reader_case_guarded(Path::new("report.txt"), &mut PanicReader,),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("Report.TXT")).unwrap(), b"protected");
        assert_eq!(
            adapter
                .create_from_reader_case_guarded(
                    Path::new("new-file.bin"),
                    &mut io::Cursor::new(b"payload"),
                )
                .unwrap(),
            7
        );
        assert_eq!(fs::read(root.join("new-file.bin")).unwrap(), b"payload");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn move_preview_is_read_only_and_rejects_collisions() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"original").unwrap();
        fs::write(root.join("existing.bin"), b"protected").unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let preview = adapter
            .preview_move_new(Path::new("source.bin"), Path::new("folder/target.bin"))
            .unwrap();
        assert_eq!(preview.size, 8);
        assert!(!preview.is_directory);
        assert!(!root.join("folder/target.bin").exists());
        assert!(matches!(
            adapter.preview_move_new(Path::new("source.bin"), Path::new("existing.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert!(matches!(
            adapter.preview_move_new(Path::new("source.bin"), Path::new("source.bin")),
            Err(LocalError::AlreadyExists)
        ));
        assert!(matches!(
            adapter.preview_move_new(Path::new("source.bin"), Path::new("../escape")),
            Err(LocalError::InvalidPath)
        ));
        assert!(matches!(
            adapter.preview_move_new(Path::new("source.bin"), Path::new("missing/target")),
            Err(LocalError::Io(_))
        ));
        assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"original");
        assert_eq!(fs::read(root.join("existing.bin")).unwrap(), b"protected");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_requires_consent_and_never_overwrites() {
        let root = sandbox();
        fs::write(root.join("source.bin"), b"source").unwrap();
        fs::write(root.join("occupied.bin"), b"occupied").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.copy_file_new(
                Path::new("source.bin"),
                Path::new("new.bin"),
                ContentReadConsent::Denied
            ),
            Err(LocalError::UnsafeEntry)
        ));
        assert!(!root.join("new.bin").exists());
        let result = adapter.copy_file_new(
            Path::new("source.bin"),
            Path::new("occupied.bin"),
            ContentReadConsent::GrantedForThisOperation,
        );
        #[cfg(target_os = "linux")]
        assert!(matches!(result, Err(LocalError::AlreadyExists)));
        #[cfg(not(target_os = "linux"))]
        assert!(matches!(result, Err(LocalError::UnsafeEntry)));
        assert_eq!(fs::read(root.join("occupied.bin")).unwrap(), b"occupied");
        let result = adapter.copy_file_new(
            Path::new("source.bin"),
            Path::new("new.bin"),
            ContentReadConsent::GrantedForThisOperation,
        );
        #[cfg(target_os = "linux")]
        {
            assert_eq!(result.unwrap(), 6);
            assert_eq!(fs::read(root.join("new.bin")).unwrap(), b"source");
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(matches!(result, Err(LocalError::UnsafeEntry)));
            assert!(!root.join("new.bin").exists());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_read_requires_explicit_operation_consent() {
        let root = sandbox();
        fs::write(root.join("local.txt"), b"private").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let mut output = Vec::new();
        assert!(adapter
            .read_to_writer(
                Path::new("local.txt"),
                &mut output,
                ContentReadConsent::Denied,
            )
            .is_err());
        assert!(output.is_empty());
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                adapter
                    .read_to_writer(
                        Path::new("local.txt"),
                        &mut output,
                        ContentReadConsent::GrantedForThisOperation,
                    )
                    .unwrap(),
                7
            );
            assert_eq!(output, b"private");
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(adapter
                .read_to_writer(
                    Path::new("local.txt"),
                    &mut output,
                    ContentReadConsent::GrantedForThisOperation,
                )
                .is_err());
            assert!(output.is_empty());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_list_stat_and_refuse_overwrite() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert_eq!(
            adapter
                .create_from_reader(Path::new("пример.txt"), &mut Cursor::new(b"abc"))
                .unwrap(),
            3
        );
        assert_eq!(adapter.stat(Path::new("пример.txt")).unwrap().size, 3);
        assert_eq!(
            adapter.list(Path::new("пример.txt")).err().map(|_| true),
            Some(true)
        );
        assert_eq!(adapter.list(Path::new(".")).err().map(|_| true), Some(true));
        assert!(matches!(
            adapter.create_from_reader(Path::new("пример.txt"), &mut Cursor::new(b"changed")),
            Err(LocalError::AlreadyExists)
        ));
        assert_eq!(fs::read(root.join("пример.txt")).unwrap(), b"abc");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_traversal_and_cleans_up_on_reader_failure() {
        struct Broken;
        impl io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected"))
            }
        }
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stat(Path::new("../escape")),
            Err(LocalError::InvalidPath)
        ));
        assert!(adapter
            .create_from_reader(Path::new("bad"), &mut Broken)
            .is_err());
        assert!(adapter.list(Path::new("bad")).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn list_page_orders_unicode_entries_without_duplicates() {
        let root = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        for name in ["ёж.txt", "alpha.txt", "b.txt"] {
            fs::write(root.join(name), b"x").unwrap();
        }
        let root_items = adapter.list(Path::new("")).unwrap();
        assert_eq!(root_items.len(), 3);
        let first = adapter.list_page(Path::new("alpha.txt"), 0, 10);
        assert!(first.is_err());
        // Root itself is not addressable as a relative file path by design.
        fs::create_dir(root.join("folder")).unwrap();
        for name in ["ёж.txt", "alpha.txt", "b.txt"] {
            fs::write(root.join("folder").join(name), b"x").unwrap();
        }
        let page1 = adapter.list_page(Path::new("folder"), 0, 2).unwrap();
        let page2 = adapter.list_page(Path::new("folder"), 2, 2).unwrap();
        let full = adapter.list(Path::new("folder")).unwrap();
        assert_eq!([page1, page2].concat(), full);
        assert!(adapter
            .list_page(Path::new("folder"), 0, 0)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn list_does_not_follow_swapped_parent_directory() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let outside = sandbox();
        fs::create_dir(root.join("child")).unwrap();
        fs::write(outside.join("private"), b"secret").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        fs::rename(root.join("child"), root.join("moved")).unwrap();
        symlink(&outside, root.join("child")).unwrap();
        assert!(adapter.list(Path::new("child")).is_err());
        assert_eq!(adapter.list(Path::new("moved")).unwrap().len(), 0);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stat_does_not_follow_swapped_parent_directory() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let outside = sandbox();
        fs::create_dir(root.join("child")).unwrap();
        fs::write(outside.join("private"), b"secret").unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        fs::rename(root.join("child"), root.join("moved")).unwrap();
        symlink(&outside, root.join("child")).unwrap();
        assert!(adapter.stat(Path::new("child/private")).is_err());
        assert_eq!(fs::read(outside.join("private")).unwrap(), b"secret");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_is_scoped_and_does_not_follow_parent_symlinks() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let outside = sandbox();
        let adapter = LocalFilesystem::open(&root).unwrap();
        adapter.create_directory(Path::new("created")).unwrap();
        assert!(root.join("created").is_dir());
        assert!(matches!(
            adapter.create_directory(Path::new("created")),
            Err(LocalError::AlreadyExists)
        ));
        symlink(&outside, root.join("link")).unwrap();
        assert!(adapter.create_directory(Path::new("link/escape")).is_err());
        assert!(!outside.join("escape").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parent_replaced_during_streaming_write_stays_on_opened_directory() {
        use std::os::unix::fs::symlink;

        struct SwapOnRead {
            child: PathBuf,
            moved: PathBuf,
            outside: PathBuf,
            swapped: bool,
        }
        impl io::Read for SwapOnRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if !self.swapped {
                    fs::rename(&self.child, &self.moved)?;
                    symlink(&self.outside, &self.child)?;
                    self.swapped = true;
                    output[0] = b'x';
                    return Ok(1);
                }
                Ok(0)
            }
        }

        let root = sandbox();
        let outside = sandbox();
        fs::create_dir(root.join("child")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        let mut reader = SwapOnRead {
            child: root.join("child"),
            moved: root.join("moved"),
            outside: outside.clone(),
            swapped: false,
        };
        let result = adapter.create_from_reader(Path::new("child/file.txt"), &mut reader);
        assert!(result.is_ok());
        assert!(reader.swapped);
        assert_eq!(fs::read(root.join("moved/file.txt")).unwrap(), b"x");
        assert!(!outside.join("file.txt").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn swapped_parent_symlink_cannot_redirect_creation() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        let outside = sandbox();
        fs::create_dir(root.join("child")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        fs::rename(root.join("child"), root.join("moved")).unwrap();
        symlink(&outside, root.join("child")).unwrap();
        assert!(adapter
            .create_from_reader(Path::new("child/escape.txt"), &mut Cursor::new(b"secret"))
            .is_err());
        assert!(!outside.join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_inside_scope() {
        use std::os::unix::fs::symlink;
        let root = sandbox();
        symlink(std::env::temp_dir(), root.join("outside")).unwrap();
        let adapter = LocalFilesystem::open(&root).unwrap();
        assert!(matches!(
            adapter.stat(Path::new("outside/any")),
            Err(LocalError::UnsafeEntry)
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
