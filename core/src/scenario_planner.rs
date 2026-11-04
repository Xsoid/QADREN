//! Pure, conservative Scenario dry-run planning.
//! No adapter I/O, Job submission, content reads or FileSyncState writes occur
//! here. Callers must supply already-authorized metadata snapshots.
use crate::model::{
    ConfirmedFileVersion, DeletionMode, FileSyncState, ScenarioDraft, ScenarioReference,
    ScenarioScope, ScopeMatch, SyncDirection,
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
    /// The revision's approved simple deletion rule covers this deletion of
    /// a confirmed object. The executor checks again that the source object
    /// is absent.
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

/// One-way Scenario: the source always wins (D-025 section 3.1). A change
/// in the destination is replaced, and a destination removal is restored.
/// Only objects with a confirmed state are deleted (D-019 rule 4).
fn directional_action(
    source: SideChange,
    destination: SideChange,
    source_file: Option<&ObservedFile>,
    destination_file: Option<&ObservedFile>,
    from_left: bool,
) -> PlanAction {
    if source == SideChange::NeedsVerification || destination == SideChange::NeedsVerification {
        return PlanAction::NeedsVerification {
            reason: "metadata does not prove the confirmed object identity",
        };
    }
    match (source_file, destination_file) {
        // Equal content on both sides needs no effect.
        (Some(source_file), Some(destination_file))
            if same_confirmed_content(source_file, destination_file) =>
        {
            PlanAction::NoAction
        }
        (Some(_), None)
            if source != SideChange::Unchanged || destination == SideChange::Removed =>
        {
            PlanAction::Create { from_left }
        }
        (Some(_), Some(_))
            if source != SideChange::Unchanged || destination != SideChange::Unchanged =>
        {
            PlanAction::Update { from_left }
        }
        // The source removed a confirmed object: delete it in the
        // destination under the deletion mode. A destination object without
        // a confirmed state was never synchronized and does not change.
        (None, Some(_)) if source == SideChange::Removed && destination != SideChange::Added => {
            PlanAction::DeleteRequiresDecision {
                on_left: !from_left,
            }
        }
        _ => PlanAction::NoAction,
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
/// makes every rule deletion executable (D-025 section 5).
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
        excluded_paths: scope.excluded_paths.clone(),
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
            plan_path(
                scenario.direction,
                &scope,
                &PathObservation {
                    path,
                    confirmed: confirmed.get(path).copied(),
                    left: left.get(path).copied(),
                    right: right.get(path).copied(),
                    skipped: skipped.contains(path.as_str()),
                },
                variants.get(&name_key(path)).copied().unwrap_or(0) > 1,
            )
        })
        .collect::<Vec<_>>();
    // D-025 section 5: the simple rule has no mass-deletion threshold.
    if scope.deletion_mode == DeletionMode::Simple {
        for item in &mut items {
            if let PlanAction::DeleteRequiresDecision { on_left } = item.action {
                item.action = PlanAction::Delete { on_left };
            }
        }
    }
    Ok(items)
}

/// What the planner knows about one relative path.
pub struct PathObservation<'a> {
    pub path: &'a str,
    pub confirmed: Option<&'a FileSyncState>,
    pub left: Option<&'a ObservedFile>,
    pub right: Option<&'a ObservedFile>,
    /// A side skipped a symbolic link or special file at this path.
    pub skipped: bool,
}

/// The plan item of one path. `collides` is true when another planned path
/// has the same name key. The streaming planner (D-024) calls this for each
/// path of a name-key group, so both planners decide the same way. The
/// simple deletion rule is applied by the caller over the whole plan.
pub fn plan_path(
    direction: SyncDirection,
    scope: &ScenarioScope,
    observation: &PathObservation<'_>,
    collides: bool,
) -> PlanItem {
    let PathObservation {
        path,
        confirmed: state,
        left: left_file,
        right: right_file,
        skipped,
    } = *observation;
    let left_change = delta(left_file, state.and_then(|entry| entry.left.as_ref()));
    let right_change = delta(right_file, state.and_then(|entry| entry.right.as_ref()));
    let action = if scope.classify(path, name_key) == ScopeMatch::NameVariant {
        PlanAction::NeedsVerification {
            reason: SELECTION_NAME_VARIANT_REASON,
        }
    } else if skipped || !valid_path(path) {
        // A name that cannot be a safe path segment is never executable.
        PlanAction::NeedsVerification {
            reason: SKIPPED_ENTRY_REASON,
        }
    } else if collides {
        PlanAction::NeedsVerification {
            reason: NAME_COLLISION_REASON,
        }
    } else {
        match direction {
            SyncDirection::LeftToRight => {
                directional_action(left_change, right_change, left_file, right_file, true)
            }
            SyncDirection::RightToLeft => {
                directional_action(right_change, left_change, right_file, left_file, false)
            }
            SyncDirection::Bidirectional => {
                bidirectional_action(left_change, right_change, left_file, right_file)
            }
        }
    };
    PlanItem {
        relative_path: path.to_owned(),
        left: left_change,
        right: right_change,
        action,
    }
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
    fn a_change_on_both_sides_is_an_update_one_way_and_a_conflict_two_way() {
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
        let action = |direction| {
            plan(
                &scenario(direction),
                &reference(),
                std::slice::from_ref(&baseline),
                std::slice::from_ref(&left),
                std::slice::from_ref(&right),
            )
            .unwrap()[0]
                .action
                .clone()
        };
        assert_eq!(
            action(SyncDirection::LeftToRight),
            PlanAction::Update { from_left: true }
        );
        assert!(matches!(
            action(SyncDirection::Bidirectional),
            PlanAction::Conflict { .. }
        ));
    }

    /// D-025 section 3.1: each row of the one-way table, from the left.
    #[test]
    fn one_way_source_always_wins() {
        let md5 = |digit: char| Some(digit.to_string().repeat(32));
        let v = |revision: &str, digit: char| file("f.txt", revision, md5(digit).as_deref());
        let state = |left: Option<&ObservedFile>, right: Option<&ObservedFile>| FileSyncState {
            scenario: reference(),
            relative_path: "f.txt".into(),
            left: left.map(confirmed),
            right: right.map(confirmed),
        };
        let (s0, s1, d0, d1) = (v("s0", '1'), v("s1", '2'), v("d0", '1'), v("d1", '3'));
        let same = v("other", '2');
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            &str,
            Option<FileSyncState>,
            Option<&ObservedFile>,
            Option<&ObservedFile>,
            PlanAction,
        )> = vec![
            (
                "added, destination absent",
                None,
                Some(&s1),
                None,
                PlanAction::Create { from_left: true },
            ),
            (
                "changed, destination absent",
                Some(state(Some(&s0), None)),
                Some(&s1),
                None,
                PlanAction::Create { from_left: true },
            ),
            (
                "changed, destination unchanged",
                Some(state(Some(&s0), Some(&d0))),
                Some(&s1),
                Some(&d0),
                PlanAction::Update { from_left: true },
            ),
            (
                "changed, destination changed",
                Some(state(Some(&s0), Some(&d0))),
                Some(&s1),
                Some(&d1),
                PlanAction::Update { from_left: true },
            ),
            (
                "added, destination added",
                None,
                Some(&s1),
                Some(&d1),
                PlanAction::Update { from_left: true },
            ),
            (
                "unchanged, destination changed",
                Some(state(Some(&s0), Some(&d0))),
                Some(&s0),
                Some(&d1),
                PlanAction::Update { from_left: true },
            ),
            (
                "unchanged, destination removed",
                Some(state(Some(&s0), Some(&d0))),
                Some(&s0),
                None,
                PlanAction::Create { from_left: true },
            ),
            (
                "removed, destination unchanged",
                Some(state(Some(&s0), Some(&d0))),
                None,
                Some(&d0),
                PlanAction::DeleteRequiresDecision { on_left: false },
            ),
            (
                "removed, destination changed",
                Some(state(Some(&s0), Some(&d0))),
                None,
                Some(&d1),
                PlanAction::DeleteRequiresDecision { on_left: false },
            ),
            (
                "removed, destination removed",
                Some(state(Some(&s0), Some(&d0))),
                None,
                None,
                PlanAction::NoAction,
            ),
            (
                "same content, no confirmed state",
                None,
                Some(&s1),
                Some(&same),
                PlanAction::NoAction,
            ),
            (
                "destination only, no confirmed state",
                None,
                None,
                Some(&d1),
                PlanAction::NoAction,
            ),
            (
                "unchanged on both sides",
                Some(state(Some(&s0), Some(&d0))),
                Some(&s0),
                Some(&d0),
                PlanAction::NoAction,
            ),
        ];
        for (name, baseline, left, right, expected) in rows {
            let items = plan(
                &scenario(SyncDirection::LeftToRight),
                &reference(),
                &baseline.into_iter().collect::<Vec<_>>(),
                &left.cloned().into_iter().collect::<Vec<_>>(),
                &right.cloned().into_iter().collect::<Vec<_>>(),
            )
            .unwrap();
            let action = items
                .first()
                .map_or(PlanAction::NoAction, |item| item.action.clone());
            assert_eq!(action, expected, "{name}");
        }
    }

    /// The same table from the right side mirrors every action.
    #[test]
    fn one_way_right_to_left_mirrors_the_rules() {
        let old = file("f.txt", "old", None);
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "f.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let items = plan(
            &scenario(SyncDirection::RightToLeft),
            &reference(),
            std::slice::from_ref(&baseline),
            &[file("f.txt", "left-changed", None)],
            std::slice::from_ref(&old),
        )
        .unwrap();
        assert_eq!(items[0].action, PlanAction::Update { from_left: false });
        let items = plan(
            &scenario(SyncDirection::RightToLeft),
            &reference(),
            &[baseline],
            std::slice::from_ref(&old),
            &[],
        )
        .unwrap();
        assert_eq!(
            items[0].action,
            PlanAction::DeleteRequiresDecision { on_left: true }
        );
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
            excluded_paths: Vec::new(),
        }
    }

    #[test]
    fn selection_removes_outside_paths_from_the_plan() {
        let scope = ScenarioScope {
            selected_paths: vec!["Docs".into(), "notes.txt".into()],
            deletion_mode: DeletionMode::Paranoid,
            excluded_paths: Vec::new(),
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

    /// D-025 section 5: the simple rule has no mass-deletion threshold.
    #[test]
    fn simple_rule_has_no_mass_deletion_threshold() {
        let (baseline, right) = deleted_on_left(500);
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
        assert_eq!(items.len(), 500);
        assert!(items
            .iter()
            .all(|item| item.action == PlanAction::Delete { on_left: false }));
    }

    /// One-way: the source wins, so a changed destination object of a
    /// removed source object is deleted. Two-way: a change against a
    /// removal stays a conflict.
    #[test]
    fn simple_rule_deletes_a_changed_destination_only_one_way() {
        let old = file("gone.txt", "old", None);
        let changed = file("gone.txt", "new", None);
        let baseline = FileSyncState {
            scenario: reference(),
            relative_path: "gone.txt".into(),
            left: Some(confirmed(&old)),
            right: Some(confirmed(&old)),
        };
        let action = |direction| {
            plan_scoped(
                &scenario(direction),
                &reference(),
                std::slice::from_ref(&baseline),
                &[],
                std::slice::from_ref(&changed),
                &[],
                &[],
                &simple(&[]),
            )
            .unwrap()[0]
                .action
                .clone()
        };
        assert_eq!(
            action(SyncDirection::LeftToRight),
            PlanAction::Delete { on_left: false }
        );
        assert!(matches!(
            action(SyncDirection::Bidirectional),
            PlanAction::Conflict { .. }
        ));
    }
}
