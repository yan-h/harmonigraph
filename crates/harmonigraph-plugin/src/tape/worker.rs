//! The tape's input-worker half: it records rolling blocks into the store,
//! assembles audio for a cue, and turns the audio thread's events into
//! console lines.
//!
//! It runs in the input worker's step with no lock held, between the step's
//! two holds of the editor lock. The Tape tab's view and request are swapped
//! in the first hold, and the console lines handed over in the second.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use harmonigraph_record::transport;
use harmonigraph_ui::tape::{Meter, TapeRequest, TapeSpan, TapeView};

use crate::audio_ingress::Block;

use super::store::{Chunk, Stamp, Store};
use super::{Event, Piece, Shared, STOP};

/// How far ahead of the audio thread the worker keeps a cue's audio: a dozen
/// of the shut window's 20 ms polls.
pub(crate) const LEAD_SECONDS: f64 = 0.25;

/// A pass's chunks are cut at about this length.
const CHUNK_SECONDS: f64 = 1.0;

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

#[derive(Clone, Copy, Debug)]
struct Playback {
    serial: u64,
    start: i64,
    cursor: i64,
    done: bool,
}

pub(crate) struct Worker {
    shared: Arc<Shared>,
    samples: rtrb::Producer<f32>,
    pieces: rtrb::Producer<Piece>,
    events: rtrb::Consumer<Event>,
    store: Option<Store>,
    pass: Option<Pass>,
    next_pass: u64,
    /// The pass's chunk still being recorded; committed when it is full, the
    /// pass ends, or a dropped stretch breaks it.
    building: Option<Chunk>,
    /// The host's meter where the chunk being recorded starts.
    building_meter: Option<Meter>,
    /// The host's meter at each chunk's start, by song frame, for the Tape
    /// tab's ruler. Kept apart from the chunks because it describes the song,
    /// not a recording: a pass replaces the anchors where it plays, whether or
    /// not its audio wins there, and only over what it recorded.
    meters: BTreeMap<i64, Meter>,
    /// Bumped whenever the store or the meters change, so the Tape tab's
    /// spans are rebuilt only then; and the value they were last built at.
    generation: u64,
    shown: Option<u64>,
    playback: Option<Playback>,
    scratch: Vec<f32>,
    /// Console lines for the step's second hold.
    pub(crate) logs: Vec<String>,
    evicted_reported: bool,
}

/// Song seconds as the console prints them.
fn at(seconds: f64) -> String {
    format!("{seconds:.3} s")
}

impl Worker {
    pub(super) fn new(
        shared: Arc<Shared>,
        samples: rtrb::Producer<f32>,
        pieces: rtrb::Producer<Piece>,
        events: rtrb::Consumer<Event>,
    ) -> Worker {
        Worker {
            shared,
            samples,
            pieces,
            events,
            store: None,
            pass: None,
            next_pass: 0,
            building: None,
            building_meter: None,
            meters: BTreeMap::new(),
            generation: 0,
            shown: None,
            playback: None,
            scratch: Vec::new(),
            logs: Vec::new(),
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
        let main = !block.format.sidechain && channels > 0;
        // Before the rolling test: a stopped block at a new rate must clear
        // the tape too, or a cue before the transport next rolls would play
        // the old recording at the wrong speed.
        if main && self.store.as_ref().is_some_and(|s| s.rate != rate || s.channels != channels) {
            self.end_pass();
            self.store = None;
            self.meters.clear();
            self.generation += 1;
            self.logs
                .push(format!("tape: cleared for a new format ({channels} channels at {rate} Hz)"));
        }
        let (song, meter) = match block.song {
            Some(song) if main => (song.seconds, song.meter),
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
        self.store.get_or_insert_with(|| Store::new(rate, channels));
        let full =
            |chunk: &Chunk| chunk.end - chunk.start >= (CHUNK_SECONDS * f64::from(rate)) as i64;
        if self.building.as_ref().is_some_and(|chunk| chunk.end != song_frame || full(chunk)) {
            self.close_chunk();
        }
        if self.building.is_none() {
            self.building_meter = meter;
        }
        let pass = self.pass.as_mut().expect("a pass");
        let chunk = self.building.get_or_insert_with(|| Chunk {
            start: song_frame,
            end: song_frame,
            stamp: Stamp { pass: pass.id, when: Instant::now(), pass_start: pass.start },
            data_start: song_frame,
            samples: Vec::with_capacity((CHUNK_SECONDS * f64::from(rate)) as usize * channels),
        });
        let frames = samples.len() / channels;
        chunk.samples.extend_from_slice(&samples[..frames * channels]);
        chunk.end += frames as i64;
        pass.next_source = block.first_frame + frames as u64;
        pass.next_song = song_frame + frames as i64;
    }

    fn end_pass(&mut self) {
        self.pass = None;
        self.close_chunk();
    }

    /// Commit the chunk being recorded, and set its meter over the stretch it
    /// covers.
    fn close_chunk(&mut self) {
        let meter = self.building_meter.take();
        let Some(chunk) = self.building.take() else { return };
        if chunk.end > chunk.start {
            let stale: Vec<i64> =
                self.meters.range(chunk.start..chunk.end).map(|(&at, _)| at).collect();
            for at in stale {
                self.meters.remove(&at);
            }
            if let Some(meter) = meter {
                self.meters.insert(chunk.start, meter);
            }
        }
        if let Some(store) = &mut self.store {
            commit(store, chunk);
        }
        self.generation += 1;
    }

    /// Pass a Tape tab request to the audio thread, replacing one it has not
    /// taken yet. A cue position that is not a number is no position.
    pub(crate) fn request(&mut self, request: TapeRequest) {
        let bits = match request {
            TapeRequest::Cue(at) if at.is_finite() => at.to_bits(),
            TapeRequest::Cue(_) => return,
            TapeRequest::Stop => STOP,
        };
        self.shared.request.store(bits, Ordering::Release);
    }

    /// Take the Tape tab's request, and show it the tape as it is now.
    pub(crate) fn exchange(&mut self, view: &mut TapeView) {
        if let Some(request) = view.request.take() {
            self.request(request);
        }
        let shared = &self.shared;
        let seconds = |bits: &std::sync::atomic::AtomicU64| {
            Some(f64::from_bits(bits.load(Ordering::Relaxed))).filter(|s| !s.is_nan())
        };
        view.present = true;
        view.enabled = shared.enabled.load(Ordering::Relaxed);
        view.rolling = shared.rolling.load(Ordering::Relaxed);
        view.position = seconds(&shared.position);
        view.heard = seconds(&shared.heard);
        view.bytes = self.store.as_ref().map_or(0, Store::bytes);
        if self.shown != Some(self.generation) {
            self.shown = Some(self.generation);
            view.spans = self.store.as_ref().map_or_else(Vec::new, spans);
            let rate = self.store.as_ref().map_or(1.0, |store| f64::from(store.rate));
            view.meters =
                self.meters.iter().map(|(&at, &meter)| (at as f64 / rate, meter)).collect();
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

    /// The audio thread's events as console lines.
    pub(crate) fn report(&mut self) {
        while let Ok(event) = self.events.pop() {
            let line = self.describe(event);
            self.logs.push(line);
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
        }
    }
}

/// What is recorded, as the Tape tab draws it: neighbouring chunks of one
/// pass and one warmth joined into a span.
fn spans(store: &Store) -> Vec<TapeSpan> {
    let rate = f64::from(store.rate);
    let mut spans: Vec<TapeSpan> = Vec::new();
    let mut last: Option<(u64, i64)> = None;
    for chunk in store.chunks() {
        let warm = chunk.stamp.warm(chunk.start, store.rate);
        let joins = last == Some((chunk.stamp.pass, chunk.start));
        match spans.last_mut() {
            Some(span) if joins && span.warm == warm => span.end = chunk.end as f64 / rate,
            _ => spans.push(TapeSpan {
                start: chunk.start as f64 / rate,
                end: chunk.end as f64 / rate,
                warm,
                recorded: chunk.stamp.when,
            }),
        }
        last = Some((chunk.stamp.pass, chunk.end));
    }
    spans
}

fn commit(store: &mut Store, mut chunk: Chunk) {
    if chunk.end > chunk.start {
        // A chunk closes on the block that takes it past its reserved second,
        // which doubles the buffer: give back the half that is never filled.
        chunk.samples.shrink_to_fit();
        store.insert(chunk);
    }
}
