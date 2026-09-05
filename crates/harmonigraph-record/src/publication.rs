//! Single-producer canonical publication. Musical retention acknowledgement is
//! the caller's audio-owned responsibility and never waits for these consumers.
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use harmonigraph_core::canonical::{
    CanonicalEvent, GapReason, NoteDelta, PublicationGap, SourceBaseline,
};

use crate::configuration::RecordAddress;

pub const PUBLICATION_RING: usize = 4096;
pub const SOURCE_ROWS: usize = 17;
pub const BASELINES_PER_SOURCE: usize = 2;
const BASELINES: usize = SOURCE_ROWS * BASELINES_PER_SOURCE;
const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const READING: u8 = 3;

/// Resolved on audio from the ORIGINAL actual-output recording segment. None
/// is explicit disarmed provenance; a drainer never consults today's arm state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Route {
    pub address: Option<RecordAddress>,
    /// Presentation-clock time + offset = original pass's transport time.
    pub time_offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Handle {
    slot: u8,
    generation: u64,
}

struct BaselineSlot {
    state: AtomicU8,
    generation: AtomicU64,
    value: UnsafeCell<Option<SourceBaseline>>,
}

// Only the single publisher writes (after Empty -> Writing); only the single
// consumer reads (after Ready -> Reading). Release/Acquire transfers ownership.
// A handle's generation is checked while Reading, before ordinary payload access.
unsafe impl Sync for BaselineSlot {}

impl Default for BaselineSlot {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            generation: AtomicU64::new(0),
            value: UnsafeCell::new(None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Value {
    Note(NoteDelta),
    Baseline(Handle),
    Gap(PublicationGap),
    PassComplete(RecordAddress),
    EpochComplete(u64),
}

#[derive(Clone, Copy, Debug)]
struct Item {
    serial: u64,
    observation_time: f64,
    route: Route,
    value: Value,
}

/// All payload words are atomic, so a concurrent read is memory-safe even if
/// the single bounded version check fails. A consumer retries on its NEXT poll.
#[derive(Default)]
struct Loss {
    version: AtomicU64,
    first: AtomicU64,
    last: AtomicU64,
    time: AtomicU64,
    through: AtomicU64,
    epoch: AtomicU64,
    pass: AtomicU64,
    offset: AtomicU64,
}

impl Loss {
    fn write(&self, gap: PublicationGap, route: Route) {
        let version = self.version.load(Ordering::Relaxed);
        // Counter exhaustion is terminal and leaves the previous durable loss.
        let Some(next) = version.checked_add(2) else { return };
        self.version.fetch_add(1, Ordering::AcqRel);
        self.first.store(gap.first, Ordering::Relaxed);
        self.last.store(gap.last, Ordering::Relaxed);
        self.time.store(gap.time.to_bits(), Ordering::Relaxed);
        self.through.store(gap.through.to_bits(), Ordering::Relaxed);
        self.epoch.store(route.address.map_or(0, |a| a.epoch), Ordering::Relaxed);
        self.pass.store(route.address.map_or(0, |a| u64::from(a.pass)), Ordering::Relaxed);
        self.offset.store(route.time_offset.to_bits(), Ordering::Relaxed);
        self.version.store(next, Ordering::Release);
    }

    fn read(&self) -> Option<(PublicationGap, Route)> {
        let before = self.version.load(Ordering::Acquire);
        if before == 0 || before & 1 != 0 {
            return None;
        }
        let first = self.first.load(Ordering::Relaxed);
        let last = self.last.load(Ordering::Relaxed);
        let time = f64::from_bits(self.time.load(Ordering::Relaxed));
        let through = f64::from_bits(self.through.load(Ordering::Relaxed));
        let epoch = self.epoch.load(Ordering::Relaxed);
        let pass = self.pass.load(Ordering::Relaxed) as u32;
        let offset = f64::from_bits(self.offset.load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.version.load(Ordering::Acquire)).then_some((
            PublicationGap {
                source: None,
                time,
                through,
                first,
                last,
                reason: GapReason::PublicationFull,
            },
            Route {
                address: (epoch != 0).then_some(RecordAddress { epoch, pass }),
                time_offset: offset,
            },
        ))
    }
}

struct Shared {
    slots: Box<[BaselineSlot]>,
    loss: Loss,
    clock: AtomicU64,
}

pub struct Publisher {
    ring: rtrb::Producer<Item>,
    shared: Arc<Shared>,
    serial: u64,
    pending_gap: Option<(PublicationGap, Route)>,
}

pub struct Consumer {
    ring: rtrb::Consumer<Item>,
    shared: Arc<Shared>,
    seen: u64,
    pending: Option<Item>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    /// An actual canonical publication attempt failed. The independent loss
    /// descriptor remains readable even if audio never calls again.
    Lost,
    /// Both payloads remain owned by the drainer. Caller retains its complete
    /// baseline; this alone creates no fictional history loss.
    BaselineBusy,
    Invalid,
}

pub enum Delivery<'a> {
    Event(CanonicalEvent<'a>),
    PassComplete(RecordAddress),
    EpochComplete(u64),
}

pub fn channel() -> (Publisher, Consumer) {
    let (producer, consumer) = rtrb::RingBuffer::new(PUBLICATION_RING);
    let shared = Arc::new(Shared {
        slots: (0..BASELINES).map(|_| BaselineSlot::default()).collect(),
        loss: Loss::default(),
        clock: AtomicU64::new(f64::NAN.to_bits()),
    });
    (
        Publisher { ring: producer, shared: shared.clone(), serial: 0, pending_gap: None },
        Consumer { ring: consumer, shared, seen: 0, pending: None },
    )
}

impl Publisher {
    /// A fresh hub clock observation, independent of delayed history and cuts.
    pub fn observe_clock(&self, time: f64) {
        if time.is_finite() {
            self.shared.clock.store(time.to_bits(), Ordering::Release);
        }
    }
    pub fn free(&self) -> usize {
        self.ring.slots()
    }

    /// Explicit downstream fanout loss when that consumer cannot retain a
    /// complete payload. This is distinct from caller-owned baseline backpressure.
    pub fn discarded(&mut self, time: f64, route: Route) {
        if let Some(serial) = self.serial.checked_add(1) {
            self.serial = serial;
            self.lost(time, route);
        }
    }

    fn flush_gap(&mut self, observation_time: f64) -> bool {
        if let Some((gap, route)) = self.pending_gap {
            if self
                .ring
                .push(Item { serial: gap.last, observation_time, route, value: Value::Gap(gap) })
                .is_err()
            {
                return false;
            }
            self.pending_gap = None;
        }
        true
    }

    fn lost(&mut self, time: f64, route: Route) -> PublishError {
        let gap = match self.pending_gap {
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
        let route = match self.pending_gap {
            Some((_, old)) if old != route => Route::default(),
            _ => route,
        };
        self.pending_gap = Some((gap, route));
        self.shared.loss.write(gap, route);
        PublishError::Lost
    }

    fn push(
        &mut self,
        value: Value,
        time: f64,
        observation_time: f64,
        route: Route,
    ) -> Result<(), PublishError> {
        let Some(serial) = self.serial.checked_add(1) else { return Err(PublishError::Invalid) };
        if !time.is_finite() || !observation_time.is_finite() || !route.time_offset.is_finite() {
            return Err(PublishError::Invalid);
        }
        let flushed = self.flush_gap(observation_time);
        self.serial = serial;
        if !flushed || self.ring.push(Item { serial, observation_time, route, value }).is_err() {
            return Err(self.lost(time, route));
        }
        Ok(())
    }

    pub fn note(
        &mut self,
        note: NoteDelta,
        observation_time: f64,
        route: Route,
    ) -> Result<(), PublishError> {
        note.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Note(note), note.event.time, observation_time, route)
    }

    pub fn gap(
        &mut self,
        gap: PublicationGap,
        observation_time: f64,
        route: Route,
    ) -> Result<(), PublishError> {
        gap.validate().map_err(|_| PublishError::Invalid)?;
        self.push(Value::Gap(gap), gap.time, observation_time, route)
    }

    pub fn baseline(
        &mut self,
        row: usize,
        baseline: &SourceBaseline,
        observation_time: f64,
        route: Route,
    ) -> Result<(), PublishError> {
        baseline.validate().map_err(|_| PublishError::Invalid)?;
        if row >= SOURCE_ROWS {
            return Err(PublishError::Invalid);
        }
        for index in row * BASELINES_PER_SOURCE..(row + 1) * BASELINES_PER_SOURCE {
            let slot = &self.shared.slots[index];
            if slot
                .state
                .compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            let Some(generation) = slot.generation.load(Ordering::Relaxed).checked_add(1) else {
                slot.state.store(EMPTY, Ordering::Release);
                return Err(PublishError::Invalid);
            };
            // SAFETY: successful acquisition owns the entire empty slot.
            unsafe {
                *slot.value.get() = Some(*baseline);
            }
            slot.generation.store(generation, Ordering::Relaxed);
            slot.state.store(READY, Ordering::Release);
            let handle = Handle { slot: index as u8, generation };
            let result = self.push(Value::Baseline(handle), baseline.time, observation_time, route);
            if result.is_err() {
                // No handle was published, so no reader can own this payload.
                let slot = &self.shared.slots[index];
                unsafe {
                    *slot.value.get() = None;
                }
                slot.state.store(EMPTY, Ordering::Release);
            }
            return result;
        }
        Err(PublishError::BaselineBusy)
    }

    pub fn pass_complete(
        &mut self,
        address: RecordAddress,
        observation_time: f64,
    ) -> Result<(), PublishError> {
        self.push(
            Value::PassComplete(address),
            observation_time,
            observation_time,
            Route::default(),
        )
    }

    pub fn epoch_complete(
        &mut self,
        epoch: u64,
        observation_time: f64,
    ) -> Result<(), PublishError> {
        self.push(Value::EpochComplete(epoch), observation_time, observation_time, Route::default())
    }
}

impl Consumer {
    pub fn clock(&self) -> Option<f64> {
        let time = f64::from_bits(self.shared.clock.load(Ordering::Acquire));
        time.is_finite().then_some(time)
    }
    /// One bounded drain of the actual selected ring capacity. The borrowed
    /// baseline cannot escape the call; deferred consumers must make an owned
    /// copy BEFORE this returns and permits reuse.
    pub fn drain(&mut self, mut consume: impl FnMut(Delivery<'_>, f64, Route) -> bool) -> usize {
        let mut count = 0;
        for _ in 0..PUBLICATION_RING {
            let Some(item) = self.pending.take().or_else(|| self.ring.pop().ok()) else { break };
            if item.serial <= self.seen {
                continue;
            }
            let consumed = match item.value {
                Value::Note(note) => consume(
                    Delivery::Event(CanonicalEvent::Note(note)),
                    item.observation_time,
                    item.route,
                ),
                Value::Gap(gap) => consume(
                    Delivery::Event(CanonicalEvent::Gap(gap)),
                    item.observation_time,
                    item.route,
                ),
                Value::PassComplete(address) => {
                    consume(Delivery::PassComplete(address), item.observation_time, item.route)
                }
                Value::EpochComplete(epoch) => {
                    consume(Delivery::EpochComplete(epoch), item.observation_time, item.route)
                }
                Value::Baseline(handle) => {
                    let slot = &self.shared.slots[usize::from(handle.slot)];
                    assert_eq!(
                        slot.state.compare_exchange(
                            READY,
                            READING,
                            Ordering::Acquire,
                            Ordering::Relaxed
                        ),
                        Ok(READY)
                    );
                    assert_eq!(slot.generation.load(Ordering::Relaxed), handle.generation);
                    // SAFETY: exclusive Reading ownership lasts through fanout.
                    let baseline = unsafe { (&*slot.value.get()).as_ref().unwrap() };
                    let consumed = consume(
                        Delivery::Event(CanonicalEvent::Baseline(baseline)),
                        item.observation_time,
                        item.route,
                    );
                    if consumed {
                        unsafe {
                            *slot.value.get() = None;
                        }
                    }
                    slot.state.store(if consumed { EMPTY } else { READY }, Ordering::Release);
                    consumed
                }
            };
            if !consumed {
                self.pending = Some(item);
                break;
            }
            self.seen = item.serial;
            count += 1;
        }
        if let Some((gap, route)) = self.shared.loss.read() {
            // A loss cannot overtake successful history still in the ring.
            // This is also what makes a no-further-callback outage observable.
            if gap.last > self.seen
                && self.seen >= gap.first - 1
                && consume(Delivery::Event(CanonicalEvent::Gap(gap)), gap.through, route)
            {
                self.seen = gap.last;
                count += 1;
            }
        }
        count
    }
}

const _: () = assert!(std::mem::size_of::<Item>() <= 256);
const _: () = assert!(std::mem::align_of::<Item>() <= 8);
const _: () = assert!(std::mem::size_of::<BaselineSlot>() <= 16 * 1024);
const _: () = assert!(std::mem::align_of::<BaselineSlot>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::canonical::{ChannelBaseline, VoiceBaseline};
    use harmonigraph_core::{NoteEvent, SourceId};

    #[test]
    fn actual_publication_capacity_retains_loss_without_another_audio_callback() {
        let (mut publisher, mut consumer) = channel();
        for i in 0..PUBLICATION_RING {
            publisher
                .note(
                    NoteEvent::on(i as f64, SourceId::DIRECT, 0, 60, 0.8).into(),
                    i as f64,
                    Route::default(),
                )
                .unwrap();
        }
        assert_eq!(publisher.free(), 0);
        assert_eq!(
            publisher.note(
                NoteEvent::off(4096.0, SourceId::DIRECT, 0, 60).into(),
                4096.0,
                Route::default()
            ),
            Err(PublishError::Lost)
        );
        let mut notes = 0;
        let mut gaps = Vec::new();
        assert_eq!(
            consumer.drain(|delivery, _, _| {
                match delivery {
                    Delivery::Event(CanonicalEvent::Note(_)) => notes += 1,
                    Delivery::Event(CanonicalEvent::Gap(gap)) => gaps.push(gap),
                    _ => panic!("unexpected control"),
                }
                true
            }),
            PUBLICATION_RING + 1
        );
        assert_eq!(notes, PUBLICATION_RING);
        assert_eq!((gaps[0].first, gaps[0].last), (4097, 4097));
        assert_eq!(consumer.drain(|_, _, _| panic!("duplicate loss")), 0);
        let mut tracker = harmonigraph_core::NoteTracker::new();
        tracker.handle_canonical(CanonicalEvent::Gap(gaps[0])).unwrap();
        assert!(!tracker.source_current_certain(SourceId::DIRECT));
        // Resume the actual full queue, including its queued duplicate loss
        // marker, and transfer a complete recovery through the owned bank.
        publisher
            .note(
                NoteEvent::on(4097.0, SourceId::DIRECT, 0, 72, 0.8).into(),
                4097.0,
                Route::default(),
            )
            .unwrap();
        consumer.drain(|event, _, _| {
            let Delivery::Event(event) = event else { panic!() };
            tracker.handle_canonical(event).unwrap();
            true
        });
        assert!(
            !tracker.source_current_certain(SourceId::DIRECT),
            "a new On repairs only its own lifetime"
        );
        let recovered = SourceBaseline::new(
            SourceId::DIRECT,
            1,
            4098.0,
            4098.0,
            0,
            true,
            &[VoiceBaseline {
                note: 72,
                actual_onset: 4097.0,
                input_onset: 4097.0,
                velocity: 0.8,
                pitch_microcents: 7_200_000_000,
                ..Default::default()
            }],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        publisher.baseline(0, &recovered, 4098.0, Route::default()).unwrap();
        publisher
            .note(NoteEvent::off(4099.0, SourceId::DIRECT, 0, 72).into(), 4099.0, Route::default())
            .unwrap();
        consumer.drain(|event, _, _| {
            let Delivery::Event(event) = event else { panic!() };
            tracker.handle_canonical(event).unwrap();
            true
        });
        assert!(tracker.source_current_certain(SourceId::DIRECT));
        assert!(
            !tracker.source_current_certain(SourceId(1)),
            "one baseline cannot restore another source"
        );
        assert_eq!(tracker.held_count(), 0);
        assert_eq!(
            tracker.publication_gaps().len(),
            1,
            "history loss remains after state recovery"
        );
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!((note.start, note.end), (4097.0, Some(4099.0)));
        assert!(note.history_complete);
        assert!(consumer
            .shared
            .slots
            .iter()
            .all(|slot| slot.state.load(Ordering::Acquire) == EMPTY));
    }

    #[test]
    fn both_complete_payload_slots_stay_owned_until_full_fanout_copy() {
        let (mut publisher, mut consumer) = channel();
        let voices: Vec<_> = (0..64)
            .map(|note| VoiceBaseline {
                note,
                pitch_microcents: i64::from(note) * 100_000_000,
                velocity: 0.8,
                ..Default::default()
            })
            .collect();
        let first = SourceBaseline::new(
            SourceId::DIRECT,
            1,
            1.0,
            0.0,
            0,
            true,
            &voices,
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let mut second = first;
        second.id = 2;
        publisher.baseline(0, &first, 1.0, Route::default()).unwrap();
        publisher.baseline(0, &second, 1.0, Route::default()).unwrap();
        assert_eq!(
            publisher.baseline(0, &second, 1.0, Route::default()),
            Err(PublishError::BaselineBusy)
        );
        assert_eq!(consumer.drain(|_, _, _| false), 0, "deferred drainer retains Reading payload");
        assert_eq!(
            publisher.baseline(0, &second, 1.0, Route::default()),
            Err(PublishError::BaselineBusy)
        );
        let mut saved = None;
        consumer.drain(|delivery, _, _| {
            if saved.is_some() {
                return false;
            }
            let Delivery::Event(CanonicalEvent::Baseline(frame)) = delivery else { panic!() };
            saved = Some(harmonigraph_take::canonical::BaselineRecord::from(frame));
            true
        });
        second.id = 3;
        publisher.baseline(0, &second, 2.0, Route::default()).unwrap();
        assert_eq!(
            saved.as_ref().unwrap().baseline().unwrap(),
            first,
            "deferred writer owns its own entire old payload"
        );
        consumer.drain(|_, _, _| true);
        assert!(consumer
            .shared
            .slots
            .iter()
            .all(|slot| slot.state.load(Ordering::Acquire) == EMPTY));
        eprintln!("canonical layouts: NoteDelta={} Item={} VoiceBaseline={} SourceBaseline={} slot={} ring_payload={} baseline_bank={}", std::mem::size_of::<NoteDelta>(), std::mem::size_of::<Item>(), std::mem::size_of::<VoiceBaseline>(), std::mem::size_of::<SourceBaseline>(), std::mem::size_of::<BaselineSlot>(), PUBLICATION_RING * std::mem::size_of::<Item>(), BASELINES * std::mem::size_of::<BaselineSlot>());
    }
}
