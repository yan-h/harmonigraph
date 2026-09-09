//! `HG-TUNING` summaries, through the Info-level plugin logger, with editors
//! closed. Audio callbacks store fixed counters into a seqlocked snapshot at
//! roughly one-second audio intervals; all formatting happens on the main
//! thread, at most once a wall-clock second and only when a value changed.
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{session, setup};

const TUNE_FIELDS: [&str; 11] = [
    "epoch",
    "slot",
    "delay",
    "input_on",
    "output_on",
    "captured",
    "misses",
    "dropped",
    "held",
    "pending",
    "status",
];
const HUB_FIELDS: [&str; 9] = [
    "epoch",
    "rows",
    "context",
    "decisions",
    "published",
    "reference_mc",
    "status",
    "direct_held",
    "direct_pending",
];

#[derive(Clone, Copy, Default)]
pub struct TuneReport {
    pub epoch: u64,
    /// The row this Tune holds, plus one; zero when it holds none.
    pub slot: u64,
    pub delay: i64,
    pub notes_in: u64,
    pub notes_out: u64,
    pub captured: u64,
    pub misses: u64,
    pub dropped: u64,
    pub held: u64,
    pub pending: u64,
    pub status: u32,
}
impl TuneReport {
    fn words(self) -> [i64; TUNE_FIELDS.len()] {
        [
            self.epoch as i64,
            self.slot as i64,
            self.delay,
            self.notes_in as i64,
            self.notes_out as i64,
            self.captured as i64,
            self.misses as i64,
            self.dropped as i64,
            self.held as i64,
            self.pending as i64,
            i64::from(self.status),
        ]
    }
}

#[derive(Clone, Copy, Default)]
pub struct HubReport {
    pub epoch: u64,
    pub rows: u64,
    pub context: u64,
    pub decisions: u64,
    pub published: u64,
    pub reference: i64,
    pub status: u32,
    pub direct_held: u64,
    pub direct_pending: u64,
}
impl HubReport {
    fn words(self) -> [i64; HUB_FIELDS.len()] {
        [
            self.epoch as i64,
            self.rows as i64,
            self.context as i64,
            self.decisions as i64,
            self.published as i64,
            self.reference,
            i64::from(self.status),
            self.direct_held as i64,
            self.direct_pending as i64,
        ]
    }
}

pub(super) struct Snapshot<const N: usize> {
    sequence: AtomicU64,
    words: [AtomicI64; N],
}
impl<const N: usize> Default for Snapshot<N> {
    fn default() -> Self {
        Self { sequence: AtomicU64::new(0), words: std::array::from_fn(|_| AtomicI64::new(0)) }
    }
}
impl<const N: usize> Snapshot<N> {
    // One serialized audio owner writes each snapshot. Readers never spin or
    // borrow non-atomic storage while that owner is running.
    pub(super) fn publish(&self, words: [i64; N]) {
        let sequence = self.sequence.load(Ordering::SeqCst);
        self.sequence.store(sequence.wrapping_add(1), Ordering::SeqCst);
        for (cell, value) in self.words.iter().zip(words) {
            cell.store(value, Ordering::SeqCst);
        }
        self.sequence.store(sequence.wrapping_add(2), Ordering::SeqCst);
    }
    pub(super) fn read(&self) -> Option<[i64; N]> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence == 0 || sequence & 1 != 0 {
            return None;
        }
        let words = std::array::from_fn(|i| self.words[i].load(Ordering::SeqCst));
        (sequence == self.sequence.load(Ordering::SeqCst)).then_some(words)
    }
}

#[derive(Default)]
struct Logged {
    at: Option<Instant>,
    value: String,
}

pub(super) struct Shared {
    tune: Snapshot<{ TUNE_FIELDS.len() }>,
    hub: Option<Snapshot<{ HUB_FIELDS.len() }>>,
    /// Frames left before the next snapshot. An audio interval rather than a
    /// wall clock, so an idle plugin never logs.
    countdown: AtomicI64,
    logged: Mutex<Logged>,
}

impl Shared {
    pub(super) fn new(hub: bool) -> Self {
        Self {
            tune: Snapshot::default(),
            hub: hub.then(Snapshot::default),
            countdown: AtomicI64::new(0),
            logged: Mutex::new(Logged::default()),
        }
    }
    pub(super) fn due(&self, frames: u32, rate: f64) -> bool {
        let left = self.countdown.fetch_sub(i64::from(frames), Ordering::AcqRel)
            - i64::from(frames);
        if left > 0 {
            return false;
        }
        self.countdown.store(rate.max(1.0) as i64, Ordering::Release);
        true
    }
    pub(super) fn publish_tune(&self, report: TuneReport) {
        self.tune.publish(report.words());
    }
    pub(super) fn publish_hub(&self, report: HubReport) {
        if let Some(hub) = &self.hub {
            hub.publish(report.words());
        }
    }

    pub(super) fn log(&self, shared: &setup::Shared) {
        let mut logged = self.logged.lock().unwrap();
        let now = Instant::now();
        if logged.at.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1)) {
            return;
        }
        let value = self.report(shared);
        if value != logged.value {
            nice_plug::nice_log!("HG-TUNING {value}");
            logged.value = value;
        }
        logged.at = Some(now);
    }

    fn report(&self, shared: &setup::Shared) -> String {
        let session = session::session();
        let mut value = format!(
            "pid={} role={} build={:?} hubs={} epoch={} status={:?}",
            std::process::id(),
            if shared.is_hub() { "Hub" } else { "Tune" },
            option_env!("HARMONIGRAPH_BUILD").unwrap_or("dev"),
            session.hubs(),
            session.epoch(),
            session::status_text(shared.status()),
        );
        if let Some(words) = self.tune.read() {
            append(&mut value, "tune", &TUNE_FIELDS, &words);
        }
        if let Some(words) = self.hub.as_ref().and_then(Snapshot::read) {
            append(&mut value, "hub", &HUB_FIELDS, &words);
        }
        value
    }
}

fn append(out: &mut String, name: &str, fields: &[&str], words: &[i64]) {
    use std::fmt::Write;
    let _ = write!(out, " {name}{{");
    for (index, (field, word)) in fields.iter().zip(words).enumerate() {
        let _ = write!(out, "{}{field}={word}", if index == 0 { "" } else { ", " });
    }
    out.push('}');
}
