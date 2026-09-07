//! Bounded audio snapshots, drained by the existing setup main-thread service.
//! These values observe ownership; none of them authorize or change playback.
use std::fmt::Write;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{event::Event, protocol::TUNERS, setup};

pub(super) const SOURCE_FIELDS: &[&str] = &[
    "session",
    "source",
    "slot",
    "incarnation",
    "epoch",
    "setup_started",
    "setup_wait",
    "clock_valid",
    "adoption",
    "emission_gate",
    "withdrawn",
    "session_closing",
    "participating",
    "held",
    "pending",
    "old_pending",
    "captures",
    "journal",
    "emergency",
    "baseline_cut",
    "baseline_acked",
    "recovery",
    "output_settled",
    "cancel_settled",
    "faults",
    "extra_delay",
    "input_on",
    "input_off",
    "last_input_key",
    "output_on",
    "output_off",
    "last_output_key",
    "last_output_pitch_mc",
    "assignments",
    "last_decision",
    "last_correction_mc",
    "committed_decision",
    "output_cut",
    "sent_cut",
    "ack_cut",
    "raw_end",
    "coverage_through",
    "ack_through",
    "input_cut",
    "cancel_cut",
    "pending_gate",
    "output_settlement_wait",
    "status_query",
    "last_output_player_mc",
    "last_output_correction_mc",
];
pub(super) const HUB_FIELDS: &[&str] = &[
    "session",
    "epoch",
    "transition",
    "transition_wait",
    "clock_valid",
    "invalidated",
    "recovery_phase",
    "recovery_source",
    "decision",
    "publication_through",
    "raw_end",
    "published_on",
    "published_off",
    "last_published_source",
    "last_published_key",
    "last_published_pitch_mc",
    "config_revision",
    "origin_mc",
    "fifth_mc",
    "third_mc",
    "seventh_mc",
    "meantone_lock",
    "kleisma_lock",
    "auto_meantone",
    "auto_kleisma",
    "learn",
    "input_wait",
    "input_wait_source",
    "publication_wait",
    "publication_wait_source",
    "session_credits",
];
pub(super) const ROW_FIELDS: &[&str] = &[
    "source",
    "incarnation",
    "member",
    "joining",
    "coverage_through",
    "input_through",
    "received",
    "applied",
    "output_queued",
    "input_queued",
    "baseline_cut",
    "held",
    "membership",
    "input_membership",
    "gate",
    "withdrawn",
    "source_detached",
    "hub_detached",
    "seal",
    "producer_joined",
    "faults",
];

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
    pub source: Snapshot<{ SOURCE_FIELDS.len() }>,
    pub hub: Option<Snapshot<{ HUB_FIELDS.len() }>>,
    pub rows: Option<Box<[Snapshot<{ ROW_FIELDS.len() }>; TUNERS]>>,
    logged: Mutex<Logged>,
}
impl Shared {
    pub(super) fn new(hub: bool) -> Self {
        Self {
            source: Snapshot::default(),
            hub: hub.then(Snapshot::default),
            rows: hub.then(|| Box::new(std::array::from_fn(|_| Snapshot::default()))),
            logged: Mutex::new(Logged::default()),
        }
    }
    pub(super) fn log(&self, shared: &setup::Shared) {
        let mut logged = self.logged.lock().unwrap();
        let now = Instant::now();
        if logged.at.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1)) {
            return;
        }
        let (value, key) = self.report(shared);
        if key != logged.value {
            nice_plug::nice_log!("HG-TUNING {value}");
            logged.value = key;
        }
        logged.at = Some(now);
    }
    pub(super) fn report(&self, shared: &setup::Shared) -> (String, String) {
        let requested = shared.value();
        let adopted = shared.adopted();
        let mut value = format!(
            "pid={} instance={:?} build=\"{}\" role={} selected={} accepted_gen={} reset={} adopted={:?}",
            std::process::id(),
            shared.registration(),
            harmonigraph_perf::BUILD_TAG,
            if self.hub.is_some() { "Hub" } else { "Tune" },
            match requested.routing {
                setup::Routing::Hub(hub) => hub.uuid.to_string(),
                setup::Routing::Source(source) =>
                    source.selected.map_or_else(|| "auto".into(), |id| id.to_string()),
            },
            requested.generation,
            requested.reset,
            adopted,
        );
        let mut key = value.clone();
        let source = self.source.read();
        let offset = adopted.map_or(0, |clock| clock.calibration.offset);
        let rate = adopted.map_or(1, |clock| clock.sample_rate.max(1.0) as i64);
        let source_end = source.map_or(0, |data| data[40].saturating_add(offset));
        append(&mut value, &mut key, "source", SOURCE_FIELDS, source, source_end, rate);
        let mut hub_end = source_end;
        if let Some(hub) = &self.hub {
            let data = hub.read();
            hub_end = data.map_or(0, |data| data[10].saturating_add(offset));
            append(&mut value, &mut key, "hub", HUB_FIELDS, data, hub_end, rate);
        }
        if let Some(rows) = &self.rows {
            for (index, row) in rows.iter().enumerate() {
                if let Some(row) = row.read().filter(|row| row[1] != 0) {
                    append(
                        &mut value,
                        &mut key,
                        &format!("row{}", index + 1),
                        ROW_FIELDS,
                        Some(row),
                        hub_end,
                        rate,
                    );
                }
            }
        }
        (value, key)
    }
}
fn append<const N: usize>(
    out: &mut String,
    key: &mut String,
    name: &str,
    labels: &[&str],
    data: Option<[i64; N]>,
    end: i64,
    rate: i64,
) {
    let _ = write!(out, " {name}={{");
    let _ = write!(key, " {name}={{");
    if let Some(data) = data {
        for (i, (label, value)) in labels.iter().zip(data).enumerate() {
            let _ = write!(out, "{}{label}={value}", if i == 0 { "" } else { "," });
            if let Some(reason) = reason(label, value) {
                let _ = write!(out, "({reason})");
            }
            // Absolute sample clocks never trigger idle logging. A growing
            // coverage lag still changes the key once per second, exposing a
            // stalled peer without logging every healthy callback advance.
            if *label != "raw_end" {
                let value = if matches!(
                    *label,
                    "coverage_through"
                        | "ack_through"
                        | "input_through"
                        | "publication_through"
                        | "joining"
                ) && value != i64::MIN
                {
                    end.saturating_sub(value) / rate.max(1)
                } else {
                    value
                };
                let _ = write!(key, "{label}={value},");
            }
        }
    } else {
        out.push_str("no_audio_snapshot");
        key.push_str("no_audio_snapshot");
    }
    out.push('}');
    key.push('}');
}

fn reason(label: &str, value: i64) -> Option<&'static str> {
    let names: &[&str] = match label {
        "setup_wait" => &[
            "none",
            "participation_capture",
            "hub_clock_owner",
            "accepted_history",
            "old_lease",
            "invalid_routing",
            "recovery",
            "output_settlement",
            "cancel_cut",
        ],
        "transition_wait" => &[
            "none",
            "terminal_needs_reset",
            "clock_exhausted",
            "invalid_routing",
            "direct_settlement",
            "direct_publication",
            "no_session",
            "row_detach_or_retention",
            "work_budget",
            "epoch_exhausted",
            "recording_clock",
        ],
        "input_wait" => &[
            "none",
            "direct_coverage",
            "direct_capture",
            "row_enrollment",
            "row_membership",
            "row_input_coverage",
            "row_capture",
            "empty_common_interval",
        ],
        "publication_wait" => &[
            "none",
            "no_callback",
            "row_join_boundary",
            "row_missing_coverage",
            "row_coverage_frontier",
            "output_assignment_lookup",
        ],
        "adoption" => &["pending", "sent", "joined"],
        "recovery_phase" => &[
            "idle",
            "prepare",
            "inventory",
            "preserve",
            "status",
            "rebuild",
            "replay",
            "resume",
            "finish",
        ],
        _ => return None,
    };
    usize::try_from(value).ok().and_then(|index| names.get(index).copied())
}

#[derive(Default)]
pub(super) struct Counts {
    pub input_on: u64,
    pub input_off: u64,
    pub last_input_key: Option<u8>,
    pub output_on: u64,
    pub output_off: u64,
    pub last_output_key: Option<u8>,
    pub last_output_pitch: i64,
    pub last_output_player: i64,
    pub last_output_correction: i64,
    pub assignments: u64,
    pub decision: u64,
    pub correction: i32,
    pub last_source: u64,
    pub setup_wait: i64,
    pub publication_wait: i64,
    pub publication_source: usize,
    pub input_wait: i64,
    pub input_source: usize,
    frames_until_snapshot: u64,
}
impl Counts {
    pub(super) fn published(&mut self, delta: harmonigraph_core::canonical::NoteDelta) {
        use harmonigraph_core::NoteEventKind;
        match delta.event.kind {
            NoteEventKind::On { .. } => {
                self.output_on = self.output_on.saturating_add(1);
                self.last_output_key = Some(delta.event.note);
                self.last_source = delta.event.source.0;
            }
            NoteEventKind::Off => self.output_off = self.output_off.saturating_add(1),
            _ => {}
        }
        if let Some(pitch) = delta.pitch_microcents {
            self.last_source = delta.event.source.0;
            self.last_output_key = Some(delta.event.note);
            self.last_output_pitch = pitch;
        }
    }
    pub(super) fn due(&mut self, frames: u32, rate: f64) -> bool {
        self.frames_until_snapshot = self.frames_until_snapshot.saturating_sub(u64::from(frames));
        if self.frames_until_snapshot != 0 {
            return false;
        }
        self.frames_until_snapshot = rate.max(1.0) as u64;
        true
    }
    pub(super) fn input(&mut self, event: Event) {
        if let Some((_, _, key, _)) = event.attack() {
            self.input_on = self.input_on.saturating_add(1);
            self.last_input_key = Some(key);
        } else if event.release() {
            self.input_off = self.input_off.saturating_add(1);
        }
    }
    pub(super) fn output(&mut self, event: Event, pitch: Option<(u8, i64, f64, i64)>, source: u64) {
        if let Some((_, _, key, _)) = event.attack() {
            self.output_on = self.output_on.saturating_add(1);
            self.last_output_key = Some(key);
            self.last_source = source;
        } else if event.release() {
            self.output_off = self.output_off.saturating_add(1);
        }
        if let Some((key, pitch, player, correction)) = pitch {
            self.last_output_key = Some(key);
            self.last_output_pitch = pitch;
            self.last_output_player = (player * 100_000_000.0).round() as i64;
            self.last_output_correction = correction;
        }
    }
}
