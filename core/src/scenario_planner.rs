//! Pure, conservative Scenario dry-run planning.
//! No adapter I/O, Job submission, content reads or FileSyncState writes occur
//! here. Callers must supply already-authorized metadata snapshots.
use crate::model::{
    ConfirmedFileVersion, FileSyncState, ScenarioDraft, ScenarioReference, SyncDirection,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedFile {
    pub relative_path: String,
    pub object_revision: Option<String>,
    pub size: u64,
    pub checksum_md5: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideChange {
    Unchanged,
    Added,
    Removed,
    Changed,
    /// Metadata cannot establish that the observed object is the confirmed one.
    NeedsVerification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    NoAction,
    Create {
        from_left: bool,
    },
    Update {
        from_left: bool,
    },
    /// Planner never authorizes deletion; a scoped policy decision is required.
    DeleteRequiresDecision {
        on_left: bool,
    },
    Conflict {
        reason: &'static str,
    },
    NeedsVerification {
        reason: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub relative_path: String,
    pub left: SideChange,
    pub right: SideChange,
    pub action: PlanAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerError {
    InvalidScenario,
    UnsupportedFilters,
    InvalidPath,
    DuplicatePath,
    BaselineForAnotherScenario,
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && !path.chars().any(char::is_control)
}

fn is_valid_checksum(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn matches_confirmed(observed: &ObservedFile, confirmed: &ConfirmedFileVersion) -> bool {
    // A connection-local object revision is comparable only with its own
    // historical value. Cross-endpoint equality requires a trusted checksum.
    if observed.size != confirmed.size {
        return false;
    }
    match (&observed.checksum_md5, &confirmed.checksum_md5) {
        (Some(observed), Some(confirmed))
            if is_valid_checksum(observed)
                && is_valid_checksum(confirmed)
                && observed.eq_ignore_ascii_case(confirmed) =>
        {
            true
        }
        _ => {
            observed.object_revision.is_some()
                && observed.object_revision == confirmed.object_revision
        }
    }
}

fn delta(observed: Option<&ObservedFile>, confirmed: Option<&ConfirmedFileVersion>) -> SideChange {
    match (observed, confirmed) {
        (None, None) => SideChange::Unchanged,
        (Some(_), None) => SideChange::Added,
        (None, Some(_)) => SideChange::Removed,
        (Some(observed), Some(confirmed)) => {
            if matches_confirmed(observed, confirmed) {
                return SideChange::Unchanged;
            }
            // Either a trusted checksum or an endpoint-local provider revision
            // can prove a difference from this endpoint's own baseline. They
            // cannot, by themselves, prove cross-endpoint byte equality.
            if matches!(
                (&observed.checksum_md5, &confirmed.checksum_md5),
                (Some(observed), Some(confirmed))
                    if is_valid_checksum(observed) && is_valid_checksum(confirmed)
            ) || (observed.object_revision.is_some() && confirmed.object_revision.is_some())
            {
                return SideChange::Changed;
            }
            SideChange::NeedsVerification
        } // Name and size alone never prove byte equality. A changed or opaque
          // observation is therefore a required verification boundary, not an
          // executable update.
    }
}

fn same_confirmed_content(left: &ObservedFile, right: &ObservedFile) -> bool {
    left.size == right.size
        && matches!(
            (&left.checksum_md5, &right.checksum_md5),
            (Some(left), Some(right))
                if is_valid_checksum(left)
                    && is_valid_checksum(right)
                    && left.eq_ignore_ascii_case(right)
        )
}

fn directional_action(
    source: SideChange,
    destination: SideChange,
    source_present: bool,
    destination_present: bool,
    from_left: bool,
) -> PlanAction {
    if source == SideChange::NeedsVerification || destination == SideChange::NeedsVerification {
        return PlanAction::NeedsVerification {
            reason: "metadata does not prove the confirmed object identity",
        };
    }
    if destination != SideChange::Unchanged {
        return PlanAction::Conflict {
            reason: "destination changed since the confirmed baseline",
        };
    }
    match source {
        SideChange::Unchanged => PlanAction::NoAction,
        SideChange::Added | SideChange::Changed if source_present && !destination_present => {
            PlanAction::Create { from_left }
        }
        SideChange::Added | SideChange::Changed if source_present => {
            PlanAction::Update { from_left }
        }
        SideChange::Removed => PlanAction::DeleteRequiresDecision {
            on_left: !from_left,
        },
        _ => PlanAction::NeedsVerification {
            reason: "source observation is incomplete",
        },
    }
}

fn bidirectional_action(
    left: SideChange,
    right: SideChange,
    left_file: Option<&ObservedFile>,
    right_file: Option<&ObservedFile>,
) -> PlanAction {
    if left == SideChange::NeedsVerification || right == SideChange::NeedsVerification {
        return PlanAction::NeedsVerification {
            reason: "metadata does not prove the confirmed object identity",
        };
    }
    if left == SideChange::Unchanged && right == SideChange::Unchanged {
        return PlanAction::NoAction;
    }
    if left == SideChange::Removed && right == SideChange::Unchanged {
        return PlanAction::DeleteRequiresDecision { on_left: false };
    }
    if right == SideChange::Removed && left == SideChange::Unchanged {
        return PlanAction::DeleteRequiresDecision { on_left: true };
    }
    if left == SideChange::Unchanged && matches!(right, SideChange::Added | SideChange::Changed) {
        return if left_file.is_none() {
            PlanAction::Create { from_left: false }
        } else {
            PlanAction::Update { from_left: false }
        };
    }
    if right == SideChange::Unchanged && matches!(left, SideChange::Added | SideChange::Changed) {
        return if right_file.is_none() {
            PlanAction::Create { from_left: true }
        } else {
            PlanAction::Update { from_left: true }
        };
    }
    if let (Some(left_file), Some(right_file)) = (left_file, right_file) {
        if same_confirmed_content(left_file, right_file) {
            return PlanAction::NoAction;
        }
    }
    PlanAction::Conflict {
        reason: "both endpoints changed or lack a shared confirmed content identity",
    }
}

/// Produce a deterministic, metadata-only plan. It is deliberately unable to
/// make destructive operations executable or to mutate the confirmed baseline.
pub fn plan(
    scenario: &ScenarioDraft,
    reference: &ScenarioReference,
    baseline: &[FileSyncState],
    left_snapshot: &[ObservedFile],
    right_snapshot: &[ObservedFile],
) -> Result<Vec<PlanItem>, PlannerError> {
    scenario
        .validate()
        .map_err(|_| PlannerError::InvalidScenario)?;
    // Pattern semantics belong to adapters. Planning them here without the
    // same matcher could silently include an excluded object, so stop until a
    // shared, tested filter contract exists.
    if !scenario.filters.excluded_patterns.is_empty() {
        return Err(PlannerError::UnsupportedFilters);
    }
    if reference.scenario_id.is_empty() || reference.revision == 0 {
        return Err(PlannerError::InvalidScenario);
    }
    let mut confirmed = BTreeMap::new();
    for entry in baseline {
        if entry.scenario != *reference {
            return Err(PlannerError::BaselineForAnotherScenario);
        }
        if !valid_path(&entry.relative_path) {
            return Err(PlannerError::InvalidPath);
        }
        if confirmed.insert(&entry.relative_path, entry).is_some() {
            return Err(PlannerError::DuplicatePath);
        }
    }
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    for file in left_snapshot {
        if !valid_path(&file.relative_path) {
            return Err(PlannerError::InvalidPath);
        }
        if left.insert(&file.relative_path, file).is_some() {
            return Err(PlannerError::DuplicatePath);
        }
    }
    for file in right_snapshot {
        if !valid_path(&file.relative_path) {
            return Err(PlannerError::InvalidPath);
        }
        if right.insert(&file.relative_path, file).is_some() {
            return Err(PlannerError::DuplicatePath);
        }
    }
    let paths = confirmed
        .keys()
        .chain(left.keys())
        .chain(right.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    Ok(paths
        .into_iter()
        .map(|path| {
            let state = confirmed.get(path).copied();
            let left_file = left.get(path).copied();
            let right_file = right.get(path).copied();
            let left_change = delta(left_file, state.and_then(|entry| entry.left.as_ref()));
            let right_change = delta(right_file, state.and_then(|entry| entry.right.as_ref()));
            let action = match scenario.direction {
                SyncDirection::LeftToRight => directional_action(
                    left_change,
                    right_change,
                    left_file.is_some(),
                    right_file.is_some(),
                    true,
                ),
                SyncDirection::RightToLeft => directional_action(
                    right_change,
                    left_change,
                    right_file.is_some(),
                    left_file.is_some(),
                    false,
                ),
                SyncDirection::Bidirectional => {
                    bidirectional_action(left_change, right_change, left_file, right_file)
                }
            };
            PlanItem {
                relative_path: path.clone(),
                left: left_change,
                right: right_change,
                action,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ConflictPolicy, DeletionPolicy, ScenarioFilters, ScenarioPolicies, StorageEndpoint,
    };

    fn scenario(direction: SyncDirection) -> ScenarioDraft {
        ScenarioDraft {
            name: "test".into(),
            left: StorageEndpoint {
                connection_id: "left".into(),
                root: "root".into(),
            },
            right: StorageEndpoint {
                connection_id: "right".into(),
                root: "root".into(),
            },
            direction,
            filters: ScenarioFilters::default(),
            policies: ScenarioPolicies {
                conflicts: ConflictPolicy::RequireDecision,
                deletions: DeletionPolicy::RequireDecision,
            },
        }
    }
    fn reference() -> ScenarioReference {
        ScenarioReference {
            scenario_id: "scenario".into(),
            revision: 1,
        }
    }
    fn file(path: &str, revision: &str, checksum: Option<&str>) -> ObservedFile {
        ObservedFile {
            relative_path: path.into(),
            object_revision: Some(revision.into()),
            size: 4,
            checksum_md5: checksum.map(str::to_owned),
        }
    }
    fn confirmed(version: &ObservedFile) -> ConfirmedFileVersion {
        ConfirmedFileVersion {
            object_revision: version.object_revision.clone(),
            size: version.size,
            checksum_md5: version.checksum_md5.clone(),
        }
    }

    #[test]
    fn directional_new_source_file_is_a_create_proposal() {
        let items = plan(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[],
            &[file("new.txt", "l1", None)],
            &[],
        )
        .unwrap();
        assert_eq!(items[0].action, PlanAction::Create { from_left: true });
    }

    #[test]
    fn same_size_without_trusted_identity_requires_verification() {
        let mut old = file("same.txt", "old", None);
        old.object_revision = None;
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "same.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let mut observed = file("same.txt", "new", None);
        observed.object_revision = None;
        let items = plan(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[baseline],
            &[observed],
            &[old],
        )
        .unwrap();
        assert!(matches!(
            items[0].action,
            PlanAction::NeedsVerification { .. }
        ));
    }

    #[test]
    fn changed_destination_is_a_conflict_not_an_overwrite() {
        let old = file("file.txt", "old", Some("abcdef0123456789abcdef0123456789"));
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "file.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let left = file(
            "file.txt",
            "new-left",
            Some("11111111111111111111111111111111"),
        );
        let right = file(
            "file.txt",
            "new-right",
            Some("22222222222222222222222222222222"),
        );
        let items = plan(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[baseline],
            &[left],
            &[right],
        )
        .unwrap();
        assert!(matches!(items[0].action, PlanAction::Conflict { .. }));
    }

    #[test]
    fn deletion_is_never_authorized_by_the_planner() {
        let old = file("gone.txt", "old", None);
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "gone.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let items = plan(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[baseline],
            &[],
            &[old],
        )
        .unwrap();
        assert_eq!(
            items[0].action,
            PlanAction::DeleteRequiresDecision { on_left: false }
        );
    }

    #[test]
    fn rejects_duplicate_and_unsafe_paths_before_planning() {
        assert_eq!(
            plan(
                &scenario(SyncDirection::LeftToRight),
                &reference(),
                &[],
                &[file("a", "1", None), file("a", "2", None)],
                &[]
            ),
            Err(PlannerError::DuplicatePath)
        );
        assert_eq!(
            plan(
                &scenario(SyncDirection::LeftToRight),
                &reference(),
                &[],
                &[file("../a", "1", None)],
                &[]
            ),
            Err(PlannerError::InvalidPath)
        );
    }

    #[test]
    fn refuses_filters_until_the_adapter_and_planner_share_the_same_semantics() {
        let mut filtered = scenario(SyncDirection::LeftToRight);
        filtered.filters.excluded_patterns.push("*.tmp".into());
        assert_eq!(
            plan(&filtered, &reference(), &[], &[], &[]),
            Err(PlannerError::UnsupportedFilters)
        );
    }
}
