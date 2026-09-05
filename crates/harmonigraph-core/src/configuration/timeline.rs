//! Retained configuration boundaries. Command handoff capacity is independent:
//! draining its queue does not retire a marker needed by unbound requests.

use super::{ConfigMutation, ConfigReducer, ResolvedConfig};

pub const CONFIG_COMMANDS: usize = 128;
pub const CONFIG_SLOTS: usize = 2;
pub const CONFIG_TIMELINE: usize = 128;
pub const CONTROL_WORK: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOrigin {
    Ui,
    Restore,
    Automation,
    Flush,
    Learning,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfigCommand {
    /// UI/restore identity, unrelated to musical revision. Zero for host/learn.
    pub command_id: u64,
    pub origin: ConfigOrigin,
    pub mutation: ConfigMutation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfigMarker {
    pub observed_sample: i64,
    pub effective_sample: i64,
    pub order: u64,
    pub command: ConfigCommand,
    pub resolved: Option<ResolvedConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineError {
    WorkPending,
    StorageFull,
    CounterExhausted,
    PendingBoundary,
    InvalidFrontier,
}

/// One budget for the enclosing callback, shared across all sub-blocks and
/// insertion/application/retirement. Exhaustion retains the caller's input.
pub struct ControlBudget {
    remaining: usize,
}
impl Default for ControlBudget {
    fn default() -> Self {
        Self { remaining: CONTROL_WORK }
    }
}
impl ControlBudget {
    fn charge(&mut self) -> Result<(), TimelineError> {
        if self.remaining == 0 {
            return Err(TimelineError::WorkPending);
        }
        self.remaining -= 1;
        Ok(())
    }
    pub fn remaining(&self) -> usize {
        self.remaining
    }
}

pub struct ConfigTimeline {
    reducer: ConfigReducer,
    markers: [Option<ConfigMarker>; CONFIG_TIMELINE],
    head: usize,
    len: usize,
    applied: usize,
    order: u64,
    finalized_exclusive: i64,
    bindings_copied_exclusive: i64,
    started_cohort: Option<i64>,
    retired: ResolvedConfig,
    pub storage_fault: bool,
}

impl Default for ConfigTimeline {
    fn default() -> Self {
        Self::new(ConfigReducer::default())
    }
}

impl ConfigTimeline {
    pub fn new(reducer: ConfigReducer) -> Self {
        let retired = reducer.resolved();
        Self {
            reducer,
            markers: [None; CONFIG_TIMELINE],
            head: 0,
            len: 0,
            applied: 0,
            order: 0,
            finalized_exclusive: 0,
            bindings_copied_exclusive: 0,
            started_cohort: None,
            retired,
            storage_fault: false,
        }
    }
    pub fn reducer(&self) -> &ConfigReducer {
        &self.reducer
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn pending(&self) -> usize {
        self.len - self.applied
    }
    pub fn finalized_exclusive(&self) -> i64 {
        self.finalized_exclusive
    }
    pub fn markers(&self) -> impl Iterator<Item = &ConfigMarker> {
        (0..self.len).map(|i| self.markers[(self.head + i) % CONFIG_TIMELINE].as_ref().unwrap())
    }

    /// Fix this boundary when the owner first observes the command, before any
    /// deferral. An untimed flush must wait for an explicit sample before here.
    pub fn effective_at(&self, observed: i64) -> Result<i64, TimelineError> {
        let mut effective = observed.max(self.finalized_exclusive);
        if let Some(started) = self.started_cohort {
            if effective <= started {
                effective = started.checked_add(1).ok_or(TimelineError::CounterExhausted)?;
            }
        }
        Ok(effective)
    }

    /// A UI/restore producer retains its command on StorageFull. Required host
    /// automation whose owner cannot retain it must call `required_storage_fault`.
    pub fn insert(
        &mut self,
        command: ConfigCommand,
        observed_sample: i64,
        effective_sample: i64,
        budget: &mut ControlBudget,
    ) -> Result<u64, TimelineError> {
        if self.len == CONFIG_TIMELINE {
            return Err(TimelineError::StorageFull);
        }
        if effective_sample < self.finalized_exclusive
            || self.markers().last().is_some_and(|last| effective_sample < last.effective_sample)
        {
            return Err(TimelineError::InvalidFrontier);
        }
        let order = self.order.checked_add(1).ok_or(TimelineError::CounterExhausted)?;
        budget.charge()?;
        self.markers[(self.head + self.len) % CONFIG_TIMELINE] = Some(ConfigMarker {
            observed_sample,
            effective_sample,
            order,
            command,
            resolved: None,
        });
        self.order = order;
        self.len += 1;
        Ok(order)
    }

    pub fn required_storage_fault(&mut self) {
        self.storage_fault = true;
    }

    pub fn apply_next(
        &mut self,
        budget: &mut ControlBudget,
    ) -> Result<Option<ConfigMarker>, TimelineError> {
        if self.applied == self.len {
            return Ok(None);
        }
        budget.charge()?;
        let cell = &mut self.markers[(self.head + self.applied) % CONFIG_TIMELINE];
        let marker = cell.as_mut().unwrap();
        if !self.reducer.apply(marker.command.mutation) {
            return Err(TimelineError::CounterExhausted);
        }
        marker.resolved = Some(self.reducer.resolved());
        self.applied += 1;
        Ok(Some(*marker))
    }

    /// Freeze one coherent configuration for a whole cohort. A later command
    /// cannot interrupt it. Requests keep this copied value through rescheduling.
    pub fn begin_cohort(&mut self, sample: i64) -> Result<ResolvedConfig, TimelineError> {
        if sample < self.finalized_exclusive {
            return Err(TimelineError::InvalidFrontier);
        }
        if self.started_cohort.is_some_and(|started| started != sample) {
            return Err(TimelineError::PendingBoundary);
        }
        let config = self.configuration_at(sample)?;
        self.started_cohort = Some(sample);
        Ok(config)
    }

    pub fn configuration_at(&self, sample: i64) -> Result<ResolvedConfig, TimelineError> {
        if sample < self.finalized_exclusive || self.storage_fault {
            return Err(TimelineError::InvalidFrontier);
        }
        let mut config = self.retired;
        for marker in self.markers() {
            if marker.effective_sample > sample {
                break;
            }
            config = marker.resolved.ok_or(TimelineError::PendingBoundary)?;
        }
        Ok(config)
    }

    /// Integration seam for the later sequencer. Both frontiers are proofs from
    /// its owner, not clock guesses: all cohorts finalized and all request copies
    /// made below the supplied exclusive boundaries. Pending markers are barriers.
    /// The integrating owner must also exclude every earlier command/input it
    /// still holds outside this timeline; such records cannot be inferred here.
    pub fn advance_frontiers(
        &mut self,
        finalized: i64,
        bindings_copied: i64,
    ) -> Result<(), TimelineError> {
        if self.storage_fault
            || finalized < self.finalized_exclusive
            || bindings_copied < self.bindings_copied_exclusive
            || bindings_copied > finalized
            || self.markers().any(|m| m.effective_sample < finalized && m.resolved.is_none())
        {
            return Err(TimelineError::InvalidFrontier);
        }
        self.finalized_exclusive = finalized;
        self.bindings_copied_exclusive = bindings_copied;
        if self.started_cohort.is_some_and(|sample| sample < finalized) {
            self.started_cohort = None;
        }
        Ok(())
    }

    pub fn retire_one(&mut self, budget: &mut ControlBudget) -> Result<bool, TimelineError> {
        let Some(marker) = self.markers[self.head] else {
            return Ok(false);
        };
        let Some(config) = marker.resolved else {
            return Ok(false);
        };
        if marker.effective_sample >= self.finalized_exclusive
            || marker.effective_sample >= self.bindings_copied_exclusive
        {
            return Ok(false);
        }
        budget.charge()?;
        self.retired = config;
        self.markers[self.head] = None;
        self.head = (self.head + 1) % CONFIG_TIMELINE;
        self.len -= 1;
        self.applied -= 1;
        Ok(true)
    }
}

const _: () = assert!(std::mem::size_of::<ConfigMarker>() <= 256);
const _: () = assert!(std::mem::align_of::<ConfigMarker>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::ConfigEdit;

    fn command(id: u64) -> ConfigCommand {
        ConfigCommand {
            command_id: id,
            origin: ConfigOrigin::Automation,
            mutation: ConfigMutation::Edit(ConfigEdit::axis(0, id as i32)),
        }
    }

    #[test]
    fn drained_commands_still_retain_128_markers_and_the_129th_faults() {
        let mut timeline = ConfigTimeline::default();
        // The command handoff is drained on EVERY iteration. It cannot be the
        // capacity this fixture reaches; every retained slot is a timeline slot.
        for id in 1..=128 {
            let mut budget = ControlBudget::default();
            timeline.insert(command(id), id as i64, id as i64, &mut budget).unwrap();
            timeline.apply_next(&mut budget).unwrap().unwrap();
            assert_eq!(timeline.pending(), 0);
        }
        assert_eq!(timeline.len(), CONFIG_TIMELINE);
        assert_eq!(
            timeline.insert(command(129), 129, 129, &mut ControlBudget::default()),
            Err(TimelineError::StorageFull)
        );
        timeline.required_storage_fault();
        assert!(timeline.storage_fault);
        assert_eq!(timeline.finalized_exclusive(), 0);
        assert_eq!(timeline.markers().count(), 128);
    }

    #[test]
    fn binding_and_retirement_obey_same_sample_order_and_one_work_budget() {
        let mut timeline = ConfigTimeline::default();
        let mut budget = ControlBudget::default();
        for id in 1..=8 {
            timeline.insert(command(id), id as i64 * 2, id as i64 * 2, &mut budget).unwrap();
            timeline.apply_next(&mut budget).unwrap().unwrap();
        }
        assert_eq!(budget.remaining(), 0);
        assert_eq!(
            timeline.insert(command(9), 18, 18, &mut budget),
            Err(TimelineError::WorkPending)
        );
        for id in 1..=8 {
            let bound = timeline.begin_cohort(id * 2 + 1).unwrap();
            assert_eq!(bound.tuning.c_offset, id as i32);
            timeline.advance_frontiers(id * 2 + 2, id * 2 + 2).unwrap();
        }
        let mut budget = ControlBudget::default();
        while timeline.retire_one(&mut budget).unwrap() {}
        assert!(timeline.is_empty());
        assert_eq!(timeline.begin_cohort(20).unwrap().tuning.c_offset, 8);
        assert_eq!(timeline.effective_at(19), Ok(21));
        timeline.insert(command(9), 19, 21, &mut budget).unwrap();
        timeline.apply_next(&mut budget).unwrap();
        assert_eq!(timeline.configuration_at(20).unwrap().tuning.c_offset, 8);
        assert_eq!(timeline.configuration_at(21).unwrap().tuning.c_offset, 9);
    }
}
