//! Pure domain models for sync planning. No I/O, credentials or persistence here.
//! Persistence and ABI contracts are defined separately before use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEndpoint {
    pub connection_id: String,
    /// Path within this connection, never a host-global filesystem path.
    pub root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    /// Only left-side changes propagate to right.
    LeftToRight,
    /// Only right-side changes propagate to left.
    RightToLeft,
    /// Both endpoints may contribute changes, conflicts require policy.
    Bidirectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Paused,
    WaitingForDecision,
    Completed,
    Failed,
    /// The user stopped this Job before an operation attempt could start.
    /// A cancellation is never used to hide an active or unknown effect.
    Cancelled,
    Unknown,
}

impl JobState {
    /// Completed requires a verified effect. Failed means the executor proved
    /// it stopped without an ambiguous effect. Cancelled means no effect
    /// attempt was active when the user stopped the Job. Unknown is never
    /// treated as a failure eligible for blind retry.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioDraft {
    pub name: String,
    pub left: StorageEndpoint,
    pub right: StorageEndpoint,
    pub direction: SyncDirection,
    pub filters: ScenarioFilters,
    pub policies: ScenarioPolicies,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScenarioFilters {
    /// Relative paths ignored by the scenario; interpreted by the adapter.
    pub excluded_patterns: Vec<String>,
}

/// How a Scenario revision treats a deletion that one side made after a
/// confirmed synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeletionMode {
    /// Every deletion waits for an explicit user decision and consent.
    #[default]
    Paranoid,
    /// The user approved one durable rule: delete the same confirmed,
    /// unchanged object on the other side. A plan with more deletions than
    /// `MASS_DELETION_THRESHOLD` still waits for an explicit decision.
    Simple,
}

impl DeletionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paranoid => "paranoid",
            Self::Simple => "simple",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "paranoid" => Some(Self::Paranoid),
            "simple" => Some(Self::Simple),
            _ => None,
        }
    }
}

/// A plan with more automatic deletions than this is a mass deletion. All of
/// its deletions then require an explicit decision (owner decision D-019).
pub const MASS_DELETION_THRESHOLD: usize = 20;

/// The objects of a Scenario revision and its deletion rule. An empty
/// selection means the complete root folder on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScenarioScope {
    /// Relative paths under each root. A directory includes all its content.
    pub selected_paths: Vec<String>,
    pub deletion_mode: DeletionMode,
}

/// The relation of one relative path to a Scenario selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeMatch {
    Inside,
    /// The path is outside by bytes, but on a case-insensitive or
    /// Unicode-normalizing volume it can name a selected object.
    NameVariant,
    Outside,
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && !path.chars().any(char::is_control)
}

fn is_inside(path: &str, selected: &str) -> bool {
    path == selected
        || (path.len() > selected.len()
            && path.starts_with(selected)
            && path.as_bytes()[selected.len()] == b'/')
}

impl ScenarioScope {
    /// Validate and normalize a selection: sorted, without duplicates, and
    /// without paths that a selected ancestor directory already includes.
    pub fn normalized_selection(paths: &[String]) -> Result<Vec<String>, ValidationError> {
        if paths.iter().any(|path| !valid_relative_path(path)) {
            return Err(ValidationError::InvalidSelection);
        }
        let mut sorted = paths.to_vec();
        sorted.sort();
        sorted.dedup();
        let mut result: Vec<String> = Vec::new();
        for path in sorted {
            if !result.iter().any(|kept| is_inside(&path, kept)) {
                result.push(path);
            }
        }
        Ok(result)
    }

    /// Classify a relative path. `name_key` is the conservative comparison
    /// key of the planner (NFC and lowercase).
    pub fn classify(&self, path: &str, name_key: impl Fn(&str) -> String) -> ScopeMatch {
        if self.selected_paths.is_empty()
            || self
                .selected_paths
                .iter()
                .any(|selected| is_inside(path, selected))
        {
            return ScopeMatch::Inside;
        }
        let key = name_key(path);
        if self
            .selected_paths
            .iter()
            .any(|selected| is_inside(&key, &name_key(selected)))
        {
            ScopeMatch::NameVariant
        } else {
            ScopeMatch::Outside
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    RequireDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionPolicy {
    RequireDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioPolicies {
    pub conflicts: ConflictPolicy,
    pub deletions: DeletionPolicy,
}

impl Default for ScenarioPolicies {
    fn default() -> Self {
        Self {
            conflicts: ConflictPolicy::RequireDecision,
            deletions: DeletionPolicy::RequireDecision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    EmptyName,
    EmptyConnectionId,
    EmptyRoot,
    IdenticalEndpoints,
    InvalidFilter,
    InvalidSelection,
}

impl ScenarioDraft {
    /// Performs syntax-level checks without touching remote or local content.
    /// Root overlaps must be resolved by the corresponding storage adapter.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName);
        }
        if self.left.connection_id.is_empty() || self.right.connection_id.is_empty() {
            return Err(ValidationError::EmptyConnectionId);
        }
        if self.left.root.is_empty() || self.right.root.is_empty() {
            return Err(ValidationError::EmptyRoot);
        }
        if self.left == self.right {
            return Err(ValidationError::IdenticalEndpoints);
        }
        if self.filters.excluded_patterns.iter().any(|pattern| {
            pattern.is_empty()
                || pattern.starts_with('/')
                || pattern.contains("..")
                || pattern.chars().any(char::is_control)
        }) {
            return Err(ValidationError::InvalidFilter);
        }
        // Same-connection distinct roots may be valid; containment, aliases,
        // symlinks and path canonicalization require adapter-level validation.
        // Successful draft validation is not authorization to execute.
        Ok(())
    }
}

/// An execution refers to an immutable Scenario revision.
/// A manual operation may be represented without any Scenario reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioReference {
    pub scenario_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobIdentity {
    pub job_id: String,
    pub scenario: Option<ScenarioReference>,
}

/// The last *confirmed* state of one object across an endpoint pair.
/// A scan result or a requested transfer must never overwrite this baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSyncState {
    pub scenario: ScenarioReference,
    pub relative_path: String,
    pub left: Option<ConfirmedFileVersion>,
    pub right: Option<ConfirmedFileVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedFileVersion {
    pub object_revision: Option<String>,
    pub size: u64,
    /// Hex digest of content, present only when established by a trusted source.
    pub checksum_md5: Option<String>,
}

/// Descriptive event emitted during an execution attempt. Not a verified receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEvent {
    pub job_id: String,
    pub attempt_id: String,
    /// Ordered within one attempt. Persistence assigns and validates ordering.
    pub sequence: u64,
    pub kind: ExecutionEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionEventKind {
    Started,
    ProgressObserved,
    AwaitingDecision,
    FinishedClaimed,
    OutcomeUnknown,
}

/// An executor's report is descriptive; it never grants completion by itself.
/// Confirmation requires the independent evidence contract in confirmation.rs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    pub job_id: String,
    pub attempt_id: String,
    pub outcome: ExecutionOutcome,
    pub message_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOutcome {
    SucceededClaimed,
    FailedClaimed,
    Uncertain,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(connection: &str, root: &str) -> StorageEndpoint {
        StorageEndpoint {
            connection_id: connection.into(),
            root: root.into(),
        }
    }

    fn draft() -> ScenarioDraft {
        ScenarioDraft {
            name: "Documents".into(),
            left: endpoint("yandex-1", "/docs"),
            right: endpoint("icloud-1", "/QADREN"),
            direction: SyncDirection::Bidirectional,
            filters: ScenarioFilters::default(),
            policies: ScenarioPolicies::default(),
        }
    }

    #[test]
    fn independent_endpoints_are_valid() {
        assert_eq!(draft().validate(), Ok(()));
    }

    #[test]
    fn allows_distinct_roots_on_the_same_connection_for_later_adapter_validation() {
        let mut scenario = draft();
        scenario.right = scenario.left.clone();
        assert_eq!(
            scenario.validate(),
            Err(ValidationError::IdenticalEndpoints)
        );
        scenario.right.root = "/other".into();
        assert_eq!(scenario.validate(), Ok(()));
    }

    #[test]
    fn rejects_incomplete_scenario_without_io() {
        let mut scenario = draft();
        scenario.name = " ".into();
        assert_eq!(scenario.validate(), Err(ValidationError::EmptyName));
        scenario = draft();
        scenario.left.connection_id.clear();
        assert_eq!(scenario.validate(), Err(ValidationError::EmptyConnectionId));
        scenario = draft();
        scenario.right.root.clear();
        assert_eq!(scenario.validate(), Err(ValidationError::EmptyRoot));
    }

    #[test]
    fn scenario_filters_reject_unsafe_patterns_and_policies_default_to_decisions() {
        let mut scenario = draft();
        scenario.filters.excluded_patterns.push("../escape".into());
        assert_eq!(scenario.validate(), Err(ValidationError::InvalidFilter));
        scenario.filters.excluded_patterns = vec!["logs/*.tmp".into()];
        assert_eq!(scenario.validate(), Ok(()));
        assert_eq!(scenario.policies.conflicts, ConflictPolicy::RequireDecision);
        assert_eq!(scenario.policies.deletions, DeletionPolicy::RequireDecision);
    }

    #[test]
    fn unknown_is_not_a_terminal_success_or_retryable_failure() {
        assert!(!JobState::Unknown.is_terminal());
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::WaitingForDecision.is_terminal());
    }

    #[test]
    fn last_confirmed_state_is_separate_from_worker_claims() {
        let baseline = FileSyncState {
            scenario: ScenarioReference {
                scenario_id: "s".into(),
                revision: 2,
            },
            relative_path: "docs/a.txt".into(),
            left: Some(ConfirmedFileVersion {
                object_revision: Some("v1".into()),
                size: 3,
                checksum_md5: None,
            }),
            right: None,
        };
        let report = ExecutionReport {
            job_id: "job".into(),
            attempt_id: "attempt".into(),
            outcome: ExecutionOutcome::SucceededClaimed,
            message_code: None,
        };
        assert_eq!(baseline.scenario.revision, 2);
        assert!(baseline.right.is_none());
        assert_eq!(report.outcome, ExecutionOutcome::SucceededClaimed);
        // No method converts a claimed report into a confirmed baseline.
    }

    #[test]
    fn execution_event_is_not_completion_evidence() {
        let event = ExecutionEvent {
            job_id: "j".into(),
            attempt_id: "a".into(),
            sequence: 1,
            kind: ExecutionEventKind::FinishedClaimed,
        };
        assert_eq!(event.kind, ExecutionEventKind::FinishedClaimed);
        assert_eq!(
            JobState::Running.transition(
                JobState::Completed,
                crate::confirmation::Confirmation::Unknown
            ),
            Err(crate::confirmation::StateTransitionError::InsufficientEvidence)
        );
    }

    #[test]
    fn manual_job_has_no_scenario_and_revision_is_explicit() {
        let manual = JobIdentity {
            job_id: "manual-1".into(),
            scenario: None,
        };
        let planned = JobIdentity {
            job_id: "job-2".into(),
            scenario: Some(ScenarioReference {
                scenario_id: "scenario-1".into(),
                revision: 3,
            }),
        };
        assert_eq!(manual.scenario, None);
        assert_eq!(planned.scenario.unwrap().revision, 3);
    }

    #[test]
    fn selection_is_normalized_and_rejects_unsafe_paths() {
        let paths = vec![
            "Docs/a.txt".to_owned(),
            "Docs".to_owned(),
            "Photos/2026".to_owned(),
            "Docs".to_owned(),
            "Docsy".to_owned(),
        ];
        assert_eq!(
            ScenarioScope::normalized_selection(&paths),
            Ok(vec!["Docs".to_owned(), "Docsy".to_owned(), "Photos/2026".to_owned()])
        );
        for unsafe_path in ["", "/abs", "a/../b", "a//b", "./a", "a\u{7}b"] {
            assert_eq!(
                ScenarioScope::normalized_selection(&[unsafe_path.to_owned()]),
                Err(ValidationError::InvalidSelection),
                "{unsafe_path:?}"
            );
        }
    }

    #[test]
    fn selection_classifies_inside_name_variant_and_outside_paths() {
        let key = |value: &str| value.to_lowercase();
        let all = ScenarioScope::default();
        assert_eq!(all.classify("any/file", key), ScopeMatch::Inside);
        let scope = ScenarioScope {
            selected_paths: vec!["Docs".to_owned(), "notes.txt".to_owned()],
            deletion_mode: DeletionMode::Simple,
        };
        assert_eq!(scope.classify("Docs", key), ScopeMatch::Inside);
        assert_eq!(scope.classify("Docs/a/b.txt", key), ScopeMatch::Inside);
        assert_eq!(scope.classify("notes.txt", key), ScopeMatch::Inside);
        assert_eq!(scope.classify("docs/a.txt", key), ScopeMatch::NameVariant);
        assert_eq!(scope.classify("Docsy/a.txt", key), ScopeMatch::Outside);
        assert_eq!(scope.classify("notes.txt.bak", key), ScopeMatch::Outside);
        assert_eq!(scope.classify("other", key), ScopeMatch::Outside);
    }

    #[test]
    fn deletion_mode_round_trips_and_defaults_to_paranoid() {
        assert_eq!(DeletionMode::default(), DeletionMode::Paranoid);
        for mode in [DeletionMode::Paranoid, DeletionMode::Simple] {
            assert_eq!(DeletionMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(DeletionMode::parse("auto"), None);
    }
}
