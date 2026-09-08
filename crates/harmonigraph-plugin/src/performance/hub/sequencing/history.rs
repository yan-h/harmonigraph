//! Transient musical history belongs to the Hub, independently of held voices
//! and display history. Release does not change it. A clear advances a
//! decision floor, so delayed acceptance cannot repopulate a withdrawn/revised
//! history.
use super::*;
use harmonigraph_core::LatticePos;

/// This table's own key directory: one cell per (source, channel, key). It is
/// a bounded direct address, not a hint — nothing here needs a fallback scan.
const KEYS_PER_SOURCE: usize = 16 * 128;
const KEYS: usize = (TUNERS + 1) * KEYS_PER_SOURCE;
fn address(lease: Lease, channel: u8, key: u8) -> usize {
    usize::from(lease.slot) * KEYS_PER_SOURCE + usize::from(channel) * 128 + usize::from(key)
}

#[derive(Clone, Copy, Default)]
struct Entry {
    node: Option<LatticePos>,
    decision: u64,
    revision: u64,
    incarnation: u64,
}

pub(in crate::performance::hub) struct History {
    prospective: Box<[Entry]>,
    cleared: [u64; TUNERS + 1],
    revision: Option<u64>,
}
impl Default for History {
    fn default() -> Self {
        Self {
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
        let entry = self.prospective[address(lease, channel, key)];
        (self.revision == Some(revision)
            && entry.revision == revision
            && entry.incarnation == lease.incarnation
            && entry.decision > self.cleared[usize::from(lease.slot)])
        .then_some(entry.node)
        .flatten()
    }
    pub fn commit(&mut self, lease: Lease, channel: u8, key: u8, binding: Assignment) {
        if !binding.selection.musical()
            || binding.decision <= self.cleared[usize::from(lease.slot)]
            || self.revision != Some(binding.configuration.revision)
        {
            return;
        }
        let entry = &mut self.prospective[address(lease, channel, key)];
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

    #[cfg(test)]
    pub fn layout(&self) -> [usize; 2] {
        [std::mem::size_of::<Entry>(), std::mem::size_of_val(&*self.prospective)]
    }
}
const _: () = assert!(std::mem::size_of::<Entry>() <= 64);

#[cfg(test)]
#[test]
fn a_clear_cannot_be_undone_on_the_prospective_table() {
    let mut history = History::default();
    let lease =
        Lease { session: 1, source: harmonigraph_core::SourceId(1), incarnation: 1, slot: 1 };
    let mut binding =
        Assignment { decision: 1, selection: Selection::Node([0, 1, 0]), ..Assignment::default() };
    let revision = binding.configuration.revision;
    let address = address(lease, 0, 64);
    history.configuration(revision, 0);
    history.commit(lease, 0, 64, binding);
    assert_eq!(history.previous(lease, 0, 64, revision), binding.node());
    assert_eq!(history.previous(Lease { incarnation: 2, ..lease }, 0, 64, revision), None);

    history.clear(1, 1);
    history.commit(lease, 0, 64, binding);
    assert_eq!(history.previous(lease, 0, 64, revision), None);
    binding.decision = 2;
    binding.selection = Selection::Unretuned;
    history.commit(lease, 0, 64, binding);
    binding.selection = Selection::Node([4, 0, 0]);
    history.commit(lease, 0, 64, binding);
    assert_eq!(history.previous(lease, 0, 64, revision), binding.node());
    binding.decision = 3;
    binding.selection = Selection::NoCandidate;
    history.commit(lease, 0, 64, binding);
    assert_eq!(history.previous(lease, 0, 64, revision), None);
    assert_eq!(history.prospective[address].decision, 3);
    history.configuration(revision + 1, 3);
    binding.decision = 4;
    binding.selection = Selection::Node([0, 1, 0]);
    history.commit(lease, 0, 64, binding);
    assert_eq!(history.prospective[address].decision, 3);
}
