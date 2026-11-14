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
    CanonicalEvent, ClockId, GapReason, NoteDelta, PublicationGap, SourceBaseline,
};

use crate::configuration::RecordIntent;

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

/// Where an input arrived: the Hub's clock and the sample on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub clock: ClockId,
    pub input: i64,
}

impl Stamp {
    /// A clock cut starts a later epoch, so a range spans one in this order.
    fn order(self) -> (u64, i64) {
        (self.clock.epoch, self.input)
    }
}

/// The inputs an item came from, and the arm intent of the callback that held
/// them. Taken on audio when the input arrived; a drainer never consults
/// today's arm state. Disarmed is explicit provenance: the writer drops it.
///
/// One input for a note or a snapshot. A gap names the first and last input
/// it lost, so the reader can tell which runs, and so which passes, it touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub first: Stamp,
    pub last: Stamp,
    pub intent: RecordIntent,
}

impl Span {
    pub fn at(clock: ClockId, input: i64, intent: RecordIntent) -> Self {
        let stamp = Stamp { clock, input };
        Self { first: stamp, last: stamp, intent }
    }

    /// Both spans' inputs, armed if either part was, under the newer epoch.
    pub fn merge(self, other: Span) -> Self {
        let epoch = self.intent.epoch().max(other.intent.epoch());
        Self {
            first: if other.first.order() < self.first.order() { other.first } else { self.first },
            last: if other.last.order() > self.last.order() { other.last } else { self.last },
            intent: if self.intent.is_armed() || other.intent.is_armed() {
                RecordIntent::armed(epoch)
            } else {
                RecordIntent::disarmed(epoch)
            },
        }
    }
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
    PassClosed {
        epoch: u64,
        pass: u32,
    },
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
    span: Span,
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
    /// The span is of the history lost; a lost closure covers no input.
    outage: Option<(PublicationGap, Option<Span>)>,
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
    /// Both frontiers have passed this pass's last run: every record of it is
    /// ahead of this on its lane.
    PassClosed {
        epoch: u64,
        pass: u32,
    },
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

    /// Explicit loss the caller detected before it reached this lane at all:
    /// history that overflowed the Hub's own batch, spanning these inputs.
    pub fn discarded(&mut self, time: f64, span: Span) {
        if let Some(serial) = self.serial.checked_add(1) {
            self.serial = serial;
            self.lost(time, Some(span));
        }
    }

    /// `span` is `None` for a closure: it was played at no input, and must not
    /// widen the range of inputs the gap names.
    fn lost(&mut self, time: f64, span: Option<Span>) -> PublishError {
        let (gap, span) = match self.outage {
            Some((old, covered)) => (
                PublicationGap { last: self.serial, through: old.through.max(time), ..old },
                match (covered, span) {
                    (Some(covered), Some(span)) => Some(covered.merge(span)),
                    (covered, span) => covered.or(span),
                },
            ),
            None => (
                PublicationGap {
                    source: None,
                    time,
                    through: time,
                    first: self.serial,
                    last: self.serial,
                    reason: GapReason::PublicationFull,
                },
                span,
            ),
        };
        self.outage = Some((gap, span));
        self.flush_outage();
        PublishError::Lost
    }

    /// Put the outage on the ring the moment it happens. The reserved cell is
    /// free unless an earlier gap already occupies it, and that gap has told
    /// the consumer everything this one would.
    fn flush_outage(&mut self) {
        let Some((gap, span)) = self.outage else { return };
        if self.ring.push(Item { span: span.unwrap_or_default(), value: Value::Gap(gap) }).is_ok() {
            self.outage = None;
        }
    }

    fn push(&mut self, value: Value, time: f64, span: Option<Span>) -> Result<(), PublishError> {
        let Some(serial) = self.serial.checked_add(1) else { return Err(PublishError::Invalid) };
        if !time.is_finite() {
            return Err(PublishError::Invalid);
        }
        if self.ring.slots() < self.needed() {
            self.serial = serial;
            return Err(self.lost(time, span));
        }
        self.flush_outage();
        self.serial = serial;
        let _ = self.ring.push(Item { span: span.unwrap_or_default(), value });
        Ok(())
    }

    /// Cells one ordinary publication costs: its own, the reserve it may not
    /// spend, and one more for an outage queued ahead of it.
    fn needed(&self) -> usize {
        1 + GAP_RESERVE + usize::from(self.outage.is_some())
    }

    pub fn note(&mut self, note: NoteDelta, span: Span) -> Result<(), PublishError> {
        note.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Note(note), note.event.time, Some(span))
    }

    pub fn gap(&mut self, gap: PublicationGap, span: Span) -> Result<(), PublishError> {
        gap.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Gap(gap), gap.time, Some(span))
    }

    /// What the source is sounding NOW. Published whole or not at all: both
    /// FIFOs are checked before either is written, so a marker and its frame
    /// cannot come apart and a refused snapshot costs no serial.
    pub fn baseline(&mut self, baseline: &SourceBaseline, span: Span) -> Result<(), PublishError> {
        baseline.validate().map_err(|_| PublishError::Invalid)?;
        if self.snapshots.slots() == 0 || self.ring.slots() < self.needed() {
            return Err(PublishError::Busy);
        }
        let _ = self.snapshots.push(*baseline);
        let result = self.push(Value::Snapshot, baseline.time, Some(span));
        debug_assert!(result.is_ok(), "capacity for both halves was checked together");
        result
    }

    /// A closure carries no time of its own, so the caller's observation time
    /// is what stamps the gap if the lane is full — the only use either of
    /// these has for it.
    pub fn pass_closed(
        &mut self,
        epoch: u64,
        pass: u32,
        observed: f64,
    ) -> Result<(), PublishError> {
        self.push(Value::PassClosed { epoch, pass }, observed, None)
    }

    pub fn epoch_complete(&mut self, epoch: u64, observed: f64) -> Result<(), PublishError> {
        self.push(Value::EpochComplete(epoch), observed, None)
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
    pub fn drain(&mut self, mut consume: impl FnMut(Delivery<'_>, Span) -> bool) -> usize {
        let mut count = 0;
        for _ in 0..PUBLICATION_RING {
            let Some(item) = self.pending.take().or_else(|| self.ring.pop().ok()) else { break };
            let consumed = match item.value {
                Value::Note(note) => {
                    consume(Delivery::Event(CanonicalEvent::Note(note)), item.span)
                }
                Value::Gap(gap) => consume(Delivery::Event(CanonicalEvent::Gap(gap)), item.span),
                Value::PassClosed { epoch, pass } => {
                    consume(Delivery::PassClosed { epoch, pass }, item.span)
                }
                Value::EpochComplete(epoch) => consume(Delivery::EpochComplete(epoch), item.span),
                Value::Snapshot => {
                    let Some(frame) = self.held.take().or_else(|| self.snapshots.pop().ok()) else {
                        debug_assert!(false, "a snapshot marker always follows its frame");
                        count += 1;
                        continue;
                    };
                    let consumed =
                        consume(Delivery::Event(CanonicalEvent::Baseline(&frame)), item.span);
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

    /// Publish releases until no ordinary cell is left.
    fn fill(publisher: &mut Publisher) {
        let mut time = 0.0;
        while publisher.free() > 0 {
            time += 1.0;
            let release = NoteEvent::off(time, SourceId::DIRECT, 0, 60).into();
            publisher.note(release, Span::default()).unwrap();
        }
    }

    /// Losses that go on while the first one's gap still holds the reserve
    /// become ONE later gap: it spans every serial lost after the first gap,
    /// runs through the latest loss, covers every input lost, across a clock
    /// cut too, and is armed if any part of it was, under the newer epoch.
    #[test]
    fn an_outage_behind_a_queued_gap_grows_into_one_gap() {
        let clock = |epoch| ClockId { runtime_session: 1, epoch };
        let armed = |input| Span::at(clock(0), input, RecordIntent::armed(1));
        let later = Span::at(clock(1), 5, RecordIntent::disarmed(2));
        let stamp = |epoch, input| Stamp { clock: clock(epoch), input };
        for (last, merged) in [
            (armed(300), Span { first: stamp(0, 200), last: stamp(0, 300), ..armed(0) }),
            (
                later,
                Span { first: stamp(0, 200), last: stamp(1, 5), intent: RecordIntent::armed(2) },
            ),
        ] {
            let (mut publisher, mut consumer) = channel();
            fill(&mut publisher);
            for (time, span) in [(9000.0, armed(100)), (9100.0, armed(200)), (9200.0, last)] {
                let release = NoteEvent::off(time, SourceId::DIRECT, 0, 60).into();
                assert_eq!(publisher.note(release, span), Err(PublishError::Lost));
            }
            let mut gaps = Vec::new();
            let mut drain = |consumer: &mut Consumer| {
                consumer.drain(|delivery, span| {
                    if let Delivery::Event(CanonicalEvent::Gap(gap)) = delivery {
                        gaps.push((gap, span));
                    }
                    true
                });
            };
            drain(&mut consumer);
            // The held outage reaches the ring ahead of the next publication.
            let attack = NoteEvent::on(9300.0, SourceId::DIRECT, 0, 60, 0.8).into();
            publisher.note(attack, Span::default()).unwrap();
            drain(&mut consumer);
            let [(queued, first), (grown, span)] = gaps[..] else { panic!("{gaps:?}") };
            let lost = PUBLICATION_RING as u64;
            assert_eq!((queued.first, queued.last), (lost, lost), "the fill's next serial");
            assert_eq!(first, armed(100));
            assert_eq!((grown.first, grown.last), (lost + 1, lost + 2));
            assert_eq!((grown.time, grown.through), (9100.0, 9200.0));
            assert_eq!(span, merged, "{last:?}");
        }
    }

    /// A closure lost in an outage was played at no input, so it must not
    /// widen the range of inputs the gap names: merged as one at sample 0, it
    /// marked every earlier, closed pass incomplete.
    #[test]
    fn a_lost_closure_leaves_the_outage_s_inputs_where_they_were() {
        let (mut publisher, mut consumer) = channel();
        let armed = |input| Span::at(ClockId::default(), input, RecordIntent::armed(1));
        fill(&mut publisher);
        let release = |time| NoteEvent::off(time, SourceId::DIRECT, 0, 60).into();
        assert_eq!(publisher.note(release(9000.0), armed(300)), Err(PublishError::Lost));
        assert_eq!(publisher.pass_closed(1, 1, 9050.0), Err(PublishError::Lost));
        assert_eq!(publisher.note(release(9100.0), armed(400)), Err(PublishError::Lost));
        consumer.drain(|_, _| true);
        publisher.note(release(9200.0), Span::default()).unwrap();
        let mut spans = Vec::new();
        consumer.drain(|delivery, span| {
            if let Delivery::Event(CanonicalEvent::Gap(_)) = delivery {
                spans.push((span.first.input, span.last.input));
            }
            true
        });
        assert_eq!(spans, [(400, 400)], "the held outage spans the note it lost, and no more");
    }

    #[test]
    fn a_full_lane_reports_its_outage_without_another_audio_callback() {
        let (mut publisher, mut consumer) = channel();
        for note in [60, 72] {
            publisher
                .note(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 0.8).into(), Span::default())
                .unwrap();
        }
        // Bends rather than repeated attacks: one onset per key, so a note the
        // snapshot restores can be checked against the onset it really had.
        for i in 2..PUBLICATION_RING - GAP_RESERVE {
            let mut event = NoteEvent::on(i as f64, SourceId::DIRECT, 0, 72, 0.8);
            event.kind = harmonigraph_core::NoteEventKind::Tuning { semitones: 0.1 };
            publisher.note(event.into(), Span::default()).unwrap();
        }
        assert_eq!(publisher.free(), 0, "the fixture must actually fill the lane");
        assert_eq!(
            publisher.note(NoteEvent::off(9000.0, SourceId::DIRECT, 0, 60).into(), Span::default()),
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
        assert!(tracker.history_missing());

        // The refresh restores what is sounding NOW and nothing else: key 60
        // was released while the lane was full and does not come back.
        publisher
            .baseline(&frame(1, 9001.0, &[held(72, 0.0, 7_200_000_000)]), Span::default())
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
        assert!(!restored.history_complete, "the roll records the history it never saw");
        assert!(!tracker.history_missing(), "and nothing is missing any more");
    }

    #[test]
    fn a_declined_snapshot_stays_owned_and_refuses_the_next_without_losing_history() {
        let (mut publisher, mut consumer) = channel();
        let voices: Vec<_> =
            (0..64).map(|note| held(note, 0.0, i64::from(note) * 100_000_000)).collect();
        for id in 1..=SNAPSHOT_SLOTS as u64 {
            publisher.baseline(&frame(id, 1.0, &voices), Span::default()).unwrap();
        }
        assert_eq!(
            publisher.baseline(&frame(99, 1.0, &voices), Span::default()),
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
        publisher.baseline(&frame(99, 2.0, &voices), Span::default()).unwrap();
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
