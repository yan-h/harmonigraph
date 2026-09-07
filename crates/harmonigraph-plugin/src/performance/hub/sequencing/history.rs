//! Transient musical history belongs to the Hub, independently of held voices
//! and display history. Release changes neither table. A clear advances a
//! decision floor, so delayed acceptance cannot repopulate a withdrawn/revised
//! history. Existing exact Plan validation precedes confirmed updates.
use super::*;
use harmonigraph_core::LatticePos;

const KEYS: usize = (TUNERS + 1) * ACTUAL_KEYS_PER_SOURCE;

#[derive(Clone, Copy, Default)]
struct Entry {
    node: Option<LatticePos>,
    decision: u64,
    revision: u64,
    incarnation: u64,
}

pub(in crate::performance::hub) struct History {
    confirmed: Box<[Entry]>,
    prospective: Box<[Entry]>,
    cleared: [u64; TUNERS + 1],
    revision: Option<u64>,
}
impl Default for History {
    fn default() -> Self {
        Self {
            confirmed: vec![Entry::default(); KEYS].into_boxed_slice(),
            prospective: vec![Entry::default(); KEYS].into_boxed_slice(),
            cleared: [0; TUNERS + 1],
            revision: None,
        }
    }
}
impl History {
    pub fn clear(&mut self, source: usize, decision: u64) {
        self.cleared[source] = decision;
    }
    pub fn clear_all(&mut self, decision: u64) {
        self.cleared.fill(decision);
    }
    pub fn configuration(&mut self, revision: u64, decision: u64) {
        if self.revision != Some(revision) {
            self.clear_all(decision);
            self.revision = Some(revision);
        }
    }
    pub fn previous(
        &self,
        lease: Lease,
        channel: u8,
        key: u8,
        revision: u64,
    ) -> Option<LatticePos> {
        let entry = self.prospective[actual_address(lease, channel, key)];
        (self.revision == Some(revision)
            && entry.revision == revision
            && entry.incarnation == lease.incarnation
            && entry.decision > self.cleared[usize::from(lease.slot)])
        .then_some(entry.node)
        .flatten()
    }
    pub fn commit(
        &mut self,
        lease: Lease,
        channel: u8,
        key: u8,
        binding: Assignment,
        accepted: bool,
    ) {
        if !binding.selection.musical()
            || binding.decision <= self.cleared[usize::from(lease.slot)]
            || self.revision != Some(binding.configuration.revision)
        {
            return;
        }
        let table = if accepted { &mut self.confirmed } else { &mut self.prospective };
        let entry = &mut table[actual_address(lease, channel, key)];
        if entry.decision >= binding.decision {
            return;
        }
        *entry = Entry {
            node: binding.node(),
            decision: binding.decision,
            revision: binding.configuration.revision,
            incarnation: lease.incarnation,
        };
    }

    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn layout(&self) -> [usize; 3] {
        [
            std::mem::size_of::<Entry>(),
            std::mem::size_of_val(&*self.confirmed),
            std::mem::size_of_val(&*self.prospective),
        ]
    }
}
const _: () = assert!(std::mem::size_of::<Entry>() <= 64);

#[cfg(test)]
#[test]
fn history_acceptance_is_separate_and_cannot_undo_a_clear() {
    let mut history = History::default();
    let lease =
        Lease { session: 1, source: harmonigraph_core::SourceId(1), incarnation: 1, slot: 1 };
    let mut binding =
        Assignment { decision: 1, selection: Selection::Node([0, 1, 0]), ..Assignment::default() };
    let revision = binding.configuration.revision;
    let address = actual_address(lease, 0, 64);
    history.configuration(revision, 0);
    history.commit(lease, 0, 64, binding, false);
    assert_eq!(history.previous(lease, 0, 64, revision), binding.node());
    assert_eq!(history.confirmed[address].node, None);
    history.commit(lease, 0, 64, binding, true);
    assert_eq!(history.confirmed[address].node, binding.node());
    assert_eq!(history.previous(Lease { incarnation: 2, ..lease }, 0, 64, revision), None);

    history.clear(1, 1);
    history.commit(lease, 0, 64, binding, true);
    history.commit(lease, 0, 64, binding, false);
    assert_eq!(history.previous(lease, 0, 64, revision), None);
    binding.decision = 2;
    binding.selection = Selection::Unretuned;
    history.commit(lease, 0, 64, binding, true);
    assert_eq!(history.confirmed[address].decision, 1);
    binding.selection = Selection::Node([4, 0, 0]);
    history.commit(lease, 0, 64, binding, false);
    assert_eq!(history.previous(lease, 0, 64, revision), binding.node());
    binding.decision = 3;
    binding.selection = Selection::NoCandidate;
    history.commit(lease, 0, 64, binding, false);
    assert_eq!(history.previous(lease, 0, 64, revision), None);
    assert_eq!(history.prospective[address].decision, 3);
    history.configuration(revision + 1, 3);
    binding.decision = 4;
    binding.selection = Selection::Node([0, 1, 0]);
    history.commit(lease, 0, 64, binding, false);
    assert_eq!(history.prospective[address].decision, 3);
}
