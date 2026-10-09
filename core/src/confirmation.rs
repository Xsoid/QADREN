//! Evidence of an operation is not the same as successful synchronization.
//! In particular, a local iCloud write cannot prove remote cloud upload.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceLevel {
    /// A request was prepared or accepted without evidence of completed effect.
    Requested,
    /// A provider or local OS acknowledged completion at its own boundary.
    ProviderAcknowledged,
    /// The target was read back and its metadata verified.
    MetadataVerified,
    /// The target's bytes were verified against the expected content.
    ContentVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Confirmed(EvidenceLevel),
    Unknown,
}

impl Confirmation {
    /// Never convert missing or ambiguous evidence into a success.
    pub fn meets(self, minimum: EvidenceLevel) -> bool {
        match self {
            Self::Confirmed(evidence) => evidence >= minimum,
            Self::Unknown => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateTransitionError {
    InvalidTransition,
    InsufficientEvidence,
}

use crate::model::JobState;

impl JobState {
    /// Pure transition guard. The future durable executor must additionally
    /// verify atomic persistence, ownership and the relevant operation result.
    pub fn transition(
        self,
        next: Self,
        confirmation: Confirmation,
    ) -> Result<Self, StateTransitionError> {
        use JobState::{Completed, Failed, Paused, Queued, Running, Unknown, WaitingForDecision};
        let allowed = matches!(
            (self, next),
            (Queued, Running)
                | (Queued, Paused)
                | (Queued, Failed)
                | (Running, Paused)
                | (Running, WaitingForDecision)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Unknown)
                | (Paused, Queued)
                | (Paused, Running)
                | (WaitingForDecision, Queued)
                | (WaitingForDecision, Running)
                | (WaitingForDecision, Failed)
                | (Unknown, WaitingForDecision)
                | (Unknown, Completed)
                | (Unknown, Failed)
        );
        if !allowed {
            return Err(StateTransitionError::InvalidTransition);
        }
        if next == Completed && !confirmation.meets(EvidenceLevel::ProviderAcknowledged) {
            return Err(StateTransitionError::InsufficientEvidence);
        }
        // UNKNOWN cannot be resumed/retried before it is explicitly resolved.
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive state/evidence matrix plus deterministic generated sequences.
    /// No external provider, credentials or file contents are involved.
    #[test]
    fn property_transition_invariants_hold_for_all_states_and_evidence() {
        use JobState::{Completed, Failed, Paused, Queued, Running, Unknown, WaitingForDecision};
        let states = [
            Queued,
            Running,
            Paused,
            WaitingForDecision,
            Completed,
            Failed,
            Unknown,
        ];
        let evidence = [
            Confirmation::Unknown,
            Confirmation::Confirmed(EvidenceLevel::Requested),
            Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged),
            Confirmation::Confirmed(EvidenceLevel::MetadataVerified),
            Confirmation::Confirmed(EvidenceLevel::ContentVerified),
        ];
        for from in states {
            for to in states {
                for proof in evidence {
                    let outcome = from.transition(to, proof);
                    if from.is_terminal() {
                        assert!(outcome.is_err(), "terminal state must be immutable");
                    }
                    if to == Completed && !proof.meets(EvidenceLevel::ProviderAcknowledged) {
                        assert!(outcome.is_err(), "completion without evidence");
                    }
                    if from == Unknown && matches!(to, Queued | Running | Paused) {
                        assert!(outcome.is_err(), "UNKNOWN cannot blindly restart");
                    }
                    if let Ok(actual) = outcome {
                        assert_eq!(actual, to);
                    }
                }
            }
        }
        // Deterministically exercise longer transition chains with a fixed seed.
        let mut seed = 0x9e37_79b9_u64;
        for _ in 0..256 {
            let mut current = Queued;
            for _ in 0..64 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let proposed = states[(seed as usize) % states.len()];
                let proof = evidence[((seed >> 8) as usize) % evidence.len()];
                if let Ok(next) = current.transition(proposed, proof) {
                    assert!(!(current.is_terminal()));
                    assert!(next != Completed || proof.meets(EvidenceLevel::ProviderAcknowledged));
                    current = next;
                }
            }
        }
    }

    #[test]
    fn local_request_cannot_prove_completed_job() {
        assert_eq!(
            JobState::Running.transition(
                JobState::Completed,
                Confirmation::Confirmed(EvidenceLevel::Requested)
            ),
            Err(StateTransitionError::InsufficientEvidence)
        );
        assert_eq!(
            JobState::Running.transition(JobState::Completed, Confirmation::Unknown),
            Err(StateTransitionError::InsufficientEvidence)
        );
    }

    #[test]
    fn confirmed_effect_allows_completion_but_not_cloud_integrity_claim() {
        let provider = Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged);
        assert_eq!(
            JobState::Running.transition(JobState::Completed, provider),
            Ok(JobState::Completed)
        );
        assert!(!provider.meets(EvidenceLevel::ContentVerified));
    }

    #[test]
    fn unknown_must_be_reconciled_and_cannot_blindly_retry() {
        assert_eq!(
            JobState::Running.transition(JobState::Unknown, Confirmation::Unknown),
            Ok(JobState::Unknown)
        );
        assert_eq!(
            JobState::Unknown.transition(JobState::Running, Confirmation::Unknown),
            Err(StateTransitionError::InvalidTransition)
        );
        assert_eq!(
            JobState::Unknown.transition(
                JobState::Completed,
                Confirmation::Confirmed(EvidenceLevel::MetadataVerified)
            ),
            Ok(JobState::Completed)
        );
    }

    #[test]
    fn completed_state_is_immutable() {
        assert_eq!(
            JobState::Completed.transition(JobState::Running, Confirmation::Unknown),
            Err(StateTransitionError::InvalidTransition)
        );
    }

    #[test]
    fn pause_and_conflict_reentry_require_explicit_transition() {
        assert_eq!(
            JobState::Running.transition(JobState::WaitingForDecision, Confirmation::Unknown),
            Ok(JobState::WaitingForDecision)
        );
        assert_eq!(
            JobState::WaitingForDecision.transition(JobState::Queued, Confirmation::Unknown),
            Ok(JobState::Queued)
        );
    }
}
