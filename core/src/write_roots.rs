//! Write-root lock of Scenario runs (D-025 section 11, issue #35).
//!
//! Two runs must not execute at the same time when the write root of one is
//! equal to, inside, or around any root of the other on the same storage.
//! This module only compares roots; the Registry keeps the durable lock.

use crate::registry::Scenario;
use crate::storage_access::YANDEX_DISK;
use std::path::{Path, PathBuf};

/// One root of a Scenario on one storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioRoot {
    /// `fs` for the Mac and SMB mounts, which share one path space, or
    /// `yandex:<connection_id>` for one Yandex Disk.
    pub storage: String,
    /// Canonical, lowercased path components. APFS and SMB usually ignore
    /// letter case, so a lowercased comparison can only be stricter.
    pub components: Vec<String>,
    pub write: bool,
}

/// The roots of a Scenario. A one-way Scenario writes only to its
/// destination; a two-way Scenario writes to both roots.
pub fn scenario_roots(scenario: &Scenario, adapter: impl Fn(&str) -> Option<String>) -> Vec<ScenarioRoot> {
    let (left_writes, right_writes) = match scenario.direction.as_str() {
        "left_to_right" => (false, true),
        "right_to_left" => (true, false),
        _ => (true, true),
    };
    [
        (&scenario.left_connection_id, &scenario.left_root, left_writes),
        (&scenario.right_connection_id, &scenario.right_root, right_writes),
    ]
    .into_iter()
    .map(|(connection, root, write)| {
        let yandex = adapter(connection).as_deref() == Some(YANDEX_DISK);
        let (storage, path) = if yandex {
            let path = root.strip_prefix("disk:").unwrap_or(root);
            (format!("yandex:{connection}"), PathBuf::from(path))
        } else {
            ("fs".to_owned(), canonical(Path::new(root)))
        };
        ScenarioRoot {
            storage,
            components: path
                .components()
                .filter_map(|part| match part {
                    std::path::Component::Normal(name) => {
                        Some(name.to_string_lossy().to_lowercase())
                    }
                    _ => None,
                })
                .collect(),
            write,
        }
    })
    .collect()
}

/// The canonical form of the longest existing ancestor, with the missing
/// rest appended. An unmounted SMB root keeps its mount directory path.
fn canonical(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(found) = std::fs::canonicalize(current) {
            return missing.iter().rev().fold(found, |path, part| path.join(part));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_owned());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn nested(a: &ScenarioRoot, b: &ScenarioRoot) -> bool {
    a.storage == b.storage
        && (a.components.starts_with(&b.components) || b.components.starts_with(&a.components))
}

/// True when the runs of two Scenarios must not execute at the same time:
/// a write root of one is equal to, inside, or around any root of the other.
pub fn conflict(a: &[ScenarioRoot], b: &[ScenarioRoot]) -> bool {
    a.iter()
        .any(|left| b.iter().any(|right| (left.write || right.write) && nested(left, right)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(left: &str, right: &str, direction: &str) -> Scenario {
        Scenario {
            id: "s".into(),
            name: "S".into(),
            left_connection_id: if left.starts_with("disk:") { "yandex" } else { "mac" }.into(),
            left_root: left.into(),
            right_connection_id: "mac".into(),
            right_root: right.into(),
            direction: direction.into(),
            enabled: true,
            revision: 1,
        }
    }

    fn roots(left: &str, right: &str, direction: &str) -> Vec<ScenarioRoot> {
        scenario_roots(&scenario(left, right, direction), |id| {
            Some(if id == "yandex" { YANDEX_DISK } else { "local_filesystem" }.to_owned())
        })
    }

    #[test]
    fn a_write_root_conflicts_with_an_equal_inner_or_outer_root() {
        let base = std::env::temp_dir().join("qadren-write-roots");
        let path = |name: &str| base.join(name).to_string_lossy().into_owned();
        let backup = roots(&path("photos"), &path("backup"), "left_to_right");
        // The same destination.
        assert!(conflict(&backup, &roots(&path("music"), &path("backup"), "left_to_right")));
        // A destination inside or around the other destination.
        assert!(conflict(&backup, &roots(&path("music"), &path("backup/music"), "left_to_right")));
        assert!(conflict(&backup, &roots(&path("music"), &base.to_string_lossy(), "left_to_right")));
        // The other run reads what this run writes.
        assert!(conflict(&backup, &roots(&path("backup"), &path("copy"), "left_to_right")));
        // Letter case does not make a different folder.
        assert!(conflict(&backup, &roots(&path("music"), &path("BACKUP"), "left_to_right")));
        // Two runs that only read the same source do not conflict.
        assert!(!conflict(&backup, &roots(&path("photos"), &path("other"), "left_to_right")));
        // A sibling with a common name prefix is another folder.
        assert!(!conflict(&backup, &roots(&path("music"), &path("backup-2"), "left_to_right")));
        // A two-way Scenario writes to both roots.
        assert!(conflict(&backup, &roots(&path("photos"), &path("elsewhere"), "bidirectional")));
    }

    #[test]
    fn yandex_roots_compare_only_within_one_yandex_disk() {
        let base = std::env::temp_dir().join("qadren-write-roots");
        let mac = base.join("yad").to_string_lossy().into_owned();
        let first = roots("disk:/Photos", &mac, "left_to_right");
        let second = roots("disk:/Photos/2026", &base.join("other").to_string_lossy(), "left_to_right");
        // Both only read the Yandex Disk: no conflict.
        assert!(!conflict(&first, &second));
        // A Mac path "/Photos" is not the Yandex Disk path "disk:/Photos".
        assert!(!conflict(&first, &roots("/Photos", "/Users/x/out", "left_to_right")));
    }

    #[test]
    fn a_missing_root_keeps_its_canonical_parent() {
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let missing = base.join("qadren-missing-root").join("inner");
        assert_eq!(canonical(&missing), missing);
    }
}
