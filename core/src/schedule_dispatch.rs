//! Crash-safe hand-off from Registry schedule receipts to the durable Job queue.
//!
//! The databases are deliberately separate. A Registry transaction first
//! commits an outbox record with the schedule receipt; dispatch can then be
//! retried until JobStore contains the exact immutable Scenario reference.
use crate::jobs::{JobStore, JobStoreError};
use crate::registry::{Registry, RegistryError, ScheduleOutboxEntry};

#[derive(Debug)]
pub enum ScheduleDispatchError {
    Registry(RegistryError),
    JobStore(JobStoreError),
    JobIdentityMismatch,
}

impl From<RegistryError> for ScheduleDispatchError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}
impl From<JobStoreError> for ScheduleDispatchError {
    fn from(value: JobStoreError) -> Self {
        Self::JobStore(value)
    }
}

/// Submit every pending schedule outbox item once. Duplicate Job IDs are not
/// an error if and only if the existing Job has the immutable Scenario
/// identity recorded by the outbox; this is the recovery path after a crash
/// between JobStore commit and Registry acknowledgement.
pub fn dispatch_pending_schedule_jobs(
    registry: &Registry,
    jobs: &mut JobStore,
) -> Result<usize, ScheduleDispatchError> {
    let pending = registry.pending_schedule_job_outbox()?;
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
    entry: &ScheduleOutboxEntry,
) -> Result<(), ScheduleDispatchError> {
    match jobs.enqueue(
        &entry.job_id,
        Some((&entry.scenario_id, entry.scenario_revision)),
    ) {
        Ok(()) => {}
        Err(JobStoreError::DuplicateJob) => {
            let existing = jobs
                .get(&entry.job_id)?
                .ok_or(ScheduleDispatchError::JobIdentityMismatch)?;
            if existing.scenario_id.as_deref() != Some(entry.scenario_id.as_str())
                || existing.scenario_revision != Some(entry.scenario_revision)
            {
                return Err(ScheduleDispatchError::JobIdentityMismatch);
            }
        }
        Err(error) => return Err(error.into()),
    }
    registry.mark_schedule_job_dispatched(entry.id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }
    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "qadren-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn schedule_outbox_survives_restart_and_dispatches_exactly_one_pinned_job() {
        let registry_path = path("schedule-registry.sqlite3");
        let job_path = path("schedule-jobs.sqlite3");
        let registry = Registry::open(&registry_path).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario("Documents", &left, "/", &right, "/archive", "left_to_right")
            .unwrap();
        registry
            .create_scenario_schedule(&scenario, "UTC", 0, 9, 30)
            .unwrap();
        assert_eq!(
            registry
                .stage_due_schedule_occurrences(
                    utc("2026-10-12T09:29:00Z"),
                    utc("2026-10-12T09:30:00Z")
                )
                .unwrap(),
            1
        );
        drop(registry);

        let registry = Registry::open(&registry_path).unwrap();
        let mut jobs = JobStore::open(&job_path).unwrap();
        assert_eq!(
            dispatch_pending_schedule_jobs(&registry, &mut jobs).unwrap(),
            1
        );
        assert!(registry.pending_schedule_job_outbox().unwrap().is_empty());
        let listed = jobs.list_jobs().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].scenario_id.as_deref(), Some(scenario.as_str()));
        assert_eq!(listed[0].scenario_revision, Some(1));
        assert_eq!(
            dispatch_pending_schedule_jobs(&registry, &mut jobs).unwrap(),
            0
        );
        std::fs::remove_file(registry_path).unwrap();
        std::fs::remove_file(job_path).unwrap();
    }

    #[test]
    fn restart_after_job_commit_acknowledges_matching_existing_job_without_replay() {
        let registry_path = path("schedule-recovery-registry.sqlite3");
        let job_path = path("schedule-recovery-jobs.sqlite3");
        let registry = Registry::open(&registry_path).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario("Documents", &left, "/", &right, "/archive", "left_to_right")
            .unwrap();
        registry
            .create_scenario_schedule(&scenario, "UTC", 0, 9, 30)
            .unwrap();
        registry
            .stage_due_schedule_occurrences(
                utc("2026-10-12T09:29:00Z"),
                utc("2026-10-12T09:30:00Z"),
            )
            .unwrap();
        let entry = registry
            .pending_schedule_job_outbox()
            .unwrap()
            .pop()
            .unwrap();
        let mut jobs = JobStore::open(&job_path).unwrap();
        // Simulate a process crash after the JobStore commit but before the
        // Registry outbox acknowledgement.
        jobs.enqueue(&entry.job_id, Some((&scenario, 1))).unwrap();
        assert_eq!(
            dispatch_pending_schedule_jobs(&registry, &mut jobs).unwrap(),
            1
        );
        assert!(registry.pending_schedule_job_outbox().unwrap().is_empty());
        assert_eq!(jobs.list_jobs().unwrap().len(), 1);
        std::fs::remove_file(registry_path).unwrap();
        std::fs::remove_file(job_path).unwrap();
    }
}
