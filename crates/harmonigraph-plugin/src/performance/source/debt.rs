//! Output debt: the obligations `output_settled` and `publish_seal` refuse to
//! settle on while live, and that only accepted host output can discharge. Two
//! kinds live here -- a per-channel reset (three pedals) and
//! a per-lifetime emergency termination -- because they are the only ones a
//! Source can acquire outside a callback.
//!
//! One rule lives here rather than at any caller: **past the producer's join
//! nothing mints output debt.** Destruction is the reset that never resumes.
//! The retirement pump invokes no host and never reaches `begin`, so a bit
//! armed after the join is an obligation with no callback left to discharge
//! it, and `output_settled` and `publish_seal` would both refuse this Source
//! for the life of the process -- holding its registry entry with it. What was
//! already owed at destruction is abandoned, after `join_producer` records
//! what the wire is left holding as evidence. Abandonment frees ownership;
//! it does not assert host acceptance or downstream physical termination.
//!
//! Guarding one arming caller at a time is what this replaces. Both `stop`
//! and `fault` reach these fields after a destroy has begun, including faults
//! from the join itself or a capture overflow inside the pump. Both fields
//! are private to this module, so **no route can
//! raise a pending reset bit or take a release slot except `arm` and
//! `arm_release`**: a new caller inherits the rule by construction. The other
//! two writers can only move debt that is already here -- `stage` needs the
//! pending bit and `complete` needs the staged one -- so neither is a mint.

use super::{Release, CHANNEL_RESETS};

pub(super) struct Debt {
    /// One pending bit and one staged bit per reset slot, per channel:
    /// `CHANNEL_RESETS` of each packed into a `u8`. Pending is owed, staged is
    /// in flight, and neither is the wire fact -- an accepted reset clears
    /// both and is read off `State` afterwards.
    channel: [u8; 16],
    /// One slot per wire lifetime this Source still owes a termination.
    releases: [Option<Release>; 64],
    joined: bool,
}

impl Debt {
    pub(super) fn new() -> Self {
        Self { channel: [0; 16], releases: [None; 64], joined: false }
    }

    /// The producer's final cut is immutable and no callback follows it, so
    /// from here nothing this Source could owe would ever be sent. The caller
    /// captures unknown-wire evidence first, then returns these references.
    /// Accepted output still lives in the factual journals until acknowledged.
    pub(super) fn join(&mut self) -> [Option<Release>; 64] {
        self.joined = true;
        self.channel = [0; 16];
        std::mem::replace(&mut self.releases, [None; 64])
    }

    /// Raise one channel reset. Every mint of channel debt is this call.
    pub(super) fn arm(&mut self, channel: usize, bit: usize) {
        if self.joined {
            return;
        }
        self.channel[channel] |= 1 << bit;
    }

    /// Take a slot for a wire lifetime that owes a termination, reporting
    /// whether this call is the one that took it -- the caller owes the life a
    /// reference for exactly that call. Every mint of release debt is this.
    pub(super) fn arm_release(&mut self, life: u16) -> bool {
        if self.joined || self.releases.iter().flatten().any(|release| release.life == life) {
            return false;
        }
        let slot = self
            .releases
            .iter()
            .position(Option::is_none)
            .expect("at most64 held/owed wire lifetimes");
        self.releases[slot] = Some(Release { life, staged: false, accepted: None });
        true
    }

    pub(super) fn armed(&self, channel: usize, bit: usize) -> bool {
        self.channel[channel] & (1 << bit) != 0
    }
    /// Staged means an attempt is in flight, which is why another fault cannot
    /// re-arm the slot underneath it.
    pub(super) fn staged(&self, channel: usize, bit: usize) -> bool {
        self.channel[channel] & (1 << (bit + CHANNEL_RESETS)) != 0
    }
    pub(super) fn channel_owes(&self, channel: usize) -> bool {
        self.channel[channel] != 0
    }
    pub(super) fn any_channel(&self) -> bool {
        self.channel != [0; 16]
    }
    pub(super) fn any_release(&self) -> bool {
        self.releases.iter().any(Option::is_some)
    }
    /// Nothing owed, nothing in flight, and nothing accepted still awaiting
    /// its acknowledgement.
    pub(super) fn settled(&self) -> bool {
        self.channel == [0; 16] && self.releases.iter().all(Option::is_none)
    }
    /// Debt whose neutralizing event the wire has not accepted. A seal refuses
    /// to close over this, and a join reports it as unknown wire state.
    pub(super) fn unsent(&self) -> bool {
        self.channel != [0; 16]
            || self.releases.iter().flatten().any(|release| release.accepted.is_none())
    }
    pub(super) fn release(&self, index: usize) -> Option<Release> {
        self.releases[index]
    }
    pub(super) fn releases(&self) -> impl Iterator<Item = Release> + '_ {
        self.releases.iter().flatten().copied()
    }

    /// Pending to in flight. Not a mint: an unarmed slot has nothing to stage.
    pub(super) fn stage(&mut self, channel: usize, bit: usize) {
        if !self.armed(channel, bit) {
            return;
        }
        self.channel[channel] &= !(1 << bit);
        self.channel[channel] |= 1 << (bit + CHANNEL_RESETS);
    }
    /// In flight to accepted, or back to pending where the host refused it.
    /// Not a mint either: an unstaged slot has nothing to hand back.
    pub(super) fn complete(&mut self, channel: usize, bit: usize, accepted: bool) {
        if !self.staged(channel, bit) {
            return;
        }
        self.channel[channel] &= !(1 << (bit + CHANNEL_RESETS));
        if !accepted {
            self.channel[channel] |= 1 << bit;
        }
    }
    /// Rewrite an occupied slot's staging and acceptance. Not a mint: an empty
    /// slot stays empty.
    pub(super) fn update_release(&mut self, index: usize, release: Release) {
        if self.releases[index].is_some() {
            self.releases[index] = Some(release);
        }
    }
    /// Accepted to acknowledged. Pending and staged owners cannot be removed
    /// through this path, so a release slot cannot be reused underneath the
    /// synchronous completion that owns it. Destruction uses [`Self::join`]
    /// as its separate, permanently closing abandonment path.
    pub(super) fn discharge_accepted_release(&mut self, index: usize) -> Option<Release> {
        self.releases[index]
            .is_some_and(|release| release.accepted.is_some())
            .then(|| self.releases[index].take().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::{
        event::Event,
        protocol::{Outcome, OutputDelta},
    };

    fn accepted(sequence: u64) -> OutputDelta {
        OutputDelta {
            decision: 0,
            player: 0.0,
            incarnation: 1,
            sequence,
            lifetime: sequence,
            input: 0,
            actual: 0,
            epoch: 1,
            mapped: true,
            discontinuity_generation: 0,
            event: Event::Midi { port: 0, data: [0xf8, 0, 0], flags: 0 },
            outcome: Outcome::wire(0, false),
        }
    }

    #[test]
    fn release_slots_reuse_only_after_accepted_acknowledgement() {
        let mut debt = Debt::new();
        assert!(debt.arm_release(7));
        assert!(!debt.arm_release(7));
        assert!(debt.discharge_accepted_release(0).is_none());
        let mut first = debt.release(0).unwrap();
        first.staged = true;
        debt.update_release(0, first);
        assert!(debt.arm_release(8));
        assert_eq!((debt.release(0).unwrap().life, debt.release(1).unwrap().life), (7, 8));
        assert!(debt.discharge_accepted_release(0).is_none());
        first.accepted = Some(accepted(11));
        debt.update_release(0, first);
        assert_eq!(debt.discharge_accepted_release(0).unwrap().life, 7);
        assert!(debt.arm_release(9));
        assert_eq!(debt.release(0).unwrap().life, 9);
    }

    #[test]
    fn destruction_vacates_release_slots_without_leaving_them_rearmable() {
        let mut debt = Debt::new();
        assert!(debt.arm_release(7));
        assert!(debt.arm_release(8));
        assert!(debt.arm_release(9));
        let pending = debt.release(0).unwrap();
        let mut staged = debt.release(1).unwrap();
        staged.staged = true;
        debt.update_release(1, staged);
        let mut accepted_release = debt.release(2).unwrap();
        accepted_release.accepted = Some(accepted(11));
        debt.update_release(2, accepted_release);
        let abandoned = debt.join();
        assert_eq!(
            abandoned.iter().flatten().map(|release| release.life).collect::<Vec<_>>(),
            [7, 8, 9]
        );
        // A staged release exists only inside one synchronous process output
        // drain, between prepare and its matching completion. CLAP destruction
        // occurs after processing has stopped and the plugin is inactive, so
        // this lower-level owner test is the only way to exercise that join
        // input directly.
        for (index, stale) in [pending, staged, accepted_release].into_iter().enumerate() {
            assert!(!debt.arm_release(10 + index as u16));
            debt.update_release(index, stale);
            assert!(debt.release(index).is_none());
        }
    }
}
