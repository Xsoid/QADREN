//! Local filesystem adapter, scoped to one existing canonical directory.
//! Initial safe slice: metadata and non-overwriting atomic creation only.
//! Content reads on macOS are intentionally not exposed until iCloud placeholder
//! detection and per-transfer consent can be enforced.
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};
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
    Io(io::Error),
}
impl From<io::Error> for LocalError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
pub type Result<T> = std::result::Result<T, LocalError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
}

pub struct LocalFilesystem {
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

    pub fn list(&self, path: &Path) -> Result<Vec<LocalEntry>> {
        let dir = self.checked(path)?;
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

    /// Stable lexical pagination over a snapshot of directory metadata.
    /// Callers must rescan if the directory changes between page requests.
    pub fn list_page(&self, path: &Path, offset: usize, limit: usize) -> Result<Vec<LocalEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let entries = self.list(path)?;
        Ok(entries.into_iter().skip(offset).take(limit).collect())
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
    use std::io::Cursor;

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
        let all = adapter.list(Path::new("")).err();
        assert!(matches!(all, Some(LocalError::InvalidPath)));
        let first = adapter.list_page(Path::new("alpha.txt"), 0, 10);
        assert!(matches!(first, Err(LocalError::NotDirectory)));
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
