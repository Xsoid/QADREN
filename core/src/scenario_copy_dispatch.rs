//! Crash-safe hand-off from completed Scenario copies to Registry baselines.
//!
//! JobStore commits the copy result and this outbox record together. Registry
//! accepts the revision-bound baseline later. Repeating this delivery is safe.
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::jobs::{JobStore, JobStoreError, ScenarioCopyBaselineOutboxEntry};
use crate::model::FileSyncState;
use crate::registry::{Registry, RegistryError};

#[derive(Debug)]
pub enum ScenarioCopyBaselineDispatchError {
    JobStore(JobStoreError),
    Registry(RegistryError),
}

impl From<JobStoreError> for ScenarioCopyBaselineDispatchError {
    fn from(value: JobStoreError) -> Self {
        Self::JobStore(value)
    }
}

impl From<RegistryError> for ScenarioCopyBaselineDispatchError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}

/// Deliver all completed Scenario-copy baselines once. This has no adapter
/// effect and never retries a copy. A crash after Registry accepts a baseline
/// is recovered by the Registry UPSERT on the next call.
pub fn dispatch_pending_scenario_copy_baselines(
    registry: &Registry,
    jobs: &mut JobStore,
) -> Result<usize, ScenarioCopyBaselineDispatchError> {
    let pending = jobs.pending_scenario_copy_baseline_outbox()?;
    let mut dispatched = 0;
    for entry in pending {
        dispatch_one(registry, jobs, &entry)?;
        dispatched += 1;
    }
    Ok(dispatched)
}

fn dispatch_one(
    registry: &Registry,
    jobs: &mut JobStore,
    entry: &ScenarioCopyBaselineOutboxEntry,
) -> Result<(), ScenarioCopyBaselineDispatchError> {
    registry.record_confirmed_file_state(
        &FileSyncState {
            scenario: entry.scenario.clone(),
            relative_path: entry.relative_path.clone(),
            left: Some(entry.left.clone()),
            right: Some(entry.right.clone()),
        },
        Confirmation::Confirmed(EvidenceLevel::MetadataVerified),
    )?;
    jobs.mark_scenario_copy_baseline_dispatched(&entry.job_id)?;
    Ok(())
}
