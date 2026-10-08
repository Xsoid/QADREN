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
    Unknown,
}

impl JobState {
    /// Completed and Failed describe a *verified* terminal result.
    /// Unknown is never treated as a failure eligible for blind retry.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioDraft {
    pub name: String,
    pub left: StorageEndpoint,
    pub right: StorageEndpoint,
    pub direction: SyncDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    EmptyName,
    EmptyConnectionId,
    EmptyRoot,
    IdenticalEndpoints,
    AmbiguousOverlap,
}

impl ScenarioDraft {
    /// Conservative admission policy: independent roots on the same connection
    /// are allowed only if neither contains the other. No content is read.
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
        // Root canonicalization is the responsibility of the storage adapter.
        // Do not attempt string-prefix filesystem safety checks here.
        if self.left.connection_id == self.right.connection_id {
            return Err(ValidationError::AmbiguousOverlap);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(connection: &str, root: &str) -> StorageEndpoint {
        StorageEndpoint { connection_id: connection.into(), root: root.into() }
    }

    fn draft() -> ScenarioDraft {
        ScenarioDraft {
            name: "Documents".into(),
            left: endpoint("yandex-1", "/docs"),
            right: endpoint("icloud-1", "/QADREN"),
            direction: SyncDirection::Bidirectional,
        }
    }

    #[test]
    fn independent_endpoints_are_valid() {
        assert_eq!(draft().validate(), Ok(()));
    }

    #[test]
    fn rejects_identical_and_same_connection_roots_until_adapter_can_prove_safety() {
        let mut scenario = draft();
        scenario.right = scenario.left.clone();
        assert_eq!(scenario.validate(), Err(ValidationError::IdenticalEndpoints));
        scenario.right.root = "/other".into();
        assert_eq!(scenario.validate(), Err(ValidationError::AmbiguousOverlap));
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
    fn unknown_is_not_a_terminal_success_or_retryable_failure() {
        assert!(!JobState::Unknown.is_terminal());
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(!JobState::WaitingForDecision.is_terminal());
    }

    #[test]
    fn manual_job_has_no_scenario_and_revision_is_explicit() {
        let manual = JobIdentity { job_id: "manual-1".into(), scenario: None };
        let planned = JobIdentity {
            job_id: "job-2".into(),
            scenario: Some(ScenarioReference { scenario_id: "scenario-1".into(), revision: 3 }),
        };
        assert_eq!(manual.scenario, None);
        assert_eq!(planned.scenario.unwrap().revision, 3);
    }
}
