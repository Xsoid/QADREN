//! Pure trigger coalescing rules for future durable scheduler storage.
//! This module never starts Jobs or performs I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticTriggerResult {
    IgnoredDisabled,
    IgnoredPaused,
    Queued,
    Coalesced,
    FollowUpQueued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioAutomationState {
    pub enabled: bool,
    pub locally_paused: bool,
    pub active_job: bool,
    /// One pending automatic run represents any size burst of changes.
    pub pending_automatic_run: bool,
}

impl Default for ScenarioAutomationState {
    fn default() -> Self {
        Self {
            enabled: true,
            locally_paused: false,
            active_job: false,
            pending_automatic_run: false,
        }
    }
}

impl ScenarioAutomationState {
    /// Coalesce any automatic trigger into at most one pending Job. An active
    /// Job is never mutated; its successor is submitted only after completion.
    pub fn request_automatic_run(&mut self) -> AutomaticTriggerResult {
        if !self.enabled {
            return AutomaticTriggerResult::IgnoredDisabled;
        }
        if self.locally_paused {
            return AutomaticTriggerResult::IgnoredPaused;
        }
        if self.pending_automatic_run {
            return AutomaticTriggerResult::Coalesced;
        }
        self.pending_automatic_run = true;
        if self.active_job {
            AutomaticTriggerResult::FollowUpQueued
        } else {
            AutomaticTriggerResult::Queued
        }
    }

    /// A manual request is deliberately not suppressed by Scenario automatic
    /// disable/pause. Durable Job submission and priority ownership are later
    /// layers; this only states that the request is admissible.
    pub fn may_queue_manual_run(&self) -> bool {
        true
    }

    /// Claim the one pending automatic run for durable Job submission.
    pub fn take_pending_automatic_run(&mut self) -> bool {
        if self.enabled && !self.locally_paused && !self.active_job && self.pending_automatic_run {
            self.pending_automatic_run = false;
            self.active_job = true;
            true
        } else {
            false
        }
    }

    /// Completion leaves a coalesced follow-up intact; caller can then claim it.
    pub fn mark_active_job_finished(&mut self) {
        self.active_job = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_coalesces_into_one_pending_run() {
        let mut state = ScenarioAutomationState::default();
        assert_eq!(
            state.request_automatic_run(),
            AutomaticTriggerResult::Queued
        );
        for _ in 0..1_000 {
            assert_eq!(
                state.request_automatic_run(),
                AutomaticTriggerResult::Coalesced
            );
        }
        assert!(state.take_pending_automatic_run());
        assert!(!state.take_pending_automatic_run());
    }

    #[test]
    fn active_job_gets_one_follow_up_without_mutation() {
        let mut state = ScenarioAutomationState {
            active_job: true,
            ..Default::default()
        };
        assert_eq!(
            state.request_automatic_run(),
            AutomaticTriggerResult::FollowUpQueued
        );
        assert_eq!(
            state.request_automatic_run(),
            AutomaticTriggerResult::Coalesced
        );
        state.mark_active_job_finished();
        assert!(state.take_pending_automatic_run());
    }

    #[test]
    fn disabled_or_paused_scenario_does_not_auto_start_but_manual_is_admissible() {
        let mut disabled = ScenarioAutomationState {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(
            disabled.request_automatic_run(),
            AutomaticTriggerResult::IgnoredDisabled
        );
        assert!(disabled.may_queue_manual_run());
        let mut paused = ScenarioAutomationState {
            locally_paused: true,
            ..Default::default()
        };
        assert_eq!(
            paused.request_automatic_run(),
            AutomaticTriggerResult::IgnoredPaused
        );
        assert!(paused.may_queue_manual_run());
    }
}
