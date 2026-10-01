//! The sweep that retires a pane nobody draws any more.
//!
//! The renderers here that can draw more than one copy keep GPU resources per
//! live pane — a bloom chain, a grid copy, a shadow atlas — and none of them is
//! told when a pane goes away. A closed tab simply stops calling back. There is
//! no teardown to hang the release on, so the panes still PREPARING are the only evidence of which ones
//! exist, and the rule every renderer needs is the same one: stamp an entry
//! each pass it is used, and drop the entries that stopped being stamped.
//!
//! Which makes "still drawn" and "still preparing" one claim, and it is the
//! CALLER that has to keep them so. A pane that adds its callback only on the
//! frames it wants something drawn ages its own entry out over a quiet stretch
//! and rebuilds it inside the frame that ends one — the worst frame to spend
//! that in, since the stretch ends when a note arrives. Add the callback on
//! every pass and let it decline the work: that is the only thing that keeps
//! the clock running.
//!
//! [`PassAged`] owns the map, the stamps and the sweep. What it deliberately
//! does not own is the clock: `pass_nr` stays an argument, because egui
//! prepares every callback in a frame before it paints any of them, and a
//! helper that read the time itself would advance it as each live pane prepared
//! and evict the first one before the frame reached its paint.

use std::collections::HashMap;

/// How many egui passes an entry may go unseen before it is dropped.
///
/// An entry is touched once per pass while its pane is on screen, so this is
/// about two seconds at 60 fps however many panes are live — the clock counts
/// passes, not sweeps, so opening a second pane does not age the first one
/// faster. Long enough that a pane hidden for a frame keeps what it built,
/// short enough that a closed one is not still holding it a minute later.
pub(crate) const TTL_PASSES: u64 = 120;

/// A map of per-pane resources, aged out by the egui pass that last touched
/// them.
///
/// The key is whatever identity the renderer already uses — a pane id, a
/// placement, a destination surface. This neither widens nor narrows it.
pub(crate) struct PassAged<T> {
    entries: HashMap<u64, Entry<T>>,
}

struct Entry<T> {
    value: T,
    /// Egui's cumulative pass number when this entry was last touched.
    last_seen_pass: u64,
}

impl<T> PassAged<T> {
    pub(crate) fn new() -> Self {
        PassAged { entries: HashMap::new() }
    }

    /// Drop every entry that has not been touched for [`TTL_PASSES`].
    ///
    /// Run from whichever pane IS preparing, so a lone survivor still clears
    /// the others. `saturating_sub` is the guard for a `pass_nr` that did not
    /// advance: an entry stamped in the future ages at 0 rather than wrapping
    /// to a number that evicts it immediately.
    pub(crate) fn evict_unseen(&mut self, pass_nr: u64) {
        self.entries.retain(|_, entry| pass_nr.saturating_sub(entry.last_seen_pass) < TTL_PASSES);
    }

    /// The entry for `id`, built by `make` if it is not there yet, stamped
    /// either way.
    ///
    /// The ordinary path: a pane that is preparing wants its resources and is
    /// by that fact still alive.
    pub(crate) fn touched_or_insert_with(
        &mut self,
        id: u64,
        pass_nr: u64,
        make: impl FnOnce() -> T,
    ) -> &mut T {
        let entry = self
            .entries
            .entry(id)
            .or_insert_with(|| Entry { value: make(), last_seen_pass: pass_nr });
        entry.last_seen_pass = pass_nr;
        &mut entry.value
    }

    /// Read an entry without stamping it — for deciding whether what is there
    /// is still the right shape, before anything claims the pane is live.
    pub(crate) fn get(&self, id: u64) -> Option<&T> {
        self.entries.get(&id).map(|entry| &entry.value)
    }

    /// Every live value, without stamping any of them — for the shared state a
    /// pane holds a copy of, which has to reach the panes that have already
    /// prepared this pass as well as the ones that have not.
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.values_mut().map(|entry| &mut entry.value)
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, id: u64) -> bool {
        self.entries.contains_key(&id)
    }

    #[cfg(test)]
    pub(crate) fn keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundary the four renderers all sit on, asserted once here rather
    /// than inferred from any of them: last seen `TTL_PASSES - 1` ago survives,
    /// last seen `TTL_PASSES` ago does not.
    #[test]
    fn an_entry_survives_to_the_last_pass_before_the_limit() {
        let mut aged: PassAged<&str> = PassAged::new();
        aged.touched_or_insert_with(7, 0, || "held");

        aged.evict_unseen(TTL_PASSES - 1);
        assert_eq!(aged.get(7), Some(&"held"), "evicted a pass early");

        aged.evict_unseen(TTL_PASSES);
        assert_eq!(aged.get(7), None, "held a pass past the limit");
    }

    /// The sweep is per entry, not per map: one pane going quiet does not take
    /// the pane beside it that is still drawing.
    #[test]
    fn the_sweep_takes_only_the_entry_that_stopped() {
        let mut aged: PassAged<&str> = PassAged::new();
        aged.touched_or_insert_with(0, 0, || "closed");
        aged.touched_or_insert_with(1, 0, || "live");

        for pass_nr in 1..=TTL_PASSES {
            aged.touched_or_insert_with(1, pass_nr, || unreachable!("already built"));
            aged.evict_unseen(pass_nr);
        }
        assert_eq!(aged.keys().collect::<Vec<_>>(), vec![1], "the wrong entry survived");
    }

    /// Egui prepares every callback in a frame before it paints any of them, so
    /// a frame with more live panes than [`TTL_PASSES`] must still not lose the
    /// first one: the clock is the argument, and it does not move between them.
    #[test]
    fn every_entry_stamped_in_one_pass_survives_that_pass() {
        let mut aged: PassAged<u64> = PassAged::new();
        for id in 0..=TTL_PASSES * 2 {
            aged.touched_or_insert_with(id, 9, || id);
            aged.evict_unseen(9);
        }
        assert_eq!(aged.get(0), Some(&0), "the first pane went before the frame painted");
    }

    /// A `pass_nr` that does not advance cannot evict: the subtraction
    /// saturates rather than wrapping to a huge age.
    #[test]
    fn a_stamp_from_the_future_ages_at_zero() {
        let mut aged: PassAged<&str> = PassAged::new();
        aged.touched_or_insert_with(7, 500, || "held");
        aged.evict_unseen(0);
        assert_eq!(aged.get(7), Some(&"held"), "a saturating age still evicted");
    }
}
