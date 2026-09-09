//! The process-wide session: one Hub, sixteen Tune rows, one epoch.
//!
//! There is no pairing protocol. The rings are built once, with the session,
//! and a side claims its own half on the main thread at activation. What the
//! audio callback does is one atomic load of [`Session::hub`], and one of
//! [`Session::epoch`] to learn whether it owes a cut. Nothing is offered,
//! returned, leased or acknowledged, so there is nothing here that can stall.
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use super::{event::Event, CAPTURE_RING, REPLY_RING, TUNERS};

/// Status bits. Every one of them is cosmetic: a fault shows in the editor and
/// in `HG-TUNING`, and never stops a note. An unassigned note sounds at its
/// raw pitch, which is the whole of what a fault costs here.
pub const NO_HUB: u32 = 1 << 0;
/// A second Harmonigraph in the process. Both show it; the first one loaded
/// keeps the session, because there is no pairing choice to offer.
pub const SECOND_HUB: u32 = 1 << 1;
/// All sixteen rows are held. A seventeenth Tune passes notes through.
pub const NO_ROW: u32 = 1 << 2;
/// A copy did not fit in the ring, so the Hub never saw that input.
pub const RING_FULL: u32 = 1 << 3;
/// The host is not supplying a usable steady sample timeline, which is the one
/// thing that lets two tracks be ordered against each other.
pub const CLOCK: u32 = 1 << 4;
/// The policy refused an evaluation — too much context, too many candidates,
/// coordinate exhaustion. That onset sounds uncorrected.
pub const POLICY: u32 = 1 << 5;
/// A display or take lane lost a report.
pub const PUBLICATION: u32 = 1 << 6;
/// A bounded store was full and an event was dropped outright: the delay line,
/// the held set, or a value the wrapper refused. Unlike every other bit here
/// this one costs a note rather than a correction, so it is named separately.
pub const DROPPED: u32 = 1 << 7;

pub fn status_text(status: u32) -> String {
    let reasons = [
        (NO_HUB, "no Harmonigraph in this process"),
        (SECOND_HUB, "a second Harmonigraph is loaded"),
        (NO_ROW, "all sixteen Tune rows are held"),
        (RING_FULL, "the copy queue is full"),
        (CLOCK, "the host supplies no steady sample timeline"),
        (POLICY, "the policy refused an evaluation"),
        (PUBLICATION, "the display or take lane lost a report"),
        (DROPPED, "a retained event did not fit and was dropped"),
    ];
    let named: Vec<_> =
        reasons.iter().filter(|(bit, _)| status & bit != 0).map(|(_, why)| *why).collect();
    if named.is_empty() {
        return "No faults".to_owned();
    }
    format!("Notes pass through uncorrected: {}", named.join(", "))
}

/// One copied input record. Everything the Hub needs to order and assign this
/// input is here; nothing points back into the Tune's storage.
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    pub epoch: u64,
    /// The originating Tune's own input sequence. A reply addresses this.
    pub serial: u64,
    /// Absolute input sample on the host's steady timeline.
    pub sample: i64,
    pub event: Event,
}

/// The whole of what the Hub says back. A correction reaches its onset by
/// serial or it does not reach it at all.
#[derive(Clone, Copy, Debug)]
pub struct Reply {
    pub epoch: u64,
    pub serial: u64,
    /// Full, unwrapped adaptive correction in microcents.
    pub correction: i64,
}

pub struct TuneEnds {
    pub captures: rtrb::Producer<Capture>,
    pub replies: rtrb::Consumer<Reply>,
}
pub struct HubEnds {
    pub captures: rtrb::Consumer<Capture>,
    pub replies: rtrb::Producer<Reply>,
}

#[derive(Default)]
struct Ends {
    tune: Option<TuneEnds>,
    hub: Option<HubEnds>,
}

pub struct Row {
    /// Registration id of the Tune holding this row, or zero.
    owner: AtomicU64,
    /// This Tune's activation-fixed delay in samples. The Tune is the only
    /// writer; the Hub reads it for the time it reports each sequenced input
    /// was scheduled for, which is that Tune's own input plus D.
    pub delay: AtomicI64,
    /// Onsets that emitted without a correction, since the last cut.
    pub misses: AtomicU64,
    ends: Mutex<Ends>,
}

impl Row {
    fn new() -> Self {
        let (captures, hub_captures) = rtrb::RingBuffer::new(CAPTURE_RING);
        let (replies, hub_replies) = rtrb::RingBuffer::new(REPLY_RING);
        Self {
            owner: AtomicU64::new(0),
            delay: AtomicI64::new(0),
            misses: AtomicU64::new(0),
            ends: Mutex::new(Ends {
                tune: Some(TuneEnds { captures, replies: hub_replies }),
                hub: Some(HubEnds { captures: hub_captures, replies }),
            }),
        }
    }
    pub fn held(&self) -> bool {
        self.owner.load(Ordering::Acquire) != 0
    }
}

pub struct Session {
    /// Registration id of the Hub that owns this session, or zero. This is the
    /// slot a Tune loads, and loading it is the whole of pairing.
    hub: AtomicU64,
    /// Harmonigraphs loaded in this process. More than one is a fault on all
    /// of them rather than a choice to resolve.
    hubs: AtomicUsize,
    /// Bumped by every attach, detach and explicit Reset. A Tune that adopts a
    /// new value cuts; that is the entire lifecycle protocol.
    epoch: AtomicU64,
    /// The epoch an explicit Reset produced. The Hub clears released memory
    /// for that one and keeps it across a membership change.
    reset_epoch: AtomicU64,
    /// The Hub's "apply this multiplier to every Tune", as generation in the
    /// high half and multiplier in the low. A generation rather than a value
    /// to consume, so every Tune sees the same request.
    delay_request: AtomicU64,
    next: AtomicU64,
    rows: [Row; TUNERS],
}

/// Allocated once, on whichever instance registers first, and never freed. The
/// rings are ~1.5 MB for the process rather than per instance, which is what
/// pays for pairing being a load rather than a handoff.
pub fn session() -> &'static Session {
    static SESSION: OnceLock<Session> = OnceLock::new();
    SESSION.get_or_init(|| Session {
        hub: AtomicU64::new(0),
        hubs: AtomicUsize::new(0),
        epoch: AtomicU64::new(1),
        reset_epoch: AtomicU64::new(0),
        delay_request: AtomicU64::new(0),
        next: AtomicU64::new(0),
        rows: std::array::from_fn(|_| Row::new()),
    })
}

impl Session {
    pub fn row(&self, slot: u8) -> &Row {
        &self.rows[usize::from(slot)]
    }
    /// One atomic load. Zero means no Hub, which is a status, not a silence.
    pub fn hub(&self) -> u64 {
        self.hub.load(Ordering::Acquire)
    }
    pub fn hubs(&self) -> usize {
        self.hubs.load(Ordering::Acquire)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    /// True when `epoch` is the one an explicit Reset produced, as opposed to
    /// a membership change. Both cut; only this one clears musical memory.
    pub fn is_reset(&self, epoch: u64) -> bool {
        self.reset_epoch.load(Ordering::Acquire) == epoch
    }
    pub fn register(&self) -> u64 {
        self.next.fetch_add(1, Ordering::AcqRel) + 1
    }
    fn bump(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }
    /// Explicit Reset. Every paired Tune cuts and the Hub clears its context.
    pub fn reset(&self) {
        let epoch = self.bump();
        self.reset_epoch.store(epoch, Ordering::Release);
    }

    /// Main thread. The first Harmonigraph to register owns the session; a
    /// later one gets no rings and both carry `SECOND_HUB`.
    pub fn attach_hub(&self, id: u64) -> Option<Box<[Option<HubEnds>; TUNERS]>> {
        self.hubs.fetch_add(1, Ordering::AcqRel);
        if self.hub.compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return None;
        }
        self.bump();
        Some(Box::new(std::array::from_fn(|slot| self.rows[slot].ends.lock().unwrap().hub.take())))
    }
    /// Main thread. Giving the rings back is what lets a reloaded Harmonigraph
    /// take the same session over.
    pub fn detach_hub(&self, id: u64, ends: Option<Box<[Option<HubEnds>; TUNERS]>>) {
        self.hubs.fetch_sub(1, Ordering::AcqRel);
        if let Some(ends) = ends {
            for (slot, end) in ends.into_iter().enumerate() {
                self.rows[slot].ends.lock().unwrap().hub = end;
            }
        }
        let _ = self.hub.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
        self.bump();
    }

    /// A free row, or nothing — a seventeenth Tune is refused visibly and keeps
    /// passing its notes through. Safe from a callback: the claim is a compare
    /// exchange and the endpoints come out under `try_lock`, which never
    /// blocks. A Tune that loses the race simply asks again next callback.
    pub fn try_attach_row(&self, id: u64) -> Option<(u8, TuneEnds)> {
        for (slot, row) in self.rows.iter().enumerate() {
            if row.owner.compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let ends = row.ends.try_lock().ok().and_then(|mut ends| ends.tune.take());
                let Some(ends) = ends else {
                    row.owner.store(0, Ordering::Release);
                    continue;
                };
                row.misses.store(0, Ordering::Release);
                self.bump();
                return Some((slot as u8, ends));
            }
        }
        None
    }
    /// Main thread. Records the departing Tune left in the ring are refused by
    /// the Hub on their epoch, so there is nothing to drain here.
    pub fn detach_row(&self, slot: u8, id: u64, ends: TuneEnds) {
        let row = &self.rows[usize::from(slot)];
        row.ends.lock().unwrap().tune = Some(ends);
        row.delay.store(0, Ordering::Release);
        let _ = row.owner.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
        self.bump();
    }

    /// The Hub asking every Tune in the process to adopt one multiplier. Each
    /// Tune writes its own parameter for it, on its own main thread, so no
    /// instance ever owns another's saved value.
    pub fn request_delay(&self, multiplier: u32) {
        let generation = (self.delay_request.load(Ordering::Acquire) >> 32) + 1;
        self.delay_request.store(generation << 32 | u64::from(multiplier), Ordering::Release);
    }
    pub fn delay_request(&self) -> (u64, u32) {
        let value = self.delay_request.load(Ordering::Acquire);
        (value >> 32, value as u32)
    }

    /// The Tune's own miss counter, mirrored where the editor and the Hub's
    /// diagnostics can read it without touching the audio owner.
    pub fn row_misses(&self, slot: Option<u8>, misses: u64) {
        if let Some(slot) = slot {
            self.rows[usize::from(slot)].misses.store(misses, Ordering::Release);
        }
    }

    #[cfg(test)]
    pub fn test_reset(&self) {
        for row in &self.rows {
            row.owner.store(0, Ordering::Release);
        }
        self.hub.store(0, Ordering::Release);
        self.hubs.store(0, Ordering::Release);
    }
}

/// A Tune's claim on one row. Holding this is what "paired" means; there is no
/// second flag anywhere that has to agree with it.
pub struct Attached {
    pub slot: u8,
    pub ends: TuneEnds,
}

const _: () = assert!(std::mem::size_of::<Capture>() <= 80);
const _: () = assert!(std::mem::size_of::<Reply>() <= 24);
