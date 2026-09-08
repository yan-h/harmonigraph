//! Single-producer canonical publication. Musical retention acknowledgement is
//! the caller's audio-owned responsibility and never waits for these consumers.
//!
//! Loss is reported, never repaired. A publication that does not fit becomes a
//! [`PublicationGap`]; the consumer clears the held state that gap covers and
//! the producer re-publishes a snapshot of what is sounding NOW. Nothing here
//! reconstructs the attacks, releases or bend trajectories that went missing
//! (#712), so there is no acknowledgement, generation or resync protocol
//! between the two ends — one FIFO of items and one FIFO of snapshot frames.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use harmonigraph_core::canonical::{
    CanonicalEvent, GapReason, NoteDelta, PublicationGap, SourceBaseline,
};

use crate::configuration::RecordAddress;

pub const PUBLICATION_RING: usize = 4096;
/// A snapshot frame is two orders of magnitude larger than an item, so it
/// rides its own FIFO rather than widening every ring cell to hold one. Depth
/// covers a frame for each of the 17 sources in one pass, plus the one a
/// drainer that declined an item is still holding.
pub const SNAPSHOT_SLOTS: usize = 20;
/// Ordinary history may never take the last cell. That reservation is what
/// makes an outage observable when the host never calls the audio thread
/// again: the gap describing it is pushed at the moment of the loss, rather
/// than waiting for a later publication to carry it out.
const GAP_RESERVE: usize = 1;

/// Which of the two publication lanes a call is about.
///
/// The take's file and the editor's display each own a [`channel`] of their
/// own, so one can fill without the other noticing. Snapshots are therefore
/// published one lane at a time: each lane's consumer deduplicates on
/// [`SourceBaseline::id`], so each lane needs an identity that advances when
/// IT accepted a frame rather than when both did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Take,
    Display,
}

impl Lane {
    pub const ALL: [Lane; 2] = [Lane::Take, Lane::Display];
}

/// One value per lane, and the only shape anything in this path hands back.
///
/// The rule it exists to enforce: **no capacity, outcome or cursor on the
/// publication path is a value derived from both lanes.** A `usize` of free
/// cells or a `Result` for "the publication" is what let a full display ring
/// gate a snapshot the take was waiting for, and what let one lane's refusal
/// hold back the other lane's baseline identity (#712). Neither is
/// expressible here without writing the fold by hand, which is what makes the
/// separation structural instead of a habit each new caller has to keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lanes<T> {
    pub take: T,
    pub display: T,
}

impl<T: Copy> Lanes<T> {
    pub fn both(value: T) -> Self {
        Self { take: value, display: value }
    }
}

impl Lanes<bool> {
    /// "Is either lane armed at all" — for a diagnostic count, or an early
    /// return that still decides per lane afterwards. Deliberately the only
    /// fold on this type, and named so a reviewer sees it happening.
    pub fn any(self) -> bool {
        self.take || self.display
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Lanes<Result<(), PublishError>> {
    /// Both lanes accepted. Test-only, and it yields nothing: a fixture that
    /// wants one `unwrap` over a publication should still be checking both
    /// halves, but nothing may DECIDE from a folded pair.
    #[track_caller]
    pub fn expect_both(self) {
        self.take.expect("the take lane accepted");
        self.display.expect("the display lane accepted");
    }
}

impl<T> std::ops::Index<Lane> for Lanes<T> {
    type Output = T;
    fn index(&self, lane: Lane) -> &T {
        match lane {
            Lane::Take => &self.take,
            Lane::Display => &self.display,
        }
    }
}

impl<T> std::ops::IndexMut<Lane> for Lanes<T> {
    fn index_mut(&mut self, lane: Lane) -> &mut T {
        match lane {
            Lane::Take => &mut self.take,
            Lane::Display => &mut self.display,
        }
    }
}

/// Resolved on audio from the ORIGINAL actual-output recording segment. None
/// is explicit disarmed provenance; a drainer never consults today's arm state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Route {
    pub address: Option<RecordAddress>,
    /// Presentation-clock time + offset = original pass's transport time.
    pub time_offset: f64,
}

#[derive(Clone, Copy, Debug)]
enum Value {
    Note(NoteDelta),
    /// The frame itself is the next entry of the snapshot FIFO. The producer
    /// writes the frame BEFORE this marker and only once both FIFOs have room,
    /// so a marker without its frame is unreachable, and a consumer that has
    /// acquired the marker has by transitivity acquired the frame's own
    /// release too. That is the whole of the ordering contract between the
    /// two FIFOs — no handle, generation or slot state.
    Snapshot,
    Gap(PublicationGap),
    PassComplete(RecordAddress),
    EpochComplete(u64),
}

/// Neither the publisher's serial nor the audio time the item was heard at is
/// carried here.
///
/// The serial names an outage's extent inside the [`PublicationGap`] it
/// produces and nothing else reads it, since nothing on this lane is ever
/// republished or acknowledged. The observation time was the writer thread's,
/// for a fanout that forwarded the display's copy along with the take's; the
/// display has its own lane now, every drainer binds the value to `_`, and a
/// payload that needs a time already carries its own.
#[derive(Clone, Copy, Debug)]
struct Item {
    route: Route,
    value: Value,
}

pub struct Publisher {
    ring: rtrb::Producer<Item>,
    snapshots: rtrb::Producer<SourceBaseline>,
    clock: Arc<AtomicU64>,
    serial: u64,
    /// An outage whose gap could not reach the ring even on the reserved cell,
    /// because an earlier gap still occupies it. It keeps growing until the
    /// drainer makes room; the earlier gap has already told the consumer to
    /// clear, so nothing downstream is waiting on this one.
    outage: Option<(PublicationGap, Route)>,
}

pub struct Consumer {
    ring: rtrb::Consumer<Item>,
    snapshots: rtrb::Consumer<SourceBaseline>,
    clock: Arc<AtomicU64>,
    pending: Option<Item>,
    /// A frame popped for a drainer that then declined its item. Popping is
    /// what frees the producer's cell, so it cannot be given back.
    held: Option<SourceBaseline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    /// An actual canonical publication attempt failed. The gap describing it
    /// is queued by the same call, so the loss stays observable even if audio
    /// never calls again.
    Lost,
    /// A snapshot found no room. Nothing was written and no serial was spent,
    /// so this is not history loss: the caller still holds a complete frame
    /// and republishes it once the drainer has caught up.
    Busy,
    Invalid,
}

pub enum Delivery<'a> {
    Event(CanonicalEvent<'a>),
    PassComplete(RecordAddress),
    EpochComplete(u64),
}

pub fn channel() -> (Publisher, Consumer) {
    let (producer, consumer) = rtrb::RingBuffer::new(PUBLICATION_RING);
    let (frames, staged) = rtrb::RingBuffer::new(SNAPSHOT_SLOTS);
    let clock = Arc::new(AtomicU64::new(f64::NAN.to_bits()));
    (
        Publisher {
            ring: producer,
            snapshots: frames,
            clock: clock.clone(),
            serial: 0,
            outage: None,
        },
        Consumer { ring: consumer, snapshots: staged, clock, pending: None, held: None },
    )
}

impl Publisher {
    /// A fresh hub clock observation, independent of delayed history and cuts.
    pub fn observe_clock(&self, time: f64) {
        if time.is_finite() {
            self.clock.store(time.to_bits(), Ordering::Release);
        }
    }
    /// Cells ordinary history may still take — the reserve is not one of them.
    pub fn free(&self) -> usize {
        self.ring.slots().saturating_sub(GAP_RESERVE)
    }

    /// Explicit loss the caller detected before it reached this lane at all —
    /// an unroutable record, or a merge that lost its clock provenance.
    pub fn discarded(&mut self, time: f64, route: Route) {
        if let Some(serial) = self.serial.checked_add(1) {
            self.serial = serial;
            self.lost(time, route);
        }
    }

    fn lost(&mut self, time: f64, route: Route) -> PublishError {
        let gap = match self.outage {
            Some((old, _)) => {
                PublicationGap { last: self.serial, through: old.through.max(time), ..old }
            }
            None => PublicationGap {
                source: None,
                time,
                through: time,
                first: self.serial,
                last: self.serial,
                reason: GapReason::PublicationFull,
            },
        };
        // If an outage crosses pass boundaries the writer marks the entire
        // still-owned recording incomplete, rather than guessing one address.
        let route = match self.outage {
            Some((_, old)) if old != route => Route::default(),
            _ => route,
        };
        self.outage = Some((gap, route));
        self.flush_outage();
        PublishError::Lost
    }

    /// Put the outage on the ring the moment it happens. The reserved cell is
    /// free unless an earlier gap already occupies it, and that gap has told
    /// the consumer everything this one would.
    fn flush_outage(&mut self) {
        let Some((gap, route)) = self.outage else { return };
        if self.ring.push(Item { route, value: Value::Gap(gap) }).is_ok() {
            self.outage = None;
        }
    }

    fn push(&mut self, value: Value, time: f64, route: Route) -> Result<(), PublishError> {
        let Some(serial) = self.serial.checked_add(1) else { return Err(PublishError::Invalid) };
        if !time.is_finite() || !route.time_offset.is_finite() {
            return Err(PublishError::Invalid);
        }
        if self.ring.slots() < self.needed() {
            self.serial = serial;
            return Err(self.lost(time, route));
        }
        self.flush_outage();
        self.serial = serial;
        let _ = self.ring.push(Item { route, value });
        Ok(())
    }

    /// Cells one ordinary publication costs: its own, the reserve it may not
    /// spend, and one more for an outage queued ahead of it.
    fn needed(&self) -> usize {
        1 + GAP_RESERVE + usize::from(self.outage.is_some())
    }

    pub fn note(&mut self, note: NoteDelta, route: Route) -> Result<(), PublishError> {
        note.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Note(note), note.event.time, route)
    }

    pub fn gap(&mut self, gap: PublicationGap, route: Route) -> Result<(), PublishError> {
        gap.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Gap(gap), gap.time, route)
    }

    /// What the source is sounding NOW. Published whole or not at all: both
    /// FIFOs are checked before either is written, so a marker and its frame
    /// cannot come apart and a refused snapshot costs no serial.
    pub fn baseline(
        &mut self,
        baseline: &SourceBaseline,
        route: Route,
    ) -> Result<(), PublishError> {
        baseline.validate().map_err(|_| PublishError::Invalid)?;
        if self.snapshots.slots() == 0 || self.ring.slots() < self.needed() {
            return Err(PublishError::Busy);
        }
        let _ = self.snapshots.push(*baseline);
        let result = self.push(Value::Snapshot, baseline.time, route);
        debug_assert!(result.is_ok(), "capacity for both halves was checked together");
        result
    }

    /// A closure carries no time of its own, so the caller's observation time
    /// is what stamps the gap if the lane is full — the only use either of
    /// these has for it.
    pub fn pass_complete(
        &mut self,
        address: RecordAddress,
        observed: f64,
    ) -> Result<(), PublishError> {
        self.push(Value::PassComplete(address), observed, Route::default())
    }

    pub fn epoch_complete(&mut self, epoch: u64, observed: f64) -> Result<(), PublishError> {
        self.push(Value::EpochComplete(epoch), observed, Route::default())
    }
}

impl Consumer {
    /// No retained payload remains. Every outage this lane produced is already
    /// an item by the time its `Err` was returned, so an empty lane is a fully
    /// delivered one.
    pub(crate) fn settled(&self) -> bool {
        self.pending.is_none() && self.held.is_none() && self.ring.is_empty()
    }

    pub fn clock(&self) -> Option<f64> {
        let time = f64::from_bits(self.clock.load(Ordering::Acquire));
        time.is_finite().then_some(time)
    }
    /// One bounded drain of the actual selected ring capacity. The borrowed
    /// snapshot cannot escape the call; deferred consumers must make an owned
    /// copy BEFORE this returns and permits reuse.
    pub fn drain(&mut self, mut consume: impl FnMut(Delivery<'_>, Route) -> bool) -> usize {
        let mut count = 0;
        for _ in 0..PUBLICATION_RING {
            let Some(item) = self.pending.take().or_else(|| self.ring.pop().ok()) else { break };
            let consumed = match item.value {
                Value::Note(note) => {
                    consume(Delivery::Event(CanonicalEvent::Note(note)), item.route)
                }
                Value::Gap(gap) => consume(Delivery::Event(CanonicalEvent::Gap(gap)), item.route),
                Value::PassComplete(address) => {
                    consume(Delivery::PassComplete(address), item.route)
                }
                Value::EpochComplete(epoch) => consume(Delivery::EpochComplete(epoch), item.route),
                Value::Snapshot => {
                    let Some(frame) = self.held.take().or_else(|| self.snapshots.pop().ok()) else {
                        debug_assert!(false, "a snapshot marker always follows its frame");
                        count += 1;
                        continue;
                    };
                    let consumed =
                        consume(Delivery::Event(CanonicalEvent::Baseline(&frame)), item.route);
                    if !consumed {
                        self.held = Some(frame);
                    }
                    consumed
                }
            };
            if !consumed {
                self.pending = Some(item);
                break;
            }
            count += 1;
        }
        count
    }
}

const _: () = assert!(std::mem::size_of::<Item>() <= 256);
const _: () = assert!(std::mem::align_of::<Item>() <= 8);

#[cfg(feature = "test-support")]
pub fn print_test_memory_layout() {
    use std::mem::size_of;
    println!(
        "LEDGER publication [item,snapshot,publisher,consumer] {:?}",
        [
            size_of::<Item>(),
            size_of::<SourceBaseline>(),
            size_of::<Publisher>(),
            size_of::<Consumer>()
        ]
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::canonical::VoiceBaseline;
    use harmonigraph_core::{NoteEvent, SourceId};

    fn held(note: u8, onset: f64, pitch: i64) -> VoiceBaseline {
        VoiceBaseline {
            note,
            actual_onset: onset,
            input_onset: onset,
            velocity: 0.8,
            pitch_microcents: pitch,
            ..Default::default()
        }
    }

    fn frame(id: u64, time: f64, voices: &[VoiceBaseline]) -> SourceBaseline {
        SourceBaseline::new(SourceId::DIRECT, id, time, 0, true, voices).unwrap()
    }

    #[test]
    fn a_full_lane_reports_its_outage_without_another_audio_callback() {
        let (mut publisher, mut consumer) = channel();
        for note in [60, 72] {
            publisher
                .note(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 0.8).into(), Route::default())
                .unwrap();
        }
        // Bends rather than repeated attacks: one onset per key, so a note the
        // snapshot restores can be checked against the onset it really had.
        for i in 2..PUBLICATION_RING - GAP_RESERVE {
            let mut event = NoteEvent::on(i as f64, SourceId::DIRECT, 0, 72, 0.8);
            event.kind = harmonigraph_core::NoteEventKind::Tuning { semitones: 0.1 };
            publisher.note(event.into(), Route::default()).unwrap();
        }
        assert_eq!(publisher.free(), 0, "the fixture must actually fill the lane");
        assert_eq!(
            publisher
                .note(NoteEvent::off(9000.0, SourceId::DIRECT, 0, 60).into(), Route::default()),
            Err(PublishError::Lost)
        );
        // Nothing publishes after the loss: whatever the drainer can see now
        // is all it would ever see if this were the host's last callback.
        let mut tracker = harmonigraph_core::NoteTracker::new();
        let mut held_at_gap = None;
        let mut gaps = 0;
        let drained = consumer.drain(|delivery, _| {
            let Delivery::Event(event) = delivery else { panic!("unexpected control") };
            if let CanonicalEvent::Gap(_) = event {
                held_at_gap = Some(tracker.held_count());
                gaps += 1;
            }
            tracker.handle_canonical(event).unwrap();
            true
        });
        assert_eq!(drained, PUBLICATION_RING, "the reserved cell carried the gap");
        assert_eq!(gaps, 1);
        assert_eq!(held_at_gap, Some(2), "the fixture must reach the gap holding both keys");
        assert_eq!(tracker.held_count(), 0, "the gap clears stale held state");
        assert!(!tracker.source_current_certain(SourceId::DIRECT));

        // The refresh restores what is sounding NOW and nothing else: key 60
        // was released while the lane was full and does not come back.
        publisher
            .baseline(&frame(1, 9001.0, &[held(72, 0.0, 7_200_000_000)]), Route::default())
            .unwrap();
        consumer.drain(|delivery, _| {
            let Delivery::Event(event) = delivery else { panic!("unexpected control") };
            tracker.handle_canonical(event).unwrap();
            true
        });
        assert!(tracker.source_current_certain(SourceId::DIRECT));
        assert_eq!(tracker.held_count(), 1);
        let restored = tracker.roll().notes().find(|n| n.note == 72 && n.end.is_none()).unwrap();
        assert_eq!(restored.start, 0.0, "the snapshot restores, it does not re-attack");
        assert!(!restored.history_complete, "the missing history stays visible");
        assert_eq!(tracker.publication_gaps().len(), 1);
    }

    #[test]
    fn a_declined_snapshot_stays_owned_and_refuses_the_next_without_losing_history() {
        let (mut publisher, mut consumer) = channel();
        let voices: Vec<_> =
            (0..64).map(|note| held(note, 0.0, i64::from(note) * 100_000_000)).collect();
        for id in 1..=SNAPSHOT_SLOTS as u64 {
            publisher.baseline(&frame(id, 1.0, &voices), Route::default()).unwrap();
        }
        assert_eq!(
            publisher.baseline(&frame(99, 1.0, &voices), Route::default()),
            Err(PublishError::Busy),
            "a full snapshot FIFO refuses rather than reporting lost history"
        );
        assert_eq!(consumer.drain(|_, _| false), 0, "a declining drainer retains the frame");
        let mut seen = Vec::new();
        consumer.drain(|delivery, _| {
            let Delivery::Event(CanonicalEvent::Baseline(frame)) = delivery else { panic!() };
            seen.push(frame.id);
            true
        });
        assert_eq!(seen, (1..=SNAPSHOT_SLOTS as u64).collect::<Vec<_>>());
        publisher.baseline(&frame(99, 2.0, &voices), Route::default()).unwrap();
        eprintln!(
            "canonical layouts: NoteDelta={} Item={} VoiceBaseline={} SourceBaseline={} ring_payload={} snapshot_fifo={}",
            std::mem::size_of::<NoteDelta>(),
            std::mem::size_of::<Item>(),
            std::mem::size_of::<VoiceBaseline>(),
            std::mem::size_of::<SourceBaseline>(),
            PUBLICATION_RING * std::mem::size_of::<Item>(),
            SNAPSHOT_SLOTS * std::mem::size_of::<SourceBaseline>()
        );
    }
}
