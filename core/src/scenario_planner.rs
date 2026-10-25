//! Pure, conservative Scenario dry-run planning.
//! No adapter I/O, Job submission, content reads or FileSyncState writes occur
//! here. Callers must supply already-authorized metadata snapshots.
use crate::model::{
    ConfirmedFileVersion, DeletionMode, FileSyncState, ScenarioDraft, ScenarioReference,
    ScenarioScope, ScopeMatch, SyncDirection, MASS_DELETION_THRESHOLD,
};
use icu_normalizer::ComposingNormalizerBorrowed;
use std::collections::{BTreeMap, BTreeSet};

/// Reason for paths that differ by bytes but can name the same file on a
/// case-insensitive or Unicode-normalizing volume, such as APFS.
pub const NAME_COLLISION_REASON: &str =
    "names differ only by letter case or Unicode form and can name the same file";

/// Reason for a path that is a symbolic link or special file on a side.
/// QADREN does not follow or synchronize such entries, so the path must not
/// be planned as a Create, an Update, or a deletion.
pub const SKIPPED_ENTRY_REASON: &str =
    "a symbolic link or special file has this name; QADREN does not synchronize it";

/// Reason for a path that is outside the selection by bytes, but can name a
/// selected object on a case-insensitive or Unicode-normalizing volume.
pub const SELECTION_NAME_VARIANT_REASON: &str =
    "the name differs from a selected object only by letter case or Unicode form";

/// Conservative comparison key: NFC form and lowercase. A false match only
/// makes a path non-executable; it never authorizes an effect.
pub fn name_key(path: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(path)
        .to_lowercase()
}

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
    /// A scoped policy decision is required before this deletion.
    DeleteRequiresDecision {
        on_left: bool,
    },
    /// The revision's approved simple deletion rule covers this deletion:
    /// the object was confirmed, is unchanged on `on_left`'s side, and the
    /// plan is not a mass deletion. The executor rechecks all of this.
    Delete {
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
    plan_with_skipped(
        scenario,
        reference,
        baseline,
        left_snapshot,
        right_snapshot,
        &[],
        &[],
    )
}

/// Like `plan`, with the relative paths of symbolic links and special files
/// that each side skipped. A skipped path that also appears in the baseline or
/// on the other side is never executable: a link that replaced a confirmed file
/// must not look like a deletion, and a file must not be created over a link.
pub fn plan_with_skipped(
    scenario: &ScenarioDraft,
    reference: &ScenarioReference,
    baseline: &[FileSyncState],
    left_snapshot: &[ObservedFile],
    right_snapshot: &[ObservedFile],
    skipped_left: &[String],
    skipped_right: &[String],
) -> Result<Vec<PlanItem>, PlannerError> {
    plan_scoped(
        scenario,
        reference,
        baseline,
        left_snapshot,
        right_snapshot,
        skipped_left,
        skipped_right,
        &ScenarioScope::default(),
    )
}

/// Like `plan_with_skipped`, limited to the revision's selected objects and
/// with its deletion rule. A path outside the selection is not in the plan.
/// A name variant of a selected object is never executable. The simple rule
/// makes a deletion executable only when the plan has at most
/// `MASS_DELETION_THRESHOLD` deletions; otherwise all of them need a decision.
#[allow(clippy::too_many_arguments)]
pub fn plan_scoped(
    scenario: &ScenarioDraft,
    reference: &ScenarioReference,
    baseline: &[FileSyncState],
    left_snapshot: &[ObservedFile],
    right_snapshot: &[ObservedFile],
    skipped_left: &[String],
    skipped_right: &[String],
    scope: &ScenarioScope,
) -> Result<Vec<PlanItem>, PlannerError> {
    let selected_paths = ScenarioScope::normalized_selection(&scope.selected_paths)
        .map_err(|_| PlannerError::InvalidScenario)?;
    let scope = ScenarioScope {
        selected_paths,
        deletion_mode: scope.deletion_mode,
    };
    let skipped = skipped_left
        .iter()
        .chain(skipped_right)
        .map(|path| path.as_str())
        .collect::<BTreeSet<_>>();
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
        .filter(|path| scope.classify(path, name_key) != ScopeMatch::Outside)
        .collect::<BTreeSet<_>>();
    let mut variants: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        *variants.entry(name_key(path)).or_default() += 1;
    }
    let mut items = paths
        .into_iter()
        .map(|path| {
            let state = confirmed.get(path).copied();
            let left_file = left.get(path).copied();
            let right_file = right.get(path).copied();
            let left_change = delta(left_file, state.and_then(|entry| entry.left.as_ref()));
            let right_change = delta(right_file, state.and_then(|entry| entry.right.as_ref()));
            let collides = variants.get(&name_key(path)).copied().unwrap_or(0) > 1;
            let action = if scope.classify(path, name_key) == ScopeMatch::NameVariant {
                PlanAction::NeedsVerification {
                    reason: SELECTION_NAME_VARIANT_REASON,
                }
            } else if skipped.contains(path.as_str()) {
                PlanAction::NeedsVerification {
                    reason: SKIPPED_ENTRY_REASON,
                }
            } else if collides {
                PlanAction::NeedsVerification {
                    reason: NAME_COLLISION_REASON,
                }
            } else {
                match scenario.direction {
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
                }
            };
            PlanItem {
                relative_path: path.clone(),
                left: left_change,
                right: right_change,
                action,
            }
        })
        .collect::<Vec<_>>();
    if scope.deletion_mode == DeletionMode::Simple {
        let deletions = items
            .iter()
            .filter(|item| matches!(item.action, PlanAction::DeleteRequiresDecision { .. }))
            .count();
        if deletions <= MASS_DELETION_THRESHOLD {
            for item in &mut items {
                if let PlanAction::DeleteRequiresDecision { on_left } = item.action {
                    item.action = PlanAction::Delete { on_left };
                }
            }
        }
    }
    Ok(items)
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
    fn skipped_links_never_become_create_update_or_deletion() {
        let confirmed_file = file("was-file.txt", "l1", None);
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "was-file.txt".into(),
            left: Some(confirmed(&confirmed_file)),
            right: Some(confirmed(&file("was-file.txt", "r1", None))),
        };
        for direction in [SyncDirection::LeftToRight, SyncDirection::Bidirectional] {
            let items = plan_with_skipped(
                &scenario(direction),
                &reference(),
                std::slice::from_ref(&baseline),
                // The left file became a link; the right still has the file.
                &[file("plain.txt", "l2", None)],
                &[
                    file("was-file.txt", "r1", None),
                    file("link-on-left.txt", "r3", None),
                ],
                &["was-file.txt".into(), "link-on-left.txt".into()],
                &[],
            )
            .unwrap();
            for item in &items {
                let expected = if item.relative_path == "plain.txt" {
                    PlanAction::Create { from_left: true }
                } else {
                    PlanAction::NeedsVerification {
                        reason: SKIPPED_ENTRY_REASON,
                    }
                };
                assert_eq!(item.action, expected, "{}", item.relative_path);
            }
        }
    }

    #[test]
    fn case_and_unicode_name_collisions_are_never_executable() {
        for direction in [SyncDirection::LeftToRight, SyncDirection::Bidirectional] {
            let items = plan(
                &scenario(direction),
                &reference(),
                &[],
                &[
                    file("Report.txt", "l1", None),
                    file("caf\u{e9}.txt", "l2", None),
                    file("Dir/inner.txt", "l3", None),
                    file("plain.txt", "l4", None),
                ],
                &[
                    file("report.txt", "r1", None),
                    file("cafe\u{301}.txt", "r2", None),
                    file("dir/inner.txt", "r3", None),
                ],
            )
            .unwrap();
            assert_eq!(items.len(), 7);
            for item in &items {
                let expected = if item.relative_path == "plain.txt" {
                    PlanAction::Create { from_left: true }
                } else {
                    PlanAction::NeedsVerification {
                        reason: NAME_COLLISION_REASON,
                    }
                };
                assert_eq!(item.action, expected, "{}", item.relative_path);
            }
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

    fn deleted_on_left(count: usize) -> (Vec<FileSyncState>, Vec<ObservedFile>) {
        (0..count)
            .map(|index| {
                let old = file(&format!("gone-{index:03}.txt"), "old", None);
                (
                    FileSyncState {
                        scenario: reference(),
                        relative_path: old.relative_path.clone(),
                        left: Some(confirmed(&old)),
                        right: Some(confirmed(&old)),
                    },
                    old,
                )
            })
            .unzip()
    }

    fn simple(selected_paths: &[&str]) -> ScenarioScope {
        ScenarioScope {
            selected_paths: selected_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            deletion_mode: DeletionMode::Simple,
        }
    }

    #[test]
    fn selection_removes_outside_paths_from_the_plan() {
        let scope = ScenarioScope {
            selected_paths: vec!["Docs".into(), "notes.txt".into()],
            deletion_mode: DeletionMode::Paranoid,
        };
        let left = [
            file("Docs/a.txt", "a", None),
            file("Docsy/b.txt", "b", None),
            file("notes.txt", "n", None),
            file("other.txt", "o", None),
        ];
        let items = plan_scoped(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[],
            &left,
            &[],
            &[],
            &[],
            &scope,
        )
        .unwrap();
        let paths = items
            .iter()
            .map(|item| item.relative_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["Docs/a.txt", "notes.txt"]);
        assert!(items
            .iter()
            .all(|item| item.action == PlanAction::Create { from_left: true }));
    }

    #[test]
    fn a_name_variant_of_a_selected_object_is_never_executable() {
        let items = plan_scoped(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[],
            &[file("docs/a.txt", "a", None)],
            &[],
            &[],
            &[],
            &simple(&["Docs"]),
        )
        .unwrap();
        assert_eq!(
            items[0].action,
            PlanAction::NeedsVerification {
                reason: SELECTION_NAME_VARIANT_REASON
            }
        );
    }

    #[test]
    fn simple_rule_makes_deletions_executable_up_to_the_threshold() {
        let (baseline, right) = deleted_on_left(MASS_DELETION_THRESHOLD);
        let items = plan_scoped(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &baseline,
            &[],
            &right,
            &[],
            &[],
            &simple(&[]),
        )
        .unwrap();
        assert_eq!(items.len(), MASS_DELETION_THRESHOLD);
        assert!(items
            .iter()
            .all(|item| item.action == PlanAction::Delete { on_left: false }));
    }

    #[test]
    fn a_mass_deletion_needs_a_decision_even_with_the_simple_rule() {
        let (baseline, right) = deleted_on_left(MASS_DELETION_THRESHOLD + 1);
        let items = plan_scoped(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &baseline,
            &[],
            &right,
            &[],
            &[],
            &simple(&[]),
        )
        .unwrap();
        assert!(items
            .iter()
            .all(|item| item.action == PlanAction::DeleteRequiresDecision { on_left: false }));
    }

    #[test]
    fn simple_rule_never_deletes_an_object_changed_on_the_other_side() {
        let old = file("gone.txt", "old", None);
        let changed = file("gone.txt", "new", None);
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "gone.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let items = plan_scoped(
            &scenario(SyncDirection::LeftToRight),
            &reference(),
            &[baseline],
            &[],
            &[changed],
            &[],
            &[],
            &simple(&[]),
        )
        .unwrap();
        assert!(matches!(items[0].action, PlanAction::Conflict { .. }));
    }
}
