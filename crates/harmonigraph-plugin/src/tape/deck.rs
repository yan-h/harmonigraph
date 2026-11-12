//! The tape's audio-thread half: it watches the transport, decides when a
//! stopped click starts audition, and mixes what the input worker assembled.
//!
//! Everything here runs inside `process`, so nothing allocates or blocks:
//! fixed rings, atomics and plain state. The worker does the store lookups,
//! the seams and the fades at the edge of coverage (see `worker`).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use harmonigraph_record::transport;

use super::{Event, Piece, Reason, Shared};

/// How long after the transport stops a move of the playhead still counts as
/// Bitwig putting it back where playback began, not as a click. Whether the
/// return lands in the stop's own block is one of #1719's measurements; this
/// covers it arriving a few blocks later, and no click comes that fast.
pub(crate) const STOP_GRACE_SECONDS: f64 = 0.3;

/// How long a cue may wait for its first audio, or a playing one go without,
/// before it gives up: so a stalled or missing worker cannot hold the tape,
/// and with it `KeepAlive`, open for good.
const STARVED_SECONDS: f64 = 0.5;

/// A run of stopped moves with no pause longer than this is one gesture, a
/// drag, for the drag-rate measurement.
const GESTURE_GAP_SECONDS: f64 = 0.25;

/// How long after a stop the console waits for the playhead's return.
const RETURN_WATCH_SECONDS: f64 = 1.0;

/// One block as the deck needs it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockIn {
    pub position: Option<f64>,
    pub playing: bool,
    pub frames: usize,
    pub rate: f64,
    /// Audition is on, the analysis input is Main, and the host is not
    /// rendering offline.
    pub enabled: bool,
    /// The main input is all zeros, for the processing-while-stopped
    /// measurement.
    pub silent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockOut {
    /// The block's song position, for the analyzer ring's descriptor, while
    /// the transport rolls; `None` otherwise.
    pub song: Option<f64>,
    /// The tape is cued or sounding, so the host must keep calling.
    pub sounding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Play {
    Idle,
    /// Waiting for the first piece, for this many seconds so far.
    Cued {
        waited: f64,
    },
    Playing {
        starved: f64,
    },
    /// Ramping down over this many frames still to go, then ending.
    Fading {
        left: usize,
        of: usize,
        reason: Reason,
    },
}

/// After a stop: waiting for the playhead to move back.
#[derive(Clone, Copy, Debug)]
struct ReturnWatch {
    from: f64,
    blocks: u32,
    seconds: f64,
}

/// A run of stopped moves, for the drag-rate measurement.
#[derive(Clone, Copy, Debug)]
struct Gesture {
    count: u32,
    from: f64,
    to: f64,
    seconds: f64,
    quiet: f64,
}

pub(crate) struct Deck {
    shared: Arc<Shared>,
    samples: rtrb::Consumer<f32>,
    pieces: rtrb::Consumer<Piece>,
    events: rtrb::Producer<Event>,
    /// The previous block's position and duration, for [`transport::step`].
    last: Option<(f64, f64)>,
    was_rolling: bool,
    was_playing: bool,
    /// Seconds of stopped blocks since the transport last rolled.
    stopped_for: f64,
    /// A click waiting for the position to hold still for one block.
    pending: Option<f64>,
    play: Play,
    /// The cue's identity, odd while active. Pieces carry the serial they were
    /// assembled for; any other is stale and skipped.
    serial: u64,
    cue_position: f64,
    played: f64,
    /// The piece being mixed and the frames of it left.
    piece: Option<(Piece, usize)>,
    /// The previous block carried tape.
    mixed: bool,
    returning: Option<ReturnWatch>,
    gesture: Option<Gesture>,
    /// Seconds of consecutive all-zero input.
    quiet: f64,
    /// Positions of the first two blocks after a reset, for the export
    /// measurement: is the first block already moving?
    after_reset: Option<(u8, f64, bool)>,
}

impl Deck {
    pub(super) fn new(
        shared: Arc<Shared>,
        samples: rtrb::Consumer<f32>,
        pieces: rtrb::Consumer<Piece>,
        events: rtrb::Producer<Event>,
    ) -> Deck {
        Deck {
            shared,
            samples,
            pieces,
            events,
            last: None,
            was_rolling: false,
            was_playing: false,
            stopped_for: f64::INFINITY,
            pending: None,
            play: Play::Idle,
            serial: 0,
            cue_position: 0.0,
            played: 0.0,
            piece: None,
            mixed: false,
            returning: None,
            gesture: None,
            quiet: 0.0,
            after_reset: None,
        }
    }

    fn emit(&mut self, event: Event) {
        // A full ring loses a console line, never audio.
        let _ = self.events.push(event);
    }

    /// The tape is cued or sounding.
    pub(crate) fn sounding(&self) -> bool {
        self.play != Play::Idle
    }

    /// The plugin was reset: stop at once, and forget the transport.
    pub(crate) fn reset(&mut self) {
        let sounding = self.sounding();
        self.end(Reason::Reset);
        self.emit(Event::Reset { sounding });
        self.last = None;
        self.was_rolling = false;
        self.was_playing = false;
        self.stopped_for = f64::INFINITY;
        self.pending = None;
        self.mixed = false;
        self.returning = None;
        self.gesture = None;
        self.after_reset = Some((0, f64::NAN, false));
    }

    /// End the cue now, without a fade.
    fn end(&mut self, reason: Reason) {
        if self.play == Play::Idle {
            return;
        }
        let ended = Event::Ended { position: self.cue_position, played: self.played, reason };
        self.play = Play::Idle;
        self.piece = None;
        // Even: inactive. Whatever the worker still sends for the old serial
        // is skipped as stale.
        self.serial = (self.serial | 1) + 1;
        self.shared.cue.store(self.serial, Ordering::Release);
        self.emit(ended);
    }

    /// Ramp down over the fade, then end. Tells the worker to stop now.
    fn fade(&mut self, reason: Reason, rate: f64) {
        match self.play {
            Play::Idle | Play::Fading { .. } => {}
            Play::Cued { .. } => self.end(reason),
            Play::Playing { .. } => {
                let of = (super::store::FADE_SECONDS * rate).ceil().max(1.0) as usize;
                self.play = Play::Fading { left: of, of, reason };
                // Same serial, inactive: the worker stops assembling, and the
                // pieces it already sent still play out through the fade.
                self.shared.cue.store(self.serial & !1, Ordering::Release);
            }
        }
    }

    fn cue(&mut self, position: f64) {
        self.end(Reason::Click);
        self.serial = (self.serial | 1) + 2;
        self.cue_position = position;
        self.played = 0.0;
        self.play = Play::Cued { waited: 0.0 };
        self.shared.cue_position.store(position.to_bits(), Ordering::Relaxed);
        self.shared.cue.store(self.serial, Ordering::Release);
        self.emit(Event::Started { position });
    }

    /// One block: observe the transport, then mix into `planes` if the tape
    /// sounds. With nothing to mix, `planes` is not touched at all, so the
    /// output stays bit-identical to the input.
    pub(crate) fn process(&mut self, block: BlockIn, planes: &mut [&mut [f32]]) -> BlockOut {
        let duration = block.frames as f64 / block.rate;
        self.shared.blocks.fetch_add(1, Ordering::Relaxed);
        self.quiet = if block.silent { self.quiet + duration } else { 0.0 };
        self.shared.quiet.store(self.quiet.to_bits(), Ordering::Relaxed);
        let Some(position) = block.position else {
            // No song position, no tape: nothing to key a recording on, and
            // nothing to say where a click went.
            self.end(Reason::NoPosition);
            self.last = None;
            self.was_rolling = false;
            self.shared.stopped.store(false, Ordering::Relaxed);
            return BlockOut { song: None, sounding: false };
        };
        let step = transport::step(self.last, position, block.playing);
        let rolling = step.rolling();
        let moved = self.last.is_some_and(|(last, _)| position != last);
        self.measure(position, block, rolling, moved, duration, step);
        if rolling {
            self.pending = None;
            self.stopped_for = 0.0;
            // Cut, not fade: a fade would be heard in, and recorded into, the
            // playback that has just started.
            self.end(Reason::Rolling);
        } else {
            if self.was_rolling {
                self.stopped_for = 0.0;
            }
            let returning = self.stopped_for < STOP_GRACE_SECONDS;
            if !block.enabled {
                self.pending = None;
                self.fade(Reason::Off, block.rate);
            } else if moved && !self.was_rolling && !returning {
                self.pending = Some(position);
                self.fade(Reason::Click, block.rate);
            } else if !moved {
                if let Some(at) = self.pending.take() {
                    self.cue(at);
                }
            }
            self.stopped_for += duration;
        }
        self.last = Some((position, duration));
        self.was_rolling = rolling;
        self.was_playing = block.playing;
        self.shared.stopped.store(!rolling, Ordering::Relaxed);

        let mixed = self.mix(planes, block.frames, duration);
        self.mixed = mixed;
        BlockOut { song: rolling.then_some(position), sounding: self.sounding() }
    }

    /// Mix whatever has been assembled for the current cue into `planes`;
    /// whether any tape was added.
    fn mix(&mut self, planes: &mut [&mut [f32]], frames: usize, duration: f64) -> bool {
        let active =
            matches!(self.play, Play::Cued { .. } | Play::Playing { .. } | Play::Fading { .. });
        let mut at = 0;
        while at < frames || !active {
            if self.piece.is_none() {
                let Ok(piece) = self.pieces.pop() else { break };
                if !active || piece.serial != self.serial {
                    let count = piece.frames * piece.channels;
                    if count > 0 {
                        // Written before its descriptor, so all of it is there.
                        self.samples.read_chunk(count).expect("described samples").commit_all();
                    }
                    continue;
                }
                self.piece = Some((piece, piece.frames));
            }
            let (piece, left) = self.piece.expect("a piece");
            if left == 0 {
                self.piece = None;
                if piece.last {
                    self.end(Reason::End);
                    break;
                }
                continue;
            }
            let mut n = left.min(frames - at);
            if let Play::Fading { left: fading, .. } = self.play {
                // Stop at the end of the ramp: past it the tape is not
                // sounding, and adding a zero would still rewrite a -0.0.
                n = n.min(fading);
            }
            let channels = piece.channels;
            let chunk = self.samples.read_chunk(n * channels).expect("described samples");
            let (a, b) = chunk.as_slices();
            for frame in 0..n {
                let gain = match &mut self.play {
                    Play::Fading { left, of, .. } => {
                        let gain = *left as f32 / *of as f32;
                        *left = left.saturating_sub(1);
                        gain
                    }
                    _ => 1.0,
                };
                for (c, plane) in planes.iter_mut().enumerate() {
                    let i = frame * channels + c.min(channels - 1);
                    let sample = if i < a.len() { a[i] } else { b[i - a.len()] };
                    plane[at + frame] += sample * gain;
                }
            }
            chunk.commit_all();
            at += n;
            self.piece = Some((piece, left - n));
            self.played += n as f64 / f64::from(piece.rate);
            match self.play {
                Play::Cued { .. } => self.play = Play::Playing { starved: 0.0 },
                Play::Fading { left: 0, reason, .. } => {
                    self.end(reason);
                    return true;
                }
                _ => {}
            }
        }
        if !active {
            return false;
        }
        match &mut self.play {
            Play::Cued { waited } => {
                *waited += duration;
                if *waited > STARVED_SECONDS {
                    self.end(Reason::Starved);
                }
            }
            Play::Playing { starved } if at < frames => {
                *starved += duration * (frames - at) as f64 / frames as f64;
                if *starved > STARVED_SECONDS {
                    self.end(Reason::Starved);
                }
            }
            Play::Playing { starved } => *starved = 0.0,
            // Nothing left to fade through.
            Play::Fading { reason, .. } if at < frames => {
                let reason = *reason;
                self.end(reason);
            }
            _ => {}
        }
        at > 0
    }

    /// The measurements #1719 asks for before audition merges, as console
    /// events. Pure observation: nothing here changes what is played.
    fn measure(
        &mut self,
        position: f64,
        block: BlockIn,
        rolling: bool,
        moved: bool,
        duration: f64,
        step: transport::Step,
    ) {
        // Export: what the first blocks after a reset look like.
        if let Some((seen, first, first_playing)) = self.after_reset {
            if seen == 0 {
                self.after_reset = Some((1, position, block.playing));
            } else {
                self.after_reset = None;
                self.emit(Event::AfterReset {
                    first,
                    first_playing,
                    second: position,
                    second_playing: block.playing,
                    frames: block.frames as u32,
                });
            }
        }
        // Export: a block with tape in it, followed by playback continuing
        // from that block, means the tape block was playback's first.
        if rolling && self.mixed {
            let advanced = self.last.is_some_and(|(last, _)| position > last);
            self.emit(Event::TapeBeforeRolling { position, advanced });
        }
        // The playhead's return on stop: same block, later, or never.
        if self.was_playing && !block.playing {
            let from = self.last.map_or(position, |(last, _)| last);
            if matches!(step, transport::Step::Back { .. }) {
                self.emit(Event::Returned { from, to: position, blocks: 0, seconds: 0.0 });
            } else {
                self.returning = Some(ReturnWatch { from, blocks: 0, seconds: 0.0 });
            }
        } else if let Some(watch) = &mut self.returning {
            watch.blocks += 1;
            if moved {
                let watch = *watch;
                self.returning = None;
                self.emit(Event::Returned {
                    from: watch.from,
                    to: position,
                    blocks: watch.blocks,
                    seconds: watch.seconds,
                });
            } else if watch.seconds > RETURN_WATCH_SECONDS {
                let from = watch.from;
                self.returning = None;
                self.emit(Event::NoReturn { from, seconds: RETURN_WATCH_SECONDS });
            } else {
                watch.seconds += duration;
            }
        }
        // Drag rate: runs of stopped moves.
        if !rolling && !block.playing && moved {
            match &mut self.gesture {
                Some(g) => {
                    g.count += 1;
                    g.to = position;
                    g.seconds += g.quiet;
                    g.quiet = 0.0;
                }
                None => {
                    let from = self.last.map_or(position, |(last, _)| last);
                    self.gesture =
                        Some(Gesture { count: 1, from, to: position, seconds: 0.0, quiet: 0.0 });
                }
            }
        } else if let Some(g) = &mut self.gesture {
            g.quiet += duration;
            if rolling || g.quiet > GESTURE_GAP_SECONDS {
                let g = *g;
                self.gesture = None;
                self.emit(Event::Moves {
                    count: g.count,
                    from: g.from,
                    to: g.to,
                    seconds: g.seconds,
                });
            }
        }
    }
}
