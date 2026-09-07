//! Tune-local input storage, and the Hub-side ordering of the copies it sends.
//!
//! Nothing here is shared between the two threads. The Tune owns its `Pending`
//! envelopes and `Life` records outright; what crosses to the Hub is a
//! `protocol::Capture` — a fixed-size copy carrying its own identity — pushed
//! through the ordinary intent ring. There is no arena, no publication token,
//! no permission plane and no retirement handshake, because the Hub never
//! reads a byte the Tune still owns.
use super::protocol::*;
use super::source::{Life, Pending, NONE};

/// One Pending envelope plus the links and phase bits that are nobody's
/// business but this Tune's.
struct Cell {
    value: Option<Pending>,
    previous: u16,
    next: u16,
    /// The input's whole reference group is built; it may now be copied out.
    sealed: bool,
    /// Its copy has reached the Hub (or was dropped with no session to reach).
    /// An unpublished envelope is retained so an input cannot vanish between
    /// the two sides; a published one has no remote reader at all.
    published: bool,
    /// Channel bookkeeping for this envelope has run.
    local_done: bool,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            value: None,
            previous: NONE,
            next: NONE,
            sealed: false,
            published: false,
            local_done: false,
        }
    }
}

pub(super) fn storage() -> (PendingStore, Lives, super::source::work::Work) {
    (
        PendingStore {
            cells: (0..PENDING_EVENTS).map(|_| Cell::default()).collect(),
            free: (0..PENDING_EVENTS as u16).rev().collect(),
            head: NONE,
            tail: NONE,
            len: 0,
        },
        Lives { cells: (0..LIFETIMES).map(|_| None).collect() },
        super::source::work::Work::default(),
    )
}

pub(super) struct PendingStore {
    cells: Box<[Cell]>,
    free: Vec<u16>,
    head: u16,
    tail: u16,
    len: usize,
}
impl PendingStore {
    pub const BACKING_CELL_BYTES: usize = std::mem::size_of::<Cell>();
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        self.free.len()
    }
    pub fn front_position(&self) -> Option<usize> {
        (self.head != NONE).then_some(self.head as usize)
    }
    pub fn back_position(&self) -> Option<usize> {
        (self.tail != NONE).then_some(self.tail as usize)
    }
    pub fn next_position(&self, index: usize) -> Option<usize> {
        let next = self.cells[index].next;
        (next != NONE).then_some(next as usize)
    }
    pub fn at(&self, index: usize) -> Option<Pending> {
        self.cells.get(index)?.value
    }
    pub fn set(&mut self, index: usize, value: Pending) {
        let cell = &mut self.cells[index];
        assert!(cell.value.is_some());
        cell.value = Some(value);
    }
    pub fn push(&mut self, value: Pending) -> Result<(), Pending> {
        let Some(index) = self.free.pop() else {
            return Err(value);
        };
        let previous = self.tail;
        self.cells[index as usize] = Cell {
            value: Some(value),
            previous,
            next: NONE,
            sealed: false,
            published: false,
            local_done: false,
        };
        if previous == NONE {
            self.head = index;
        } else {
            self.cells[previous as usize].next = index;
        }
        self.tail = index;
        self.len += 1;
        Ok(())
    }
    pub fn seal(&mut self, index: usize) {
        self.cells[index].sealed = true;
    }
    pub fn sealed(&self, index: usize) -> bool {
        self.cells[index].sealed
    }
    pub fn local_done(&self, index: usize) -> bool {
        self.cells[index].local_done
    }
    pub fn finish_local(&mut self, index: usize) {
        self.cells[index].local_done = true;
    }
    pub fn published(&self, index: usize) -> bool {
        self.cells[index].published
    }
    pub fn publish(&mut self, index: usize) {
        self.cells[index].published = true;
    }
    pub fn remove(&mut self, index: usize) -> Option<Pending> {
        let value = self.cells[index].value.take()?;
        let (previous, next) = (self.cells[index].previous, self.cells[index].next);
        if previous == NONE {
            self.head = next;
        } else {
            self.cells[previous as usize].next = next;
        }
        if next == NONE {
            self.tail = previous;
        } else {
            self.cells[next as usize].previous = previous;
        }
        self.cells[index] = Cell::default();
        self.free.push(index as u16);
        self.len -= 1;
        Some(value)
    }
    #[cfg(test)]
    pub fn test_layout(&self) -> [usize; 5] {
        [
            std::mem::size_of::<Pending>(),
            Self::BACKING_CELL_BYTES,
            PENDING_EVENTS,
            std::mem::size_of_val(&*self.cells),
            self.free.capacity(),
        ]
    }
}

pub(super) struct Lives {
    cells: Box<[Option<Life>]>,
}
impl Lives {
    pub fn at(&self, index: u16) -> Option<Life> {
        *self.cells.get(index as usize)?
    }
    pub fn local_mut(&mut self, index: u16) -> Option<&mut Life> {
        self.cells.get_mut(index as usize)?.as_mut()
    }
    pub fn insert(&mut self, index: u16, value: Life) {
        assert!(self.cells[index as usize].is_none());
        self.cells[index as usize] = Some(value);
    }
    pub fn remove(&mut self, index: u16) {
        let value = self.cells[index as usize].take().expect("live request slot");
        assert_eq!(value.refs, 0);
        assert!(!value.active && !value.reserved && !value.ready_queued && !value.assignment_held);
    }
    #[cfg(test)]
    pub fn test_layout(&self) -> [usize; 3] {
        [std::mem::size_of::<Option<Life>>(), LIFETIMES, std::mem::size_of_val(&*self.cells)]
    }
}

/// The Hub's ordering pass over one sample's copied records.
///
/// This is the whole algorithm: merge the per-source streams by sample, then
/// within a sample apply every release and controller from every source before
/// assigning any onset, onsets in the key/channel/source tie-break. There is no
/// adjacency matrix, phase machine or resumable cursor. Within one source the
/// host already delivered same-sample events in dependency order and copying
/// preserved it; across sources nothing depends on anything except onsets on
/// onsets, which the sort orders.
pub(super) struct Batch {
    records: Vec<Capture>,
    /// Per-note pitch expression seen in this sample's first pass for a note
    /// that has no assignment yet — the value a fresh onset starts from.
    tuning: Vec<(u8, u64, u64)>,
    /// Lifetimes this sample ended before the Hub held a voice for them. The
    /// release-first order puts a terminal ahead of its own onset whenever a
    /// note is born and ends at one sample, so the onset has to learn it is
    /// already over instead of leaving a live context entry behind.
    terminated: Vec<(u8, u64)>,
    /// The serial of each source's last Stop in this sample, or zero. A Stop
    /// sorts into the release-first half, so a same-sample onset it ends is
    /// still ahead of it in the pass and would otherwise insert the context
    /// entry the Stop was there to take out.
    stopped: [u64; TUNERS + 1],
    cursor: usize,
    pub sample: i64,
    pub active: bool,
}
impl Default for Batch {
    fn default() -> Self {
        Self {
            records: Vec::with_capacity(BATCH_EVENTS),
            tuning: Vec::with_capacity(harmonigraph_core::policy::MAX_COHORT_ONSETS),
            terminated: Vec::with_capacity(BATCH_EVENTS),
            stopped: [0; TUNERS + 1],
            cursor: 0,
            sample: 0,
            active: false,
        }
    }
}
impl Batch {
    pub fn begin(&mut self, sample: i64) {
        self.records.clear();
        self.tuning.clear();
        self.terminated.clear();
        self.stopped = [0; TUNERS + 1];
        self.cursor = 0;
        self.sample = sample;
        self.active = true;
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.records.len()
    }
    /// False is the bounded overflow: this sample carries more records than the
    /// pass can hold, and the caller latches rather than dropping an input.
    #[must_use]
    pub fn push(&mut self, record: Capture) -> bool {
        if self.records.len() == BATCH_EVENTS {
            return false;
        }
        self.records.push(record);
        true
    }
    /// Sorts in place with no allocation; the tie-break in `Capture::order`
    /// keeps every source's own input order inside each half.
    pub fn order(&mut self) {
        self.records.sort_unstable_by_key(|record| record.order());
    }
    pub fn next(&mut self) -> Option<Capture> {
        let record = *self.records.get(self.cursor)?;
        self.cursor += 1;
        Some(record)
    }
    /// Records a pitch expression that arrived at the same sample as the note
    /// it addresses, before that note had a voice to apply it to. False is the
    /// bounded overflow.
    #[must_use]
    pub fn stash_tuning(&mut self, source: u8, lifetime: u64, value_bits: u64) -> bool {
        if let Some(slot) =
            self.tuning.iter_mut().find(|(row, life, _)| *row == source && *life == lifetime)
        {
            slot.2 = value_bits;
            return true;
        }
        if self.tuning.len() == harmonigraph_core::policy::MAX_COHORT_ONSETS {
            return false;
        }
        self.tuning.push((source, lifetime, value_bits));
        true
    }
    /// A terminal that found no voice to end. One entry per record and the
    /// pass already refuses more records than `BATCH_EVENTS`, so the reserve
    /// cannot be outrun and the guard never allocates.
    pub fn ended(&mut self, source: u8, lifetime: u64) {
        if self.terminated.len() < BATCH_EVENTS {
            self.terminated.push((source, lifetime));
        }
    }
    /// True when this sample already ended the note this onset is starting.
    pub fn already_ended(&self, source: u8, lifetime: u64) -> bool {
        self.terminated.contains(&(source, lifetime))
    }
    /// This source stopped at this sample, at the given input serial.
    pub fn stopped(&mut self, source: u8, serial: u64) {
        let slot = &mut self.stopped[usize::from(source)];
        *slot = (*slot).max(serial);
    }
    /// True when the Stop this sample carries is later in the source's own
    /// input than this onset, and so ended it.
    pub fn already_stopped(&self, source: u8, serial: u64) -> bool {
        self.stopped[usize::from(source)] > serial
    }
    pub fn initial_tuning(&self, source: u8, lifetime: u64) -> Option<f64> {
        self.tuning
            .iter()
            .find(|(row, life, _)| *row == source && *life == lifetime)
            .map(|(_, _, bits)| f64::from_bits(*bits))
    }
    pub fn end(&mut self) {
        self.records.clear();
        self.tuning.clear();
        self.terminated.clear();
        self.stopped = [0; TUNERS + 1];
        self.cursor = 0;
        self.active = false;
    }
}

const _: () = assert!(std::mem::size_of::<Cell>() <= 128);

#[cfg(test)]
pub(super) fn print_test_memory_layout() {
    use std::mem::size_of;
    println!(
        "LEDGER capture [pending_cell,life_cell,pending_backing,life_backing,record,batch_backing] {:?}",
        [
            size_of::<Cell>(),
            size_of::<Option<Life>>(),
            size_of::<Cell>() * PENDING_EVENTS,
            size_of::<Option<Life>>() * LIFETIMES,
            size_of::<Capture>(),
            size_of::<Capture>() * BATCH_EVENTS,
        ]
    );
}
