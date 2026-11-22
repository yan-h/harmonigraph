//! The tape's input-worker half: it records rolling blocks into the store
//! and the notes the display showed beside them, assembles audio for a cue,
//! plays the recorded notes into the display as the cue sounds, and turns the
//! audio thread's events into console lines.
//!
//! It runs in the input worker's step with no lock held, between the step's
//! two holds of the editor lock. It takes Tape mode's request and fills its
//! view in the first hold, and hands its console lines over in the second.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use harmonigraph_core::NoteRoll;

use harmonigraph_record::transport;
use harmonigraph_ui::tape::spectrogram::TapeSpectrogram;
use harmonigraph_ui::tape::{Meter, Speed, TapeRequest, TapeSpan, TapeView};
use harmonigraph_ui::SpectrumConfig;

use crate::audio_ingress::{Block, TapePlay};
use crate::lattice_maps::MapSelection;

use super::notes::{Book, Mark, Player, NOTES};
use super::spectrum::{Spectrum, STEP_BUDGET};
use super::store::{Chunk, Rendered, Stamp, Store};
use super::stretch::Stretched;
use super::{Event, Piece, Shared, NO_REQUEST, STOP};

/// Recent rolling blocks kept to place notes, which can arrive ahead of the
/// audio they sound in: as many as one drain of the ring can bring, so a step
/// catching up after a stall still places its notes.
const RECENT: usize = crate::audio_ingress::DESCRIPTORS;

/// Passes recently opened, kept for a mark dated before a pass's start that
/// is drained after it opened.
const OPENED: usize = 16;

/// A rolling block's place on both clocks, to place notes.
#[derive(Clone, Copy, Debug)]
struct Placed {
    presentation: f64,
    seconds: f64,
    rate: f64,
    song: i64,
    pass: u64,
}

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

struct Playback {
    serial: u64,
    /// Frames sent so far, for the fade in.
    sent: i64,
    done: bool,
    /// Song frames per frame heard, fixed for the cue: a change of speed
    /// cues again ([`Worker::exchange`]).
    speed: f64,
    source: Source,
}

/// Where a cue's audio comes from.
enum Source {
    /// The store, from this song frame on.
    Direct(i64),
    /// The store through the stretcher, at any speed but full.
    Stretched(Box<Stretched>),
}

/// Previous view allocations displaced under [`EditorLock`](crate::editor::EditorLock)
/// so [`background::step`](crate::background) drops them after releasing it.
///
/// Only tests read its fields: the step holds it for its drop alone.
#[must_use = "dropped here, it frees the displaced view under the editor lock"]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DisplacedView {
    pub(crate) roll: Option<Arc<NoteRoll>>,
    pub(crate) spectrogram: Option<TapeSpectrogram>,
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
    /// pass turns warm, the pass ends, or a dropped stretch breaks it.
    building: Option<Chunk>,
    /// The host's meter where the chunk being recorded starts.
    building_meter: Option<Meter>,
    /// The host's meter at each chunk's start, by song frame, for the Tape
    /// tab's ruler. Kept apart from the chunks because it describes the song,
    /// not a recording: a pass replaces the anchors where it plays, whether or
    /// not its audio wins there, and its last rules on to the next anchor.
    meters: BTreeMap<i64, Meter>,
    /// Bumped whenever the store or the meters change, so Tape mode's spans
    /// are rebuilt only then; and the value they were last built at. While
    /// the transport rolls it moves once a chunk, about a second.
    generation: u64,
    shown: Option<u64>,
    /// Bumped when a mark lands where Tape mode's roll draws it; with
    /// `generation`, what the roll was last built from.
    notes_generation: u64,
    roll_shown: Option<(u64, u64)>,
    playback: Option<Playback>,
    /// The speed Tape mode asks for, which the next cue plays at.
    speed: Speed,
    scratch: Vec<f32>,
    spectrum: Spectrum,
    /// Console lines for the step's second hold.
    pub(crate) logs: Vec<String>,
    evicted_reported: bool,
    /// Where the lattice map selection is read from.
    params: Arc<crate::HarmonigraphParams>,
    /// The notes the live drain records, straight into here; [`Self::settle`]
    /// empties it every step.
    pub(crate) inbox: Vec<(f64, Mark)>,
    recent: VecDeque<Placed>,
    /// Marks waiting for the block they sound in.
    pending: VecDeque<(f64, Mark)>,
    /// The presentation time the drained blocks reach.
    drained_through: f64,
    /// Passes started, by presentation time and song frame, waiting to be
    /// opened with the voices held when they started.
    opening: VecDeque<(u64, f64, i64)>,
    /// The last passes opened, the same way: a mark dated before one's start
    /// and drained after it opened is what it opened without.
    opened: VecDeque<(u64, f64, i64)>,
    /// The song frame the last rolling block reached, for a change of map.
    last_song: Option<i64>,
    /// The map selected when last sampled.
    map: Option<MapSelection>,
    notes_dropped_reported: bool,
    book: Book,
    player: Player,
    /// Tape mode's roll was on screen at the last exchange, so worth
    /// building.
    visible: bool,
    /// A roll built between holds, for the next exchange to hand over; any
    /// displaced roll returns in [`DisplacedView`] to be freed off-lock.
    built_roll: Option<Arc<NoteRoll>>,
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
            speed: Speed::default(),
            scratch: Vec::new(),
            spectrum: Spectrum::default(),
            logs: Vec::new(),
            evicted_reported: false,
            params,
            inbox: Vec::with_capacity(NOTES),
            recent: VecDeque::with_capacity(RECENT),
            pending: VecDeque::new(),
            drained_through: f64::NEG_INFINITY,
            opening: VecDeque::new(),
            opened: VecDeque::with_capacity(OPENED),
            last_song: None,
            map: None,
            notes_dropped_reported: false,
            book: Book::default(),
            player: Player::default(),
            notes_generation: 0,
            roll_shown: None,
            visible: false,
            built_roll: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn passes(&self) -> &BTreeMap<u64, super::notes::PassNotes> {
        &self.book.passes
    }

    /// One analyzer-ring block, in drain order.
    pub(crate) fn capture(&mut self, block: Block, samples: &[f32]) {
        let rate = block.format.sample_rate;
        let channels = block.format.channels;
        let main = !block.format.sidechain && channels > 0;
        let presentation = block.presentation();
        self.drained_through = presentation + block.frames as f64 / f64::from(rate);
        // Before the rolling test: a stopped block at a new rate must clear
        // the tape too, or a cue before the transport next rolls would play
        // the old recording at the wrong speed.
        if main && self.store.as_ref().is_some_and(|s| s.rate != rate || s.channels != channels) {
            self.end_pass();
            self.store = None;
            self.spectrum.clear();
            self.meters.clear();
            self.book.passes.clear();
            self.recent.clear();
            self.opening.clear();
            self.opened.clear();
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
                self.opening.push_back((self.next_pass, presentation, frame));
                frame
            }
        };
        self.store.get_or_insert_with(|| Store::new(rate, channels));
        let full =
            |chunk: &Chunk| chunk.end - chunk.start >= (CHUNK_SECONDS * f64::from(rate)) as i64;
        // A chunk is wholly cold or wholly warm, to the block, because the
        // store and Tape mode judge one by its warmth at a single frame
        // (#1742): one that turns warm here closes, and its warm rest starts
        // the next.
        let settled = |chunk: &Chunk| {
            !chunk.stamp.warm(chunk.start, rate) && chunk.stamp.warm(song_frame, rate)
        };
        if self
            .building
            .as_ref()
            .is_some_and(|chunk| chunk.end != song_frame || full(chunk) || settled(chunk))
        {
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
        self.last_song = Some(song_frame + frames as i64 - 1);
    }

    fn end_pass(&mut self) {
        self.pass = None;
        self.last_song = None;
        self.close_chunk();
    }

    /// Open pass `pass` with the voices held now, and keep it a while for a
    /// mark that turns out to be older than its start.
    fn open(&mut self, (pass, start, frame): (u64, f64, i64), map: MapSelection) {
        self.book.open(pass, frame, map);
        self.map = Some(map);
        if self.opened.len() == OPENED {
            self.opened.pop_front();
        }
        self.opened.push_back((pass, start, frame));
    }

    /// After a step's blocks: place the notes the live drain handed over at
    /// the song frames they sound at, open the passes that started with the
    /// voices held then, and record the lattice map selected where it
    /// changed.
    pub(crate) fn settle(&mut self) {
        let map = crate::lattice_maps::selection(&self.params);
        // A full inbox is one the drain stopped recording into.
        let mut dropped = self.inbox.len() >= NOTES;
        self.pending.extend(self.inbox.drain(..));
        while self.pending.len() > NOTES {
            self.pending.pop_front();
            dropped = true;
        }
        if dropped && !std::mem::replace(&mut self.notes_dropped_reported, true) {
            self.logs.push(format!(
                "tape: over {NOTES} notes in one step; the rest are lost, so the tape may hold a \
                 note past its end"
            ));
        }
        let mut waiting = VecDeque::new();
        while let Some((time, mark)) = self.pending.pop_front() {
            // Ahead of the audio: wait for its block, and keep every mark
            // after it waiting too, so the voices stay in order.
            if time >= self.drained_through || !waiting.is_empty() {
                waiting.push_back((time, mark));
                continue;
            }
            while self.opening.front().is_some_and(|&(_, start, _)| start <= time) {
                let opening = self.opening.pop_front().expect("a pass to open");
                self.open(opening, map);
            }
            mark.apply(&mut self.book.held);
            // Older than a pass that has already opened: that pass opened
            // without it, so it takes it at its start. After every mark
            // already there, which is right because the Hub publishes in time
            // order: nothing at the pass's start can be older than this.
            for &(pass, start, frame) in &self.opened {
                if start > time {
                    self.book.passes.entry(pass).or_default().place(frame, mark);
                    self.notes_generation += 1;
                }
            }
            // Newest first, and no further back than the first block that
            // ended before the mark: a mark played while stopped would
            // otherwise compare against every block kept.
            let placed = self
                .recent
                .iter()
                .rev()
                .take_while(|p| time < p.presentation + p.seconds)
                .find(|p| p.presentation <= time)
                .copied();
            // Not rolling then: nothing of the tape's.
            if let Some(p) = placed {
                let frame = p.song + ((time - p.presentation) * p.rate).round() as i64;
                self.book.passes.entry(p.pass).or_default().place(frame, mark);
                // Tape mode's roll draws committed chunks only: a mark in the
                // chunk still being recorded changes it when that commits.
                let recording = self.building.as_ref().filter(|c| c.stamp.pass == p.pass);
                if recording.is_none_or(|chunk| frame < chunk.start) {
                    self.notes_generation += 1;
                }
            }
        }
        self.pending = waiting;
        while let Some(opening) = self.opening.pop_front() {
            self.open(opening, map);
        }
        if self.map != Some(map) {
            if let (Some(frame), Some(pass)) = (self.last_song, self.pass) {
                if let Some(notes) = self.book.passes.get_mut(&pass.id) {
                    notes.maps.push((frame, map));
                }
            }
            self.map = Some(map);
        }
    }

    /// Play the recorded notes of the stretch each block carried into the
    /// display's tracker, on the display clock, and say where the audition
    /// is and which lattice map was selected where it was recorded.
    pub(crate) fn feed(
        &mut self,
        taken: &crate::editor::input::Taken<'_>,
        runtime: &mut harmonigraph_ui::VisualRuntime,
    ) {
        let tracker = &mut runtime.tracker;
        for (block, clock) in taken.clocked_blocks() {
            match (block.tape, &self.store) {
                (TapePlay::Heard(heard), Some(store)) => {
                    self.player.heard(tracker, store, &self.book.passes, heard, clock)
                }
                // Cued, or starved for a block: the audition goes on, and
                // what it holds stays held.
                (TapePlay::Waiting, Some(_)) => {}
                _ => self.player.silent(tracker, clock, block.song.is_some()),
            }
        }
        runtime.tape.audition = self.player.audition;
        runtime.tape.map =
            self.player.map.map(|map| crate::lattice_maps::recorded(&self.params, map));
    }

    /// Commit the chunk being recorded, and set its meter over the stretch it
    /// covers in place of any anchors there.
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
            let changed = store.take_changed();
            self.spectrum.invalidate(store, &changed);
            // A pass none of whose audio survives has no notes to show.
            let live: BTreeSet<u64> = store.chunks().map(|c| c.stamp.pass).collect();
            let current = self.pass.map(|p| p.id);
            self.book.passes.retain(|id, _| live.contains(id) || Some(*id) == current);
        }
        self.generation += 1;
    }

    /// Pass a Tape mode request to the audio thread, replacing one it has not
    /// taken yet. A cue position that is not a number is no position.
    pub(crate) fn request(&mut self, request: TapeRequest) {
        let bits = match request {
            TapeRequest::Cue(at) if at.is_finite() => at.to_bits(),
            TapeRequest::Cue(_) => return,
            TapeRequest::Stop => STOP,
        };
        self.shared.request.store(bits, Ordering::Release);
    }

    /// Take Tape mode's request, and show it the tape as it is now, analyzed
    /// under the Analyzer's `settings`. `open`: the editor's window is.
    ///
    /// Returns any displaced allocations so the caller can drop them once the
    /// editor lock is released.
    pub(crate) fn exchange(
        &mut self,
        view: &mut TapeView,
        settings: &SpectrumConfig,
        open: bool,
    ) -> DisplacedView {
        if let Some(request) = view.request.take() {
            self.request(request);
        } else if view.speed != self.speed {
            // A speed is fixed for a cue, so one active now starts again at the
            // new speed: from the spot heard, or from its own spot while it still
            // waits for audio. Not while one fades, whose serial is inactive
            // already: a Stop is a Stop, and a click waiting out the fade cues
            // at the new speed anyway. Nor over a request the audio thread has
            // not taken yet, for the same reasons.
            let heard = f64::from_bits(self.shared.heard.load(Ordering::Relaxed));
            let (serial, position) = self.cue();
            let at = (serial & 1 == 1).then(|| if heard.is_nan() { position } else { heard });
            if let Some(at) = at.filter(|at| at.is_finite()) {
                let request = &self.shared.request;
                let (taken, ordering) = (Ordering::AcqRel, Ordering::Relaxed);
                let _ = request.compare_exchange(NO_REQUEST, at.to_bits(), taken, ordering);
            }
        }
        self.speed = view.speed;
        self.spectrum.configure(settings);
        let spectrogram = self.spectrum.sync(&mut view.spectrogram);
        let shared = &self.shared;
        let seconds = |bits: &std::sync::atomic::AtomicU64| {
            Some(f64::from_bits(bits.load(Ordering::Relaxed))).filter(|s| !s.is_nan())
        };
        view.present = true;
        view.enabled = shared.enabled.load(Ordering::Relaxed);
        view.rolling = shared.rolling.load(Ordering::Relaxed);
        view.position = seconds(&shared.position);
        view.heard = seconds(&shared.heard);
        view.bytes = self.store.as_ref().map_or(0, Store::bytes) + self.book.bytes();
        self.visible = view.showing && open && settings.show_roll;
        let roll = self.built_roll.take().map(|roll| std::mem::replace(&mut view.notes, roll));
        if self.shown != Some(self.generation) {
            self.shown = Some(self.generation);
            view.spans = self.store.as_ref().map_or_else(Vec::new, spans);
            let rate = self.store.as_ref().map_or(1.0, |store| f64::from(store.rate));
            view.meters =
                self.meters.iter().map(|(&at, &meter)| (at as f64 / rate, meter)).collect();
        }
        DisplacedView { roll, spectrogram }
    }

    /// Rebuild Tape mode's roll if the recording changed under it, with no
    /// lock held: it walks every note recorded.
    pub(crate) fn roll(&mut self) {
        let built = (self.generation, self.notes_generation);
        if !self.visible || self.roll_shown == Some(built) {
            return;
        }
        self.roll_shown = Some(built);
        let roll = self.store.as_ref().map_or_else(Default::default, |store| {
            super::notes::song_roll(store, &self.book.passes)
        });
        self.built_roll = Some(Arc::new(roll));
    }

    /// Analyze what the recording or a new analysis window changed, for a
    /// step's share of time.
    pub(crate) fn analyze(&mut self) {
        self.spectrum.analyze(self.store.as_ref(), STEP_BUDGET);
    }

    #[cfg(test)]
    pub(crate) fn analyze_all(&mut self) {
        self.spectrum.analyze(self.store.as_ref(), std::time::Duration::MAX);
        assert_eq!(self.spectrum.pending(), 0);
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
        if self.playback.as_ref().is_none_or(|p| p.serial != serial) {
            let speed = self.speed.factor();
            let source = match &self.store {
                Some(store) => {
                    let start = (position * f64::from(store.rate)).round() as i64;
                    // A cue where nothing is recorded ends at once either way.
                    if speed != 1.0 && store.holding(start).is_some() {
                        Source::Stretched(Box::new(Stretched::new(store, start, speed)))
                    } else {
                        Source::Direct(start)
                    }
                }
                None => Source::Direct(0),
            };
            let speed = if matches!(source, Source::Stretched(_)) { speed } else { 1.0 };
            self.playback = Some(Playback { serial, sent: 0, done: false, speed, source });
        }
        let playback = self.playback.as_mut().expect("a playback");
        if playback.done || self.pieces.slots() == 0 {
            return;
        }
        let Some(store) = &self.store else {
            playback.done = true;
            let _ = self.pieces.push(Piece {
                serial,
                frames: 0,
                channels: 1,
                rate: 1.0,
                speed: 1.0,
                last: true,
            });
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
        let rendered: Rendered = match &mut playback.source {
            Source::Stretched(stretched) => stretched.render(store, &mut self.scratch),
            Source::Direct(cursor) => {
                let rendered = store.render(*cursor, &mut self.scratch);
                *cursor += rendered.frames as i64;
                rendered
            }
        };
        let fade = super::store::fade_frames(f64::from(store.rate)) as i64;
        for f in 0..rendered.frames {
            let k = playback.sent + f as i64;
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
            speed: playback.speed,
            last: rendered.ended,
        });
        playback.sent += rendered.frames as i64;
        playback.done = rendered.ended;
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
            Event::TapeBeforeRolling { position } => format!(
                "tape: LEAK? rolling began at {} continuing a block that carried tape: one block of \
                 tape may be in what the host is rendering or recording",
                at(position),
            ),
        }
    }
}

/// What is recorded, as Tape mode draws it: neighbouring chunks of one
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
