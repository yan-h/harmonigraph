//! The tape's audio-thread half: it watches the transport, takes the Tape
//! tab's requests, and mixes what the input worker assembled.
//!
//! Everything here runs inside `process`, so nothing allocates or blocks:
//! fixed rings, atomics and plain state. The worker does the store lookups,
//! the seams and the fades at the edge of coverage (see `worker`).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use harmonigraph_record::transport;
use harmonigraph_ui::tape::TapeRequest;

use super::{Event, Piece, Reason, Shared, NO_REQUEST, STOP};
use crate::audio_ingress::{Heard, TapePlay};

/// How long a cue may wait for its first audio, or a playing one go without,
/// before it gives up: so a stalled or missing worker cannot hold the tape,
/// and with it `KeepAlive`, open for good.
const STARVED_SECONDS: f64 = 0.5;

/// One block as the deck needs it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockIn {
    pub position: Option<f64>,
    pub playing: bool,
    pub frames: usize,
    pub rate: f64,
    /// The analysis input is Main and the host is not rendering offline.
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockOut {
    /// The block's song position, for the analyzer ring's descriptor, while
    /// the transport rolls; `None` otherwise.
    pub song: Option<f64>,
    /// The tape is cued or sounding, so the host must keep calling.
    pub sounding: bool,
    /// What the tape did in this block, for the analyzer ring's descriptor.
    pub play: TapePlay,
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

pub(crate) struct Deck {
    shared: Arc<Shared>,
    samples: rtrb::Consumer<f32>,
    pieces: rtrb::Consumer<Piece>,
    events: rtrb::Producer<Event>,
    /// The previous block's position and duration, for [`transport::step`].
    last: Option<(f64, f64)>,
    /// A cue waiting for the one it replaces to finish fading.
    pending: Option<f64>,
    play: Play,
    /// The cue's identity, odd while active. Pieces carry the serial they were
    /// assembled for; any other is stale and skipped.
    serial: u64,
    cue_position: f64,
    /// Song seconds of the cue heard so far.
    played: f64,
    /// Song frames per frame heard, from the cue's pieces.
    speed: f64,
    /// The piece being mixed and the frames of it left.
    piece: Option<(Piece, usize)>,
    /// The previous block carried tape.
    mixed: bool,
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
            pending: None,
            play: Play::Idle,
            serial: 0,
            cue_position: 0.0,
            played: 0.0,
            speed: 1.0,
            piece: None,
            mixed: false,
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
        // A click from before the reset is not the host's to play now.
        self.shared.request.store(NO_REQUEST, Ordering::Release);
        self.last = None;
        self.pending = None;
        self.mixed = false;
    }

    /// End the cue now, without a fade.
    fn end(&mut self, reason: Reason) {
        if self.play == Play::Idle {
            return;
        }
        let ended = Event::Ended { position: self.cue_position, played: self.played, reason };
        self.play = Play::Idle;
        // The rest of a piece cut short is still in the ring, with its
        // descriptor already popped. Left there it would open the next cue,
        // and put every piece after it out of step with its samples.
        if let Some((piece, left)) = self.piece.take() {
            let count = left * piece.channels;
            if count > 0 {
                self.samples.read_chunk(count).expect("described samples").commit_all();
            }
        }
        // Even: inactive. Whatever the worker still sends for the old serial
        // is skipped as stale.
        self.serial = (self.serial | 1) + 1;
        self.shared.cue.store(self.serial, Ordering::Release);
        self.emit(ended);
    }

    /// Ramp down over the fade, then end. Tells the worker to stop now.
    fn fade(&mut self, reason: Reason, rate: f64) {
        match &mut self.play {
            Play::Idle => {}
            // Already going: the newest reason is the one that keeps it down.
            Play::Fading { reason: why, .. } => *why = reason,
            Play::Cued { .. } => self.end(reason),
            Play::Playing { .. } => {
                let of = super::store::fade_frames(rate);
                self.play = Play::Fading { left: of, of, reason };
                // Same serial, inactive: the worker stops assembling, and the
                // pieces it already sent still play out through the fade.
                self.shared.cue.store(self.serial & !1, Ordering::Release);
            }
        }
    }

    fn cue(&mut self, position: f64) {
        self.end(Reason::Cue);
        self.serial = (self.serial | 1) + 2;
        self.cue_position = position;
        self.played = 0.0;
        self.play = Play::Cued { waited: 0.0 };
        self.shared.cue_position.store(position.to_bits(), Ordering::Relaxed);
        self.shared.cue.store(self.serial, Ordering::Release);
        self.emit(Event::Started { position });
    }

    /// Tape mode's newest request since the last block, if any.
    fn take_request(&self) -> Option<TapeRequest> {
        match self.shared.request.swap(NO_REQUEST, Ordering::AcqRel) {
            NO_REQUEST => None,
            STOP => Some(TapeRequest::Stop),
            bits => Some(TapeRequest::Cue(f64::from_bits(bits))),
        }
    }

    /// One block: observe the transport, take a request, then mix into
    /// `planes` if the tape sounds. With nothing to mix, `planes` is not
    /// touched at all, so the output stays bit-identical to the input.
    pub(crate) fn process(&mut self, block: BlockIn, planes: &mut [&mut [f32]]) -> BlockOut {
        let duration = block.frames as f64 / block.rate;
        // Taken whatever the transport does, so a request the deck cannot
        // honour now is dropped rather than replayed when it can.
        let request = self.take_request();
        self.shared.enabled.store(block.enabled, Ordering::Relaxed);
        let Some(position) = block.position else {
            // No song position, no tape: nothing to key a recording on, and
            // nothing to say where a cue would play from.
            self.end(Reason::NoPosition);
            self.pending = None;
            self.last = None;
            self.mixed = false;
            self.publish(None, false);
            return BlockOut { song: None, sounding: false, play: TapePlay::Idle };
        };
        let rolling = transport::step(self.last, position, block.playing).rolling();
        self.watch_for_leak(position, rolling);
        if rolling {
            self.pending = None;
            // Cut, not fade: a fade would be heard in, and recorded into, the
            // playback that has just started.
            self.end(Reason::Rolling);
        } else if !block.enabled {
            self.pending = None;
            self.fade(Reason::Off, block.rate);
        } else {
            match request {
                Some(TapeRequest::Cue(at)) => {
                    self.pending = Some(at);
                    self.fade(Reason::Cue, block.rate);
                }
                Some(TapeRequest::Stop) => {
                    self.pending = None;
                    self.fade(Reason::Stop, block.rate);
                }
                None => {}
            }
            // Not while the cue being replaced is still fading: a block
            // shorter than the fade would cut it off partway down.
            if !matches!(self.play, Play::Fading { .. }) {
                if let Some(at) = self.pending.take() {
                    self.cue(at);
                }
            }
        }
        self.last = Some((position, duration));

        let from = self.cue_position + self.played;
        // Before the mix, which bumps the serial when the cue ends in it.
        let cue = self.serial;
        let frames = self.mix(planes, block.frames, duration);
        self.mixed = frames > 0;
        self.publish(Some(position), rolling);
        let play = if frames > 0 {
            TapePlay::Heard(Heard { cue, seconds: from, frames, speed: self.speed })
        } else if self.sounding() {
            TapePlay::Waiting
        } else {
            TapePlay::Idle
        };
        BlockOut { song: rolling.then_some(position), sounding: self.sounding(), play }
    }

    /// What Tape mode draws: the transport, and where the audition is.
    fn publish(&self, position: Option<f64>, rolling: bool) {
        let heard = match self.play {
            Play::Playing { .. } | Play::Fading { .. } => self.cue_position + self.played,
            Play::Idle | Play::Cued { .. } => f64::NAN,
        };
        let shared = &self.shared;
        shared.position.store(position.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
        shared.rolling.store(rolling, Ordering::Relaxed);
        shared.heard.store(heard.to_bits(), Ordering::Relaxed);
    }

    /// Mix whatever has been assembled for the current cue into `planes`;
    /// how many frames of tape were added, from the block's first.
    fn mix(&mut self, planes: &mut [&mut [f32]], frames: usize, duration: f64) -> usize {
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
                self.speed = piece.speed;
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
            self.played += n as f64 * piece.speed / f64::from(piece.rate);
            match self.play {
                Play::Cued { .. } => self.play = Play::Playing { starved: 0.0 },
                Play::Fading { left: 0, reason, .. } => {
                    self.end(reason);
                    return at;
                }
                _ => {}
            }
        }
        if !active {
            return 0;
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
        at
    }

    /// A sentinel for tape reaching what the host renders. Exports are kept
    /// clean by the reset that follows every activation and by `Offline`
    /// disabling the tape (Bitwig, #1721); this names a way around both on
    /// the console. Pure observation: nothing here changes what is played.
    fn watch_for_leak(&mut self, position: f64, rolling: bool) {
        // A block with tape in it, followed by playback continuing from that
        // block, means the tape block was playback's first.
        if rolling && self.mixed {
            let advanced = self.last.is_some_and(|(last, _)| position > last);
            self.emit(Event::TapeBeforeRolling { position, advanced });
        }
    }
}
