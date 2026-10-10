//! Local filesystem adapter, scoped to one existing canonical directory.
//! Initial safe slice: metadata and non-overwriting atomic creation only.
//! Content reads on macOS are intentionally not exposed until iCloud placeholder
//! detection and per-transfer consent can be enforced.
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
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

pub struct LocalFilesystem {
    #[cfg(not(unix))]
    root: PathBuf,
    #[cfg(unix)]
    root_fd: File,
}

impl LocalFilesystem {
    pub fn open(root: &Path) -> Result<Self> {
        let meta = fs::symlink_metadata(root)?;
        if meta.file_type().is_symlink() {
            return Err(LocalError::UnsafeEntry);
        }
        if !meta.is_dir() {
            return Err(LocalError::NotDirectory);
        }
        let canonical = fs::canonicalize(root)?;
        #[cfg(unix)]
        let root_fd = secure::root(&canonical)?;
        Ok(Self {
            #[cfg(not(unix))]
            root: canonical,
            #[cfg(unix)]
            root_fd,
        })
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

    /// Stable lexical pagination over a snapshot of directory metadata.
    /// Callers must rescan if the directory changes between page requests.
    pub fn list_page(&self, path: &Path, offset: usize, limit: usize) -> Result<Vec<LocalEntry>> {
        if !path.as_os_str().is_empty() {
            Self::relative(path)?;
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let entries = self.list(path)?;
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
            Ok(io::copy(&mut input, output)?)
        }
        #[cfg(not(unix))]
        {
            let _ = output;
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

    /// Create a new file through directory descriptors, without following
    /// symlinks or replacing an existing target. Unsupported platforms fail closed.
    /// Reading cloud-backed local placeholders remains prohibited until guarded.
    pub fn create_from_reader<R: io::Read>(&self, target: &Path, input: &mut R) -> Result<u64> {
        Self::relative(target)?;
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
        let p = std::env::temp_dir().join(format!(
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
