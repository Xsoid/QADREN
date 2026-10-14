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
}
