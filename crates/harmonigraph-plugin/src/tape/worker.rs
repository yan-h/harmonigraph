//! The tape's input-worker half: it records rolling blocks into the store,
//! places notes and tuning beside them, assembles audio for a cue, and turns
//! the audio thread's events into console lines.
//!
//! It runs in the input worker's step with no lock held, between the step's
//! two holds of the editor lock; only the console lines need the lock, and
//! the step hands them over in its second hold.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harmonigraph_record::transport;
use harmonigraph_ui::params::ParamKey;

use crate::audio_ingress::Block;

use super::store::{Chunk, Stamp, Store};
use super::{Event, Mark, Piece, Shared, Tuning};

/// How far ahead of the audio thread the worker keeps a cue's audio: a dozen
/// of the shut window's 20 ms polls.
pub(crate) const LEAD_SECONDS: f64 = 0.25;

/// A pass's chunks are cut at about this length.
const CHUNK_SECONDS: f64 = 1.0;

/// Recent rolling blocks kept to place notes, which can arrive ahead of the
/// audio they sound in.
const RECENT: usize = 512;
/// Notes waiting for their block, and notes the live drain holds for a step.
pub(crate) const NOTES: usize = 4096;

/// How long without a process call before the console says the host stopped
/// calling.
const PAUSE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug)]
struct Pass {
    id: u64,
    epoch: u64,
    /// Song frame the pass started at.
    start: i64,
    /// Where the next block continues: source frame and song frame.
    next_source: u64,
    next_song: i64,
}

/// A rolling block's place on both clocks, to place notes.
#[derive(Clone, Copy, Debug)]
struct Placed {
    presentation: f64,
    seconds: f64,
    rate: f64,
    song: i64,
    pass: u64,
}

#[derive(Clone, Copy, Debug)]
struct Playback {
    serial: u64,
    start: i64,
    cursor: i64,
    done: bool,
}

pub(crate) struct Worker {
    shared: Arc<Shared>,
    /// Where the tuning is read from.
    params: Arc<crate::HarmonigraphParams>,
    samples: rtrb::Producer<f32>,
    pieces: rtrb::Producer<Piece>,
    events: rtrb::Consumer<Event>,
    store: Option<Store>,
    pass: Option<Pass>,
    next_pass: u64,
    /// The pass's chunk still being recorded; committed when it is full, the
    /// pass ends, or a dropped stretch breaks it.
    building: Option<Chunk>,
    recent: VecDeque<Placed>,
    pending: VecDeque<(f64, Mark)>,
    /// The presentation time the drained blocks reach.
    drained_through: f64,
    /// The song frame of the last rolling block, for a tuning change.
    last_song: Option<i64>,
    tuning: Option<Tuning>,
    playback: Option<Playback>,
    scratch: Vec<f32>,
    /// Swapped with the live drain's note buffer each step.
    pub(crate) inbox: Vec<(f64, Mark)>,
    /// Console lines for the step's second hold.
    pub(crate) logs: Vec<String>,
    blocks_seen: u64,
    blocks_at: Instant,
    paused: Option<Instant>,
    evicted_reported: bool,
}

/// Song seconds as the console prints them.
fn at(seconds: f64) -> String {
    format!("{seconds:.3} s")
}

impl Worker {
    pub(super) fn new(
        shared: Arc<Shared>,
        params: Arc<crate::HarmonigraphParams>,
        samples: rtrb::Producer<f32>,
        pieces: rtrb::Producer<Piece>,
        events: rtrb::Consumer<Event>,
    ) -> Worker {
        Worker {
            shared,
            params,
            samples,
            pieces,
            events,
            store: None,
            pass: None,
            next_pass: 0,
            building: None,
            recent: VecDeque::with_capacity(RECENT),
            pending: VecDeque::new(),
            drained_through: f64::NEG_INFINITY,
            last_song: None,
            tuning: None,
            playback: None,
            scratch: Vec::new(),
            inbox: Vec::with_capacity(NOTES),
            logs: Vec::new(),
            blocks_seen: 0,
            blocks_at: Instant::now(),
            paused: None,
            evicted_reported: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    /// One analyzer-ring block, in drain order.
    pub(crate) fn capture(&mut self, block: Block, samples: &[f32]) {
        let rate = block.format.sample_rate;
        let channels = block.format.channels;
        let presentation = block.origin + block.first_frame as f64 / f64::from(rate);
        self.drained_through = presentation + block.frames as f64 / f64::from(rate);
        let main = !block.format.sidechain && channels > 0;
        // Before the rolling test: a stopped block at a new rate must clear
        // the tape too, or a click before the transport next rolls would play
        // the old recording at the wrong speed.
        if main && self.store.as_ref().is_some_and(|s| s.rate != rate || s.channels != channels) {
            self.end_pass();
            self.store = None;
            self.recent.clear();
            self.logs
                .push(format!("tape: cleared for a new format ({channels} channels at {rate} Hz)"));
        }
        let song = match block.song {
            Some(song) if main => song,
            _ => return self.end_pass(),
        };
        let frame = (song * f64::from(rate)).round() as i64;
        // A rolling block this close to where the previous one left off
        // continues its pass: the jitter the shared rolling test allows.
        let tolerance = (transport::JITTER_SECONDS * f64::from(rate)) as i64;
        let continued = self.pass.and_then(|pass| {
            if pass.epoch != block.epoch || block.first_frame < pass.next_source {
                return None;
            }
            let expected = pass.next_song + (block.first_frame - pass.next_source) as i64;
            ((frame - expected).abs() <= tolerance).then_some(expected)
        });
        let song_frame = match continued {
            Some(expected) => expected,
            None => {
                self.end_pass();
                self.next_pass += 1;
                self.pass = Some(Pass {
                    id: self.next_pass,
                    epoch: block.epoch,
                    start: frame,
                    next_source: block.first_frame,
                    next_song: frame,
                });
                frame
            }
        };
        let store = self.store.get_or_insert_with(|| Store::new(rate, channels));
        let pass = self.pass.as_mut().expect("a pass");
        let full =
            |chunk: &Chunk| chunk.end - chunk.start >= (CHUNK_SECONDS * f64::from(rate)) as i64;
        if self.building.as_ref().is_some_and(|chunk| chunk.end != song_frame || full(chunk)) {
            let chunk = self.building.take().expect("a chunk");
            commit(store, chunk);
        }
        let chunk = self.building.get_or_insert_with(|| {
            let mut marks = Vec::new();
            if let Some(tuning) = &self.tuning {
                marks.push((song_frame, Mark::Tuning(Box::new(tuning.clone()))));
            }
            Chunk {
                start: song_frame,
                end: song_frame,
                stamp: Stamp { pass: pass.id, when: Instant::now(), pass_start: pass.start },
                data_start: song_frame,
                samples: Vec::with_capacity((CHUNK_SECONDS * f64::from(rate)) as usize * channels),
                marks,
            }
        });
        let frames = samples.len() / channels;
        chunk.samples.extend_from_slice(&samples[..frames * channels]);
        chunk.end += frames as i64;
        pass.next_source = block.first_frame + frames as u64;
        pass.next_song = song_frame + frames as i64;
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(Placed {
            presentation,
            seconds: frames as f64 / f64::from(rate),
            rate: f64::from(rate),
            song: song_frame,
            pass: pass.id,
        });
        self.last_song = Some(song_frame);
    }

    fn end_pass(&mut self) {
        self.pass = None;
        self.last_song = None;
        if let (Some(chunk), Some(store)) = (self.building.take(), self.store.as_mut()) {
            commit(store, chunk);
        }
    }

    /// Place a mark at song frame `frame` of pass `pass`.
    fn place(&mut self, frame: i64, pass: u64, mark: Mark) {
        match &mut self.building {
            Some(chunk)
                if chunk.stamp.pass == pass && (chunk.start..chunk.end).contains(&frame) =>
            {
                let at = chunk.marks.partition_point(|(t, _)| *t <= frame);
                chunk.marks.insert(at, (frame, mark));
            }
            _ => {
                if let Some(store) = &mut self.store {
                    store.mark(frame, pass, mark);
                }
            }
        }
    }

    /// The tuning the live display draws with now: the published
    /// configuration and the parameter values the take records.
    fn current_tuning(&self) -> Tuning {
        let params = &self.params;
        Tuning {
            resolved: params.configuration.get().map(|mailbox| {
                crate::configuration::view(mailbox.published.load(), false).resolved
            }),
            params: ParamKey::ALL.map(|key| params.param_for(key).value()),
        }
    }

    /// After a step's blocks: place the notes the live drain handed over, and
    /// record the tuning if it changed.
    pub(crate) fn settle(&mut self) {
        let tuning = self.current_tuning();
        self.pending.extend(self.inbox.drain(..));
        while self.pending.len() > NOTES {
            self.pending.pop_front();
        }
        let mut waiting = VecDeque::with_capacity(self.pending.len());
        while let Some((time, mark)) = self.pending.pop_front() {
            let placed = self
                .recent
                .iter()
                .rev()
                .find(|p| p.presentation <= time && time < p.presentation + p.seconds)
                .copied();
            match placed {
                Some(p) => {
                    let frame = p.song + ((time - p.presentation) * p.rate).round() as i64;
                    self.place(frame, p.pass, mark);
                }
                // Not rolling then: nothing of the tape's.
                None if time < self.drained_through => {}
                // Ahead of the audio: wait for its block.
                None => waiting.push_back((time, mark)),
            }
        }
        self.pending = waiting;
        if self.tuning.as_ref() != Some(&tuning) {
            if let (Some(frame), Some(pass)) = (self.last_song, self.pass) {
                self.place(frame, pass.id, Mark::Tuning(Box::new(tuning.clone())));
            }
            self.tuning = Some(tuning);
        }
        if let Some(store) = &self.store {
            if store.evicted > 0 && !self.evicted_reported {
                self.evicted_reported = true;
                self.logs.push(format!(
                    "tape: full at {} MiB; dropping the oldest-recorded audio from here on",
                    store.bytes() >> 20,
                ));
            }
        }
    }

    /// The current cue, read consistently: `(serial, song seconds)`.
    fn cue(&self) -> (u64, f64) {
        loop {
            let serial = self.shared.cue.load(Ordering::Acquire);
            let position = f64::from_bits(self.shared.cue_position.load(Ordering::Relaxed));
            if self.shared.cue.load(Ordering::Acquire) == serial {
                return (serial, position);
            }
        }
    }

    /// Keep the playback ring [`LEAD_SECONDS`] ahead for the current cue.
    pub(crate) fn play(&mut self) {
        let (serial, position) = self.cue();
        if serial & 1 == 0 {
            self.playback = None;
            return;
        }
        if self.playback.is_none_or(|p| p.serial != serial) {
            let start = self
                .store
                .as_ref()
                .map_or(0, |store| (position * f64::from(store.rate)).round() as i64);
            self.playback = Some(Playback { serial, start, cursor: start, done: false });
        }
        let mut playback = self.playback.expect("a playback");
        if playback.done || self.pieces.slots() == 0 {
            return;
        }
        let Some(store) = &self.store else {
            playback.done = true;
            self.playback = Some(playback);
            let _ =
                self.pieces.push(Piece { serial, frames: 0, channels: 1, rate: 1.0, last: true });
            return;
        };
        let channels = store.channels;
        let lead = (LEAD_SECONDS * f64::from(store.rate)) as usize;
        let queued = (self.samples.buffer().capacity() - self.samples.slots()) / channels;
        if queued >= lead {
            return;
        }
        let want = (lead - queued).min(self.samples.slots() / channels);
        self.scratch.clear();
        self.scratch.resize(want * channels, 0.0);
        let rendered = store.render(playback.cursor, &mut self.scratch);
        let fade = super::store::fade_frames(f64::from(store.rate)) as i64;
        for f in 0..rendered.frames {
            let k = playback.cursor + f as i64 - playback.start;
            if k >= fade {
                break;
            }
            let gain = (k + 1) as f32 / fade as f32;
            self.scratch[f * channels..(f + 1) * channels].iter_mut().for_each(|s| *s *= gain);
        }
        let count = rendered.frames * channels;
        if count > 0 {
            self.samples
                .write_chunk_uninit(count)
                .expect("sized by the free slots")
                .fill_from_iter(self.scratch[..count].iter().copied());
        }
        // Samples first: the audio thread reads a piece only once described.
        let _ = self.pieces.push(Piece {
            serial,
            frames: rendered.frames,
            channels,
            rate: store.rate,
            last: rendered.ended,
        });
        playback.cursor += rendered.frames as i64;
        playback.done = rendered.ended;
        self.playback = Some(playback);
    }

    /// The audio thread's events, and the host's process calls, as console
    /// lines.
    pub(crate) fn report(&mut self, now: Instant) {
        while let Ok(event) = self.events.pop() {
            let line = self.describe(event);
            self.logs.push(line);
        }
        let blocks = self.shared.blocks.load(Ordering::Relaxed);
        if blocks != self.blocks_seen {
            if let Some(since) = self.paused.take() {
                self.logs.push(format!(
                    "tape: host resumed processing after {:.1} s without a call",
                    now.duration_since(since).as_secs_f64(),
                ));
            }
            self.blocks_seen = blocks;
            self.blocks_at = now;
        } else if blocks > 0 && self.paused.is_none() && now.duration_since(self.blocks_at) > PAUSE
        {
            self.paused = Some(self.blocks_at);
            let stopped = self.shared.stopped.load(Ordering::Relaxed);
            let quiet = f64::from_bits(self.shared.quiet.load(Ordering::Relaxed));
            self.logs.push(format!(
                "tape: host stopped processing (transport {}, input silent for {quiet:.1} s before \
                 the last call)",
                if stopped { "stopped" } else { "rolling" },
            ));
        }
    }

    fn recording_at(&self, position: f64) -> String {
        let Some(store) = &self.store else { return "nothing recorded yet".into() };
        let frame = (position * f64::from(store.rate)).round() as i64;
        match store.stamp_at(frame) {
            None => "nothing recorded there".into(),
            Some(stamp) => format!(
                "{}, run-up {:.1} s, recorded {:.0} s ago",
                if stamp.is_from_start(store.rate) {
                    "warm (from the song's start)"
                } else if stamp.warm(frame, store.rate) {
                    "warm"
                } else {
                    "cold"
                },
                stamp.run_up(frame, store.rate),
                stamp.when.elapsed().as_secs_f64(),
            ),
        }
    }

    fn describe(&self, event: Event) -> String {
        match event {
            Event::Started { position } => {
                format!("tape: audition from {} ({})", at(position), self.recording_at(position))
            }
            Event::Ended { position, played, reason } => format!(
                "tape: audition from {} stopped after {played:.2} s by {}",
                at(position),
                reason.describe(),
            ),
            Event::Reset { sounding } => format!(
                "tape: plugin reset{}",
                if sounding { " while the tape sounded; it stopped" } else { "" },
            ),
            Event::AfterReset { first, first_playing, second, second_playing, frames } => format!(
                "tape: after the reset, block 1 at {} (playing {first_playing}), block 2 at {} \
                 (playing {second_playing}), {frames} frames: the first block {}",
                at(first),
                at(second),
                if second > first { "is already moving" } else { "is parked" },
            ),
            Event::TapeBeforeRolling { position, advanced: true } => format!(
                "tape: LEAK? rolling began at {} continuing a block that carried tape: one block of \
                 tape may be in what the host is rendering or recording",
                at(position),
            ),
            Event::TapeBeforeRolling { position, advanced: false } => format!(
                "tape: rolling began at {} from the parked playhead; the block before carried tape, \
                 playback did not",
                at(position),
            ),
            Event::Returned { from, to, blocks: 0, .. } => format!(
                "tape: transport stopped at {}; the playhead returned to {} in the same block",
                at(from),
                at(to),
            ),
            Event::Returned { from, to, blocks, seconds } => format!(
                "tape: transport stopped at {}; the playhead returned to {} {blocks} blocks \
                 ({:.0} ms) later{}",
                at(from),
                at(to),
                seconds * 1000.0,
                if seconds >= super::deck::STOP_GRACE_SECONDS {
                    " — past the grace period, so it would start the tape"
                } else {
                    ", inside the grace period"
                },
            ),
            Event::NoReturn { from, seconds } => format!(
                "tape: transport stopped at {}; the playhead did not move within {seconds:.0} s",
                at(from),
            ),
            Event::Moves { count: 1, from, to, .. } => {
                format!("tape: playhead moved {} → {}", at(from), at(to))
            }
            Event::Moves { count, from, to, seconds } => format!(
                "tape: playhead dragged {} → {}: {count} position updates over {:.0} ms, one per \
                 {:.0} ms",
                at(from),
                at(to),
                seconds * 1000.0,
                seconds * 1000.0 / f64::from(count - 1).max(1.0),
            ),
        }
    }
}

fn commit(store: &mut Store, mut chunk: Chunk) {
    if chunk.end > chunk.start {
        // A chunk closes on the block that takes it past its reserved second,
        // which doubles the buffer: give back the half that is never filled.
        chunk.samples.shrink_to_fit();
        store.insert(chunk);
    }
}
