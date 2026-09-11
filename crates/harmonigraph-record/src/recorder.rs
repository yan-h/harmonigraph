//! Audio-thread capture and GUI control share the recording protocol here.
//! The writer owns files and finalization; render jobs own subprocesses.
//! Keeping both endpoint structs here lets the writer construct their private
//! state without exposing fields or adding a second wiring API.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use harmonigraph_core::notes::{NoteEventKind, SourceId};
use harmonigraph_take::ParamKey;
use parking_lot::Mutex;

use crate::{configuration, publication};
use render_job::{spawn_render, Progress, RenderControl};

mod render_job;
mod writer;

pub use render_job::{default_renderer_path, RenderRequest};
pub use writer::channel;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use writer::testing;

use configuration::{RecordAddress, RecordFence, CALLBACK_ACTIVE};

/// Ring capacity. Sized for a fast offline render rather than for a
/// frame: even at 20x realtime a dense piece is only a few thousand
/// records a second, and the writer thread drains continuously.
const TAKE_RING_CAPACITY: usize = 1 << 16;

const CONFIGURATION_FAILURE: &str =
    "recording incomplete: configuration/audio ownership failed; no render started";

/// Capacity of the audio ring, in interleaved samples. Generous on
/// purpose: during an offline export audio arrives many times faster than
/// realtime, and unlike the spectrum's ring — which drops by design,
/// because a stalled meter must never stall audio — dropping here would
/// put a silent hole in the finished video.
const AUDIO_RING_CAPACITY: usize = 1 << 20;

/// A take's WAV is always stereo, whatever selected audio stream the host
/// carries. The spec handed to the writer, the reservation in
/// [`Recorder::audio`] and the interleaving in `process` all read this one
/// number: if any of them disagrees, the header describes frames the data does
/// not have, and nothing downstream checks.
pub const TAKE_CHANNELS: usize = 2;

/// How much of a block fits in an interleaved audio ring, in SAMPLES, rounded
/// DOWN to whole frames.
///
/// The rounding is the one thing here that must not be wrong. A tail dropped
/// mid-frame leaves the ring one sample out of phase, and every later frame
/// hands the left channel's data to the right for as long as the plugin runs —
/// silently, because nothing downstream can tell a phase slip from the signal.
/// Bounding by free space is what makes a near-full ring (a stalled or closed
/// consumer) drop the block's tail rather than stall the audio thread.
///
/// Shared by [`Recorder::audio`] and the plugin's own spectrum ring, which is
/// why it is public: one guard, so the two cannot disagree about what a whole
/// frame is.
///
/// The zero-channel arm is not a live path — both callers gate on a positive
/// channel count. It is handled so the function carries no precondition a
/// caller can violate, which is what makes it safe to reuse; it does not move
/// the division off the audio thread, and does not pretend to.
pub fn interleaved_reservation(free_slots: usize, samples: usize, channels: usize) -> usize {
    if channels == 0 {
        return 0;
    }
    let free = free_slots / channels * channels;
    (samples * channels).min(free)
}

/// One recorded thing, in a form the audio thread can push without
/// allocating. Converted to a `harmonigraph_take::Record` on the writer thread.
#[derive(Clone, Copy)]
pub enum Entry {
    Configuration(harmonigraph_take::ConfigurationRecord),
    ConfigurationAt {
        address: RecordAddress,
        config: harmonigraph_take::ConfigurationRecord,
    },
    ConfigurationPassComplete(RecordAddress),
    ConfigurationEpochComplete(u64),
    ProducerClosed(u64),
    /// Exactly this committed audio prefix belongs at this point in the stream.
    AudioSamples(usize),
    Note {
        t: f64,
        source: SourceId,
        channel: u8,
        note: u8,
        kind: NoteEventKind,
    },
    /// `key` is an index into [`ParamKey::ALL`] — an id string would mean
    /// allocating on the audio thread.
    Param {
        t: f64,
        key: usize,
        value: f32,
    },
    /// The take time of the first audio sample about to be written.
    /// Sent once per pass, before any audio, so the header can say where
    /// the WAV sits relative to the notes.
    AudioStart(f64),
    /// The transport jumped backwards: a loop wrapped, or the playhead
    /// was dragged. Everything after this belongs to a different pass
    /// through the song, so the writer starts a new file rather than
    /// interleaving two performances at the same song positions.
    NewPass,
}

/// What the writer thread needs to open a WAV beside the take.
#[derive(Clone, Copy)]
pub struct AudioSpec {
    pub sample_rate: f32,
    pub channels: u16,
}

enum Command {
    Start(u64, Box<harmonigraph_take::Header>, std::path::PathBuf, Option<AudioSpec>),
    /// Close the file, and — if asked — render it to video.
    Stop(u64, Option<Box<RenderRequest>>),
}

/// The user's home directory, or `.` when `HOME` is unset — the base for the
/// renderer and takes locations below.
fn home_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// The two halves of [`RenderTrigger::AtBar`]: the bar the GUI asked the take
/// to end at, and the latch the audio thread sets once the transport played
/// through it.
///
/// One struct rather than two atomics because they are one setting, and every
/// construction site of the [`Recorder`]/[`Control`] pair would otherwise carry
/// both and be able to carry one.
///
/// `bar` is `f64` bits in an `AtomicU64`, which is what makes this writable
/// from a GUI frame and readable from the audio thread without a lock. **Off is
/// NaN, not zero**: zero is a bar, and a bar can only be crossed from below, so
/// a zeroed "off" would be indistinguishable from the one value that is
/// silently unreachable.
///
/// [`RenderTrigger::AtBar`]: harmonigraph_take::RenderTrigger::AtBar
struct StopAtBar {
    bar: AtomicU64,
    hit: AtomicBool,
}

impl Default for StopAtBar {
    fn default() -> Self {
        StopAtBar { bar: AtomicU64::new(f64::NAN.to_bits()), hit: AtomicBool::new(false) }
    }
}

impl StopAtBar {
    /// The bar to stop at, or `None` when the trigger is off. A non-finite
    /// stored value reads as off, so a NaN that arrived any other way than
    /// through [`set`](Self::set) cannot arm a comparison that is false against
    /// everything.
    fn get(&self) -> Option<f64> {
        let bar = f64::from_bits(self.bar.load(Ordering::Relaxed));
        bar.is_finite().then_some(bar)
    }

    fn set(&self, bar: Option<f64>) {
        self.bar.store(bar.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
    }
}

/// The audio-thread half: push entries, gated by an atomic the GUI owns.
pub struct Recorder {
    /// Pins the writer independently from all GUI Control clones. A retired
    /// producer may still receive actual remote history after editor teardown.
    _writer_lifetime: Option<mpsc::Sender<Command>>,
    fence: Arc<RecordFence>,
    publication: publication::Publisher,
    /// The display's own copy of every canonical report, published from here
    /// rather than forwarded by the writer thread. #712 §4: the display must
    /// not inherit the writer's idle sleep, and the two lanes fail apart —
    /// a full display lane leaves the take intact and vice versa.
    display: publication::Publisher,
    /// A gap covers every source, not the one report that overflowed: the
    /// consumer clears its whole held set. So one lost report owes every
    /// source a fresh snapshot ON THE LANE THAT LOST IT, and these latches are
    /// how the Hub learns that. Per lane, because the other lane lost nothing
    /// and its consumer is still holding a set that is correct.
    outage: publication::Lanes<bool>,
    record_epoch: u64,
    record_pass: u32,
    closed_epoch: u64,
    last_configuration: Option<harmonigraph_core::configuration::ResolvedConfig>,
    producer: rtrb::Producer<Entry>,
    /// Interleaved input samples, when the take is recording audio too.
    audio: rtrb::Producer<f32>,
    /// Selected for the whole take; Stop cannot change an in-flight callback.
    with_audio: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    /// Last value written per parameter, so only changes are recorded.
    /// Reset to NaN on arm so the first block of a take always writes a
    /// full set — a take that inherited "no change since last time" would
    /// replay with default tuning.
    last_params: [f32; ParamKey::ALL.len()],
    was_armed: bool,
    /// Previous block start and duration in seconds, for continuity detection.
    last_position: Option<(f64, f64)>,
    /// Published for the GUI: is the transport actually moving?
    rolling: Arc<AtomicBool>,
    /// Whether this pass has already declared its audio start.
    audio_started: bool,
    /// Set by the GUI for every trigger whose take is over the moment the
    /// transport goes backwards — a loop wrapping under
    /// [`AtLoopEnd`](harmonigraph_take::RenderTrigger::AtLoopEnd), and the host
    /// returning the playhead under
    /// [`OnTransportStop`](harmonigraph_take::RenderTrigger::OnTransportStop).
    /// The take ends there rather than splitting into another pass.
    end_at_rewind: Arc<AtomicBool>,
    /// Notes that entered the current take, through either the plain-MIDI arm
    /// or addressed publication. The status line reads this independently of
    /// transport progress: audio-only takes and delayed notes also roll.
    captured: Arc<AtomicU64>,
    /// Published for the GUI: the transport went backwards and the take is done
    /// — the GUI reads this, stops, and renders the one pass.
    hit_rewind: Arc<AtomicBool>,
    /// The bar the GUI wants the take to end at, and the latch saying it did.
    /// See [`StopAtBar`] and [`Recorder::observe_bar`].
    stop_at_bar: Arc<StopAtBar>,
    /// Bar position of the previous block, for the crossing test in
    /// [`observe_bar`](Recorder::observe_bar). Separate from `last_position`
    /// because the two are different clocks and either can be absent: a host
    /// can report seconds with no beats timeline, and a stop bar must not fire
    /// off a stale bar the way a missing one would.
    last_bar: Option<f64>,
    /// Local latch: once the rewind has ended the take, record nothing more
    /// until re-armed, so nothing after it reaches the file.
    finished: bool,
    /// Whether the transport has actually rolled FORWARD since arming. Under
    /// `end_at_rewind` a backward jump only ends the take once this is set —
    /// otherwise the very first backward jump (the transport snapping to the
    /// loop/play start when you hit play) would end the take before it recorded
    /// a single block.
    advanced: bool,
    /// A backward jump arrived while the transport was stopped, so the take owes
    /// a split — applied at the next block that actually records, not here. See
    /// [`Recorder::observe_transport`].
    pending_split: bool,
    /// Whether this take has recorded a block yet, without which an owed split
    /// has nothing to split from. See [`Recorder::observe_transport`].
    rolled: Arc<AtomicBool>,
}

impl Recorder {
    /// What EACH lane can still take. There is deliberately no combined
    /// number: a full display ring must not gate a snapshot an otherwise
    /// healthy writer is waiting for, and the minimum of the two did exactly
    /// that (#712).
    pub fn publication_free(&self) -> publication::Lanes<usize> {
        publication::Lanes { take: self.publication.free(), display: self.display.free() }
    }
    /// Consume a real publication serial on both lanes. Each queues its own
    /// gap immediately, so an absent drainer cannot hide this missing cut.
    pub fn publication_lost(&mut self, time: f64, route: publication::Route) {
        self.publication.discarded(time, route);
        self.display.discarded(time, publication::Route::default());
        self.outage = publication::Lanes::both(true);
        self.publication_result(Err(publication::PublishError::Lost), route);
    }
    /// Read and clear the outage latches. A lane that reports one owes every
    /// source the caller knows about a fresh snapshot on that lane, because
    /// its gap cleared all of them there.
    pub fn take_publication_outage(&mut self) -> publication::Lanes<bool> {
        std::mem::take(&mut self.outage)
    }
    pub fn publish_clock(&self, time: f64) {
        self.publication.observe_clock(time);
        self.display.observe_clock(time);
    }
    pub fn enable_canonical(&self) {
        self.fence.canonical_enabled.store(true, Ordering::Release);
    }

    /// The same delta on both lanes, with an outcome for each. There is no
    /// combined `Result`: a lane that lost the report owes a snapshot on that
    /// lane alone, and the caller has to say which.
    pub fn publish_note(
        &mut self,
        note: harmonigraph_core::canonical::NoteDelta,
        route: publication::Route,
    ) -> publication::Lanes<Result<(), publication::PublishError>> {
        let take = self.publication.note(note, route);
        // Only the take lane's own outcome accounts the take. A display lane
        // that overflowed still returns Err on its own half, so the caller
        // arms a snapshot there, but it must not touch the file.
        self.publication_result(take, route);
        // A route with an address is a note landing in a pass: with a
        // configuration owner installed, this is the only way one gets there.
        // A reset is no note of the take's, whatever route it carries.
        if take.is_ok()
            && route.address.is_some()
            && !matches!(note.event.kind, NoteEventKind::SourceReset | NoteEventKind::SessionReset)
        {
            self.captured.fetch_add(1, Ordering::Relaxed);
        }
        let display = self.display.note(note, publication::Route::default());
        self.outage.take |= take == Err(publication::PublishError::Lost);
        self.outage.display |= display == Err(publication::PublishError::Lost);
        publication::Lanes { take, display }
    }

    /// One lane at a time, because a snapshot carries an identity and each
    /// lane's consumer deduplicates on it. Publishing the same `id` to both
    /// and advancing the cursor only when both accepted left the lane that DID
    /// accept rejecting the retry as a duplicate, with the source's voices
    /// missing for good (#712).
    pub fn publish_baseline(
        &mut self,
        lane: publication::Lane,
        baseline: &harmonigraph_core::canonical::SourceBaseline,
        route: publication::Route,
    ) -> Result<(), publication::PublishError> {
        match lane {
            publication::Lane::Take => {
                let result = self.publication.baseline(baseline, route);
                self.publication_result(result, route);
                result
            }
            // The display's copy addresses no file, so it carries no route.
            publication::Lane::Display => {
                self.display.baseline(baseline, publication::Route::default())
            }
        }
    }

    /// What a take-lane publication outcome costs the file.
    ///
    /// A hole in the note history is NOT a recording failure (#712): the take
    /// keeps capturing, finalises normally and exports with a warning, and the
    /// gap the lane queued on its reserved cell is what carries that warning
    /// into the file as an `IncompleteRecord`. Refusal is left to failures
    /// that are actually fatal to the recording — audio, ownership and I/O,
    /// which reach the fence through [`Recorder::fail_configuration`], the
    /// closures' own [`Recorder::source_pass_complete`] /
    /// [`Recorder::source_epoch_complete`], and the writer's paths — plus
    /// `Invalid` here, which is the one publication outcome with NO gap to
    /// describe it and so the one that would otherwise leave a silent hole.
    fn publication_result(
        &self,
        result: Result<(), publication::PublishError>,
        route: publication::Route,
    ) {
        match result {
            Ok(()) | Err(publication::PublishError::Busy) => {}
            Err(publication::PublishError::Lost) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(publication::PublishError::Invalid) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                if route.address.is_some() || self.fence.finishing.load(Ordering::Acquire) {
                    self.fence.fail();
                }
            }
        }
    }

    /// A complete audio-owned publication frontier, not a configuration, GUI,
    /// source-retention or disk acknowledgement, authorizes these closures.
    pub fn source_pass_complete(&mut self, address: RecordAddress, observation_time: f64) {
        if self.publication.pass_complete(address, observation_time).is_err() {
            self.fence.fail();
        }
    }
    pub fn source_epoch_complete(&mut self, epoch: u64, observation_time: f64) {
        if epoch != 0 && epoch > self.fence.source_closed.load(Ordering::Acquire) {
            if self.publication.epoch_complete(epoch, observation_time).is_err() {
                self.fence.fail();
            }
            self.fence.source_closed.store(epoch, Ordering::Release);
        }
    }
    /// Begin the callback's recording observation. Ordinary callers pair this
    /// with `finish_callback` after their final publication, even when disarmed.
    pub fn is_armed(&mut self) -> bool {
        self.is_armed_at(self.capture_recording_intent())
    }

    /// Release ordinary callback ownership after its final publication, or
    /// after joining callbacks. Stop can then close the prefix even when no
    /// further callback will run. Configuration/source closures stay separate.
    pub fn finish_callback(&mut self) {
        let intent =
            self.fence.intent.fetch_and(!CALLBACK_ACTIVE, Ordering::AcqRel) & !CALLBACK_ACTIVE;
        if intent & 1 == 0 {
            self.is_armed_at(intent);
        }
    }

    /// Use the arm/disarm intent captured at the enclosing callback boundary,
    /// so a concurrent stop cannot cut off that callback's remaining audio.
    pub fn is_armed_at(&mut self, intent: u64) -> bool {
        let armed = intent & 1 != 0;
        let epoch = intent >> 1;
        if armed && epoch != self.record_epoch {
            self.record_epoch = epoch;
            self.record_pass = 1;
        }
        if !armed && epoch > self.closed_epoch {
            if self.fence.enabled.load(Ordering::Acquire) {
                self.push(Entry::ProducerClosed(epoch));
            }
            #[cfg(feature = "test-support")]
            self.fence.producer_close_pause.reach();
            self.closed_epoch = epoch;
        }
        self.update_armed(armed)
    }

    fn update_armed(&mut self, armed: bool) -> bool {
        if armed && !self.was_armed {
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
            self.last_configuration = None;
            self.last_position = None;
            self.audio_started = false;
            self.finished = false;
            self.advanced = false;
            self.captured.store(0, Ordering::Relaxed);
            self.pending_split = false;
            self.rolled.store(false, Ordering::Relaxed);
            self.hit_rewind.store(false, Ordering::Relaxed);
            self.last_bar = None;
            self.stop_at_bar.hit.store(false, Ordering::Relaxed);
        }
        self.was_armed = armed;
        armed
    }

    fn push(&mut self, entry: Entry) {
        if self.producer.push(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            if self.fence.enabled.load(Ordering::Acquire)
                || matches!(entry, Entry::AudioSamples(_) | Entry::AudioStart(_) | Entry::NewPass)
            {
                self.fence.fail();
            }
        }
    }

    pub fn note(&mut self, t: f64, source: SourceId, channel: u8, note: u8, kind: NoteEventKind) {
        self.push(Entry::Note { t, source, channel, note, kind });
        self.captured.fetch_add(1, Ordering::Relaxed);
    }

    pub fn wants_audio(&self) -> bool {
        self.was_armed
            && self.with_audio.load(Ordering::Relaxed)
            && !self.fence.failed.load(Ordering::Acquire)
    }

    /// Declare where the audio about to be written sits in take time.
    /// Idempotent per pass; the first call is the one that counts.
    pub fn mark_audio_start(&mut self, t: f64) {
        if !self.audio_started {
            self.audio_started = true;
            self.push(Entry::AudioStart(t));
        }
    }

    /// Append one block of interleaved input samples.
    ///
    /// Reserves the whole block at once rather than pushing per sample:
    /// one ring-atomic touch per block instead of tens of thousands a
    /// second. A short reservation means the ring filled, which is a
    /// hole in the recording, so it is counted like any dropped record.
    ///
    /// Rounded down to whole FRAMES, for the reason `interleaved_reservation`
    /// gives — and it matters more here than on the spectrum's ring. This one
    /// lands in a WAV, so half a frame swaps that file's channels from there
    /// on, permanently, and the writer cannot notice: it derives its frame
    /// count by dividing the bytes it was handed. The callers happen to keep
    /// the free count even today (even capacity, even `samples`, and a drain
    /// that commits everything), but nothing declares that, and an odd
    /// capacity or a partial drain would end the argument silently.
    pub fn audio(&mut self, block: &mut dyn Iterator<Item = f32>, samples: usize) {
        if self.fence.failed.load(Ordering::Acquire) {
            return;
        }
        let room =
            interleaved_reservation(self.audio.slots(), samples / TAKE_CHANNELS, TAKE_CHANNELS);
        if room < samples {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.fence.fail();
        }
        if room == 0 {
            return;
        }
        let written = if let Ok(chunk) = self.audio.write_chunk_uninit(room) {
            chunk.fill_from_iter(block.take(room));
            true
        } else {
            false
        };
        if written {
            self.push(Entry::AudioSamples(room));
        }
    }

    /// Note where the transport is, and answer whether it is rolling —
    /// i.e. whether this block's events belong in the take. Called once
    /// per block with the block's song position, the host's own `playing`
    /// flag, and this block's frame count divided by its sample rate.
    ///
    /// A playing host records immediately. A stopped host records only when
    /// its position advances by the PREVIOUS callback's duration, within 50%
    /// either way (inclusive). This allows host timing/reporting variation and
    /// variable callback sizes without accepting arbitrary forward scrubs as
    /// offline export progress. The first stopped observation only seeds history;
    /// rejected jumps also update it, so an export can start at a new position.
    /// A tiny manual scrub in that interval is indistinguishable from export:
    /// it records one block and can qualify a subsequent rewind as the take's
    /// end. Repeated accepted tiny scrubs record one block each.
    ///
    /// Any stopped backward movement is a rewind, including the restore of a
    /// single accepted export block. Playing hosts retain the
    /// 50 ms backward jitter allowance used for loop detection.
    ///
    /// **What a backward jump costs is what the host does when an audio export
    /// finishes: it puts the playhead back, and that lands here as one backward
    /// block.** Under `end_at_rewind` that block ends the take, which is what
    /// the export wants — everything the transport does afterwards belongs to no
    /// take, and `Stop` renders the pass that holds the piece. Under OnDisarm,
    /// which has to keep recording across a loop, the take still splits, but a
    /// jump the host reports as NOT playing only OWES the split: it is paid at
    /// the next block that records, so a playhead put back and left alone opens
    /// no pass at all, and one played away from opens its pass with the block
    /// that fills it.
    pub fn observe_transport(&mut self, position: f64, playing: bool, duration: f64) -> bool {
        // Once a rewind has ended the take, record nothing more until a fresh
        // arm clears the latch.
        if self.finished {
            return false;
        }
        const BACKWARD_JUMP: f64 = 0.05;
        let rolling = match self.last_position {
            Some((last, _)) if position < last - if playing { BACKWARD_JUMP } else { 0.0 } => {
                // A backward jump means the transport looped back, snapped to
                // the loop/play start as playback began, or the playhead was
                // dragged.
                //
                // Under `end_at_rewind` a jump that comes AFTER the take has
                // rolled forward (`advanced`) IS the end of the take, so latch
                // done and tell the GUI to stop + render — WITHOUT splitting,
                // because these triggers want exactly one file and the jump is
                // its end. Keyed off the jump itself, not the host's loop range:
                // hosts (Bitwig included) don't flag the loop as active to the
                // plugin, so nih-plug's loop_range stays None. It is also what a
                // host does when an audio export finishes and it puts the
                // playhead back; everything the transport does after that
                // belongs to no take. The cost is that a manual rewind mid-take
                // also ends it, which is the bargain both triggers are.
                //
                // But a backward jump BEFORE any forward motion is just the
                // transport arriving at the loop/play start (the playhead was
                // parked past it). Ending there would finish the take with
                // nothing recorded — an empty file and a broken render. So
                // instead begin the pass here: no NewPass (these triggers only
                // ever want one file), no end.
                //
                if self.end_at_rewind.load(Ordering::Relaxed) {
                    if self.advanced {
                        self.finished = true;
                        self.hit_rewind.store(true, Ordering::Relaxed);
                        self.last_position = Some((position, duration));
                        self.rolling.store(false, Ordering::Relaxed);
                        return false;
                    }
                    // Still the same pass: never redeclare an audio origin or
                    // parameter baseline over samples already in this file.
                    // One file, so an owed split is dropped rather than
                    // carried into the pass beginning here.
                    self.pending_split = false;
                    playing
                } else if !playing {
                    // OnDisarm, and the playhead moved while the transport was
                    // stopped: note where it went and owe a split, but record
                    // nothing here.
                    self.pending_split = true;
                    self.last_position = Some((position, duration));
                    self.rolling.store(false, Ordering::Relaxed);
                    return false;
                } else {
                    self.pending_split = true;
                    true
                }
            }
            Some((last, previous_duration)) => {
                let step = position - last;
                let continuous = previous_duration.is_finite()
                    && previous_duration > 0.0
                    && position >= last + previous_duration * 0.5
                    && position <= last + previous_duration * 1.5;
                let rolling = playing || continuous;
                self.advanced |= rolling && step > 0.0;
                rolling
            }
            // Nothing to compare on the first block; the flag is all
            // there is.
            None => playing,
        };
        self.last_position = Some((position, duration));
        // An owed split lands on the first block that records again, ahead of
        // that block's own events — which belong to the new pass. A new file
        // starts empty, so every parameter must be written again or the new pass
        // replays with whatever the previous one happened to end on, and the
        // next pass's audio starts somewhere new.
        //
        // A trigger that wants one file drops the debt instead of paying it: the
        // split can only have been owed under OnDisarm, so a trigger chosen
        // since must not be handed a second pass — the take's notes would sit in
        // it while the first file is the one that renders. Clearing it here as
        // well as at a backward jump is what makes "one file" hold whichever way
        // the debt comes due.
        //
        // So does a take that has not recorded a block yet: its only pass is
        // empty, and the configuration side learns of a pass only from a block
        // routed to it. Paid here, the split left that empty pass waiting for a
        // close nothing would ever send, and Stop never finished — from the
        // ordinary way to begin a take: arm stopped, return to the start, play.
        if rolling
            && std::mem::take(&mut self.pending_split)
            && !self.end_at_rewind.load(Ordering::Relaxed)
            && self.rolled.load(Ordering::Relaxed)
        {
            if let Some(pass) = self.record_pass.checked_add(1) {
                self.record_pass = pass;
            } else {
                self.fence.fail();
            }
            self.push(Entry::NewPass);
            self.last_configuration = None;
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
            self.audio_started = false;
        }
        if rolling {
            self.rolled.store(true, Ordering::Relaxed);
        }
        self.rolling.store(rolling, Ordering::Relaxed);
        rolling
    }

    /// End the take if this block played THROUGH the stop bar, and answer
    /// whether it did. Called once per block with the transport's bar position,
    /// BEFORE [`observe_transport`](Self::observe_transport): a block that ends
    /// the take here contributes nothing, so the cut lands on a block boundary
    /// at or before the bar rather than a block after it, and latching
    /// `finished` first is also what stops `observe_transport` paying an owed
    /// split into a pass that this same block immediately finishes empty.
    ///
    /// The bar is counted from ZERO at the song's start, which is the base the
    /// transport reports; the arranger's 1-based number is converted on the way
    /// in by `RenderConfig::stop_at_bar`.
    ///
    /// **The test is a CROSSING, not a level.** A take armed with the playhead
    /// already past the bar never sees a position below it, so it never fires
    /// and simply runs until you disarm — the same graceful nothing
    /// [`AtLoopEnd`](harmonigraph_take::RenderTrigger::AtLoopEnd) does with
    /// looping off. A level test would instead end it on the first block, with
    /// an empty file and a render of nothing, which is #569's failure exactly.
    ///
    /// **A crossing also has to be a small step.** A playhead DRAGGED past the
    /// bar crosses it as surely as playback does, and the two are
    /// indistinguishable in this API: an offline export reports
    /// `playing = false` throughout while its position climbs, so the host's
    /// own flag cannot separate them (#569 eliminated that). What does separate
    /// them is size — playback advances by a block, a drag by seconds — so a
    /// crossing wider than `STEP_BARS` below is read as a drag and only moves
    /// `last_bar`. The residual case is a drag that lands within a bar of where
    /// it started and straddles the stop bar; it ends the take early, visibly
    /// (Record take switches off), and re-arming is the whole of the repair.
    pub fn observe_bar(&mut self, bar: Option<f64>) -> bool {
        if self.finished {
            return false;
        }
        /// The widest forward step in bars that still counts as playing rather
        /// than dragging. A block is milliseconds — a hundredth of a bar at any
        /// tempo a DAW offers — so this is three orders of magnitude of slack
        /// against the one thing it must not misread, an offline export whose
        /// blocks are large and whose transport reports itself stopped.
        const STEP_BARS: f64 = 1.0;
        let (Some(bar), Some(stop)) = (bar, self.stop_at_bar.get()) else {
            // Remember the bar even with the trigger off, so switching it on
            // mid-take compares against a real previous position rather than
            // against wherever the take started.
            self.last_bar = bar;
            return false;
        };
        let crossed =
            self.last_bar.is_some_and(|last| last < stop && stop <= bar && bar - last <= STEP_BARS);
        self.last_bar = Some(bar);
        if crossed {
            self.finished = true;
            self.stop_at_bar.hit.store(true, Ordering::Relaxed);
            self.rolling.store(false, Ordering::Relaxed);
        }
        crossed
    }

    pub fn enable_configuration(&self) {
        self.fence.enabled.store(true, Ordering::Release);
    }
    /// Capture a callback boundary. An ordinary producer must pair this with
    /// `finish_callback` after its last publication, even for a disarmed block.
    pub fn capture_recording_intent(&self) -> u64 {
        let intent = if self.fence.enabled.load(Ordering::Acquire) {
            self.fence.intent.load(Ordering::Acquire)
        } else {
            // A single RMW arbitrates with Stop: either this callback owns
            // its armed prefix, or it sees disarmed and can publish no audio.
            self.fence.intent.fetch_or(CALLBACK_ACTIVE, Ordering::AcqRel)
        } & !CALLBACK_ACTIVE;
        #[cfg(feature = "test-support")]
        self.fence.boundary_pause.reach();
        intent
    }
    pub fn recording_epoch(&self) -> u64 {
        self.fence.epoch()
    }
    pub fn configuration_address(&self) -> Option<RecordAddress> {
        (self.was_armed && !self.finished && self.record_epoch != 0)
            .then_some(RecordAddress { epoch: self.record_epoch, pass: self.record_pass })
    }
    pub fn fail_configuration(&self) {
        self.fence.fail();
    }
    /// Called after callback join, before retiring configuration can fail.
    /// Moving this Recorder preserves the unique publication owner's hold.
    pub fn hold_retired_publication(&self) {
        self.fence.retirement_hold.store(true, Ordering::Release);
    }
    /// Every joined source's immutable final actual cut has a publication
    /// payload or explicit loss disposition. This does not release note credit.
    pub fn retired_publication_complete(&mut self) {
        self.fence.retirement_hold.store(false, Ordering::Release);
        self._writer_lifetime = None;
    }
    pub fn configuration_at(
        &mut self,
        address: RecordAddress,
        t: f64,
        resolved: harmonigraph_core::configuration::ResolvedConfig,
    ) {
        self.push(Entry::ConfigurationAt {
            address,
            config: harmonigraph_take::ConfigurationRecord::new(t, resolved),
        });
    }
    pub fn configuration_pass_complete(&mut self, address: RecordAddress) {
        self.push(Entry::ConfigurationPassComplete(address));
    }
    pub fn configuration_epoch_complete(&mut self, epoch: u64) {
        if epoch != 0 && epoch > self.fence.configuration_closed.load(Ordering::Acquire) {
            self.push(Entry::ConfigurationEpochComplete(epoch));
            self.fence.configuration_closed.store(epoch, Ordering::Release);
        }
    }

    /// Preserve each effective resolved boundary, independently of UI cadence.
    pub fn configuration(
        &mut self,
        t: f64,
        resolved: harmonigraph_core::configuration::ResolvedConfig,
    ) {
        if self.last_configuration != Some(resolved) {
            self.push(Entry::Configuration(harmonigraph_take::ConfigurationRecord::new(
                t, resolved,
            )));
            self.last_configuration = Some(resolved);
        }
    }

    /// Record raw parameter mirrors at plugin-block granularity.
    pub fn params(&mut self, t: f64, values: [f32; ParamKey::ALL.len()]) {
        for (i, value) in values.into_iter().enumerate() {
            if self.last_params[i] != value {
                self.last_params[i] = value;
                self.push(Entry::Param { t, key: i, value });
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.fence.enabled.load(Ordering::Acquire) {
            self.finish_callback();
        }
        if self.record_epoch != 0
            && (self.closed_epoch < self.record_epoch
                || (self.fence.enabled.load(Ordering::Acquire)
                    && self.fence.configuration_closed.load(Ordering::Acquire) < self.record_epoch)
                || (self.fence.canonical_enabled.load(Ordering::Acquire)
                    && self.fence.source_closed.load(Ordering::Acquire) < self.record_epoch))
        {
            self.fence.fail();
        }
    }
}

/// The GUI-thread half: start and stop recording, and report what
/// happened. Cloneable so the editor can hold it.
#[derive(Clone)]
pub struct Control {
    display: Arc<Mutex<Option<publication::Consumer>>>,
    fence: Arc<RecordFence>,
    commands: mpsc::Sender<Command>,
    dropped: Arc<AtomicU64>,
    /// One line for the UI, owned by whichever side last had news.
    status: Arc<Mutex<String>>,
    /// Path of the take most recently finished this session — the target for
    /// [`render_now`](Self::render_now).
    last_take: Arc<Mutex<Option<std::path::PathBuf>>>,
    recording: Arc<AtomicBool>,
    /// Set by the audio thread; the GUI's only honest view of whether
    /// the transport is moving.
    rolling: Arc<AtomicBool>,
    with_audio: Arc<AtomicBool>,
    /// Mirror for the audio thread of whether a backward jump ends the take.
    end_at_rewind: Arc<AtomicBool>,
    /// Whether any transport block has been accepted in this take.
    rolled: Arc<AtomicBool>,
    /// The audio thread's count of notes in the current take.
    captured: Arc<AtomicU64>,
    /// Set by the audio thread when the transport went backwards: the take is
    /// done and the GUI should stop + render it.
    hit_rewind: Arc<AtomicBool>,
    /// The bar to end the take at, and the audio thread's latch saying it
    /// happened. See [`StopAtBar`].
    stop_at_bar: Arc<StopAtBar>,
    /// How far the background render has got, for the Video pane's bar.
    progress: Arc<Progress>,
    /// Shared by every render this Control starts, so a new request cancels
    /// the one in flight rather than racing it. See [`RenderControl`].
    render: Arc<RenderControl>,
}

impl Control {
    pub fn take_display(&self) -> Option<publication::Consumer> {
        self.display.lock().take()
    }
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Tell the audio thread whether a backward jump ends the take rather than
    /// splitting it — true for every trigger but
    /// [`OnDisarm`](harmonigraph_take::RenderTrigger::OnDisarm), which is the
    /// one that has to survive a looping transport. Called every GUI frame.
    pub fn set_end_at_rewind(&self, on: bool) {
        self.end_at_rewind.store(on, Ordering::Relaxed);
    }

    /// Whether this take accepted a transport block, including audio-only
    /// exports and blocks whose notes are still awaiting publication.
    pub fn has_rolled(&self) -> bool {
        self.rolled.load(Ordering::Relaxed)
    }

    /// Notes the current take has captured, by either path into it.
    pub fn captured(&self) -> u64 {
        self.captured.load(Ordering::Relaxed)
    }

    /// Whether the audio thread saw the transport go backwards and ended the
    /// take — the GUI's cue to stop recording and render the one pass.
    pub fn hit_rewind(&self) -> bool {
        self.hit_rewind.load(Ordering::Relaxed)
    }

    /// The bar to end the take at, or `None` for every trigger but
    /// [`AtBar`](harmonigraph_take::RenderTrigger::AtBar). Called every GUI
    /// frame, like [`set_end_at_rewind`](Self::set_end_at_rewind), so a
    /// mid-take change of mind reaches the audio thread.
    pub fn set_stop_bar(&self, bar: Option<f64>) {
        self.stop_at_bar.set(bar);
    }

    /// Whether the audio thread played the take through its stop bar and ended
    /// it there — the GUI's cue to stop recording and render.
    pub fn hit_stop_bar(&self) -> bool {
        self.stop_at_bar.hit.load(Ordering::Relaxed)
    }

    /// Whether the audio thread last saw the transport moving.
    pub fn is_rolling(&self) -> bool {
        self.rolling.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> String {
        let status = self.status.lock().clone();
        if status == CONFIGURATION_FAILURE {
            self.fence.failure_message.lock().clone().unwrap_or(status)
        } else {
            status
        }
    }

    /// The take most recently finished this session, if any.
    pub fn last_take(&self) -> Option<std::path::PathBuf> {
        self.last_take.lock().clone()
    }

    /// Render the last finished take now, in the background, with `request`
    /// (which carries the current look, bounce, and offset).
    pub fn render_now(&self, request: RenderRequest) {
        match self.last_take() {
            Some(path) => spawn_render(
                request,
                path,
                self.status.clone(),
                self.progress.clone(),
                self.render.clone(),
            ),
            None => *self.status.lock() = "no take recorded yet to render".into(),
        }
    }

    /// How far the render running in the background has got, or `None` when
    /// none is. Read every GUI frame; see [`Progress`].
    pub fn render_progress(&self) -> Option<harmonigraph_take::RenderProgress> {
        self.progress.read()
    }

    /// Stop the render running in the background and throw away the part of
    /// the video it had written.
    ///
    /// The deletion is the render thread's own — see
    /// [`RenderControl::cancel`]. A video an EARLIER render finished is not
    /// touched: only the run in flight has anything half-written, and the
    /// finished one is a file that came out whole.
    pub fn cancel_render(&self) {
        if self.render.cancel() {
            *self.status.lock() = "render cancelled — the part-written video goes with it".into();
        }
    }

    /// Begin a take. `appearance` is the appearance document that decides how the
    /// replay will look; `sample_rate` stamps the header. `audio`
    /// records the selected audio stream alongside the notes.
    pub fn start(&self, sample_rate: f32, appearance: String, audio: bool) {
        if self.is_recording() {
            return;
        }
        if self.fence.failed.load(Ordering::Acquire) {
            *self.status.lock() =
                "recording incomplete — reload the plugin before starting another take".into();
            return;
        }
        if self.fence.finishing.load(Ordering::Acquire) {
            *self.status.lock() =
                "finishing the previous take — waiting for its recording prefix".into();
            return;
        }
        let dir = take_dir();
        #[cfg(feature = "test-support")]
        let dir = self.fence.test_directory.lock().clone().unwrap_or(dir);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            *self.status.lock() = format!("cannot create {}: {err}", dir.display());
            return;
        }
        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let base =
            dir.join(format!("take-{}.{}", stamp_for(epoch_secs), harmonigraph_take::EXTENSION));
        let path = disambiguate(base);
        let header = header_for(sample_rate, appearance);

        self.dropped.store(0, Ordering::Relaxed);
        self.with_audio.store(audio, Ordering::Relaxed);
        let spec = audio.then_some(AudioSpec { sample_rate, channels: TAKE_CHANNELS as u16 });
        let Some(epoch) =
            self.fence.epoch().checked_add(1).filter(|epoch| *epoch < CALLBACK_ACTIVE >> 1)
        else {
            *self.status.lock() = "recording epoch exhausted".into();
            return;
        };
        if self.commands.send(Command::Start(epoch, Box::new(header), path, spec)).is_err() {
            *self.status.lock() = "take writer thread is gone".into();
            return;
        }
        self.recording.store(true, Ordering::Relaxed);
        self.rolling.store(false, Ordering::Relaxed);
        // Clear a previous take's end latches so neither can end this one
        // before the transport even rolls — nor its note count, which would
        // read as this take being under way. The audio thread also clears
        // them on arm.
        self.hit_rewind.store(false, Ordering::Relaxed);
        self.stop_at_bar.hit.store(false, Ordering::Relaxed);
        self.captured.store(0, Ordering::Relaxed);
        self.rolled.store(false, Ordering::Relaxed);
        // Finishing barred Start until every old armed callback retired.
        // An overlapping idle callback captured disarmed and owns no audio,
        // so its activity bit cannot carry ownership into this new epoch.
        self.fence.intent.store(epoch << 1 | 1, Ordering::Release);
        *self.status.lock() = "armed — waiting for the transport to roll".into();
    }

    /// Stop recording, optionally rendering the finished take to video.
    ///
    /// The render is launched by the writer thread, after it has closed
    /// the file — the only place that knows the take is actually complete.
    pub fn stop(&self, render: Option<RenderRequest>) {
        if !self.is_recording() {
            return;
        }
        // Stop is intent, not producer closure. Keep the selected audio mode
        // until the next Start so a callback that observed armed can finish.
        let epoch = self.fence.epoch();
        self.fence.finishing.store(true, Ordering::Release);
        self.fence.intent.fetch_and(!1, Ordering::AcqRel);
        let _ = self.commands.send(Command::Stop(epoch, render.map(Box::new)));
        self.recording.store(false, Ordering::Relaxed);
    }

    /// Called each GUI frame while recording, so the status line reflects
    /// what the audio thread is actually doing.
    pub fn tick(&self, rolling: bool, events: u64) {
        if !self.is_recording() {
            return;
        }
        let dropped = self.dropped.load(Ordering::Relaxed);
        *self.status.lock() = if self.fence.failed.load(Ordering::Acquire) {
            CONFIGURATION_FAILURE.into()
        } else if dropped > 0 {
            format!("RECORDS DROPPED ({dropped}) — the take is incomplete")
        } else if rolling {
            format!("recording — {events} events")
        } else if events > 0 {
            format!("paused ({events} events) — transport stopped")
        } else {
            "armed — waiting for the transport to roll".into()
        };
    }
}

/// The take's opening line: everything constant for the whole recording,
/// decided before the first event is written and not revisable afterwards.
///
/// Shared by the recording control and capture probes; the UI supplies the
/// serialized appearance while this layer remains independent of its types.
///
/// Each field fails SILENTLY rather than loudly if it stops being set, which
/// is what makes the three of them worth a function and a test of their own
/// rather than a struct literal inline:
///
/// - `appearance` is the appearance document that decides how the replay LOOKS. Unset,
///   the take still records and the render still succeeds — and the video is
///   of the default palette and camera instead of the ones the take
///   was recorded under.
/// - `sample_rate` is the take's whole time base. Unset it falls back to
///   [`harmonigraph_take::Header::default`], which is 48 kHz rather than a
///   zero that would show up at once: every event lands at the wrong offset
///   and the video drifts against the bounce it was supposed to line up with
///   by construction.
/// - `source` says which shell wrote the take — this one, or the standalone,
///   which writes its own name. Nothing reads it: it is a field for whoever
///   is holding a take and asking where it came from, and the reason it
///   belongs here is that a shell which forgets to stamp it leaves that
///   question unanswerable later, with nothing at the time to notice.
pub fn header_for(sample_rate: f32, appearance: String) -> harmonigraph_take::Header {
    harmonigraph_take::Header {
        sample_rate,
        appearance: Some(appearance),
        source: "harmonigraph".into(),
        ..Default::default()
    }
}

/// Turn a Unix timestamp into the sortable, legible stamp a take's filename
/// carries — `YYYY-MM-DD_HH-MM-SS`, UTC. UTC rather than the host's local
/// time because reading it back out needs no timezone database, only `:`
/// is invalid in a filename on every platform this runs on so hyphens
/// stand in for it, and the lexical and chronological orders coincide.
fn stamp_for(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs_of_day = epoch_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (secs_of_day / 3_600, (secs_of_day / 60) % 60, secs_of_day % 60);
    format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

/// The proleptic-Gregorian calendar date `days` days after the Unix epoch —
/// Howard Hinnant's `civil_from_days`, exact over the whole range a take's
/// clock can produce.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

/// If `base` (or its `.wav` companion) already sits on disk — two takes
/// started within the same UTC second — append `_1`, `_2`, ... until a name
/// neither file uses, rather than let the second take silently truncate the
/// first's. Distinct from the writer's `Open::path_for` suffix `-N`, which numbers later
/// PASSES of one take rather than takes that collided on a name.
fn disambiguate(base: std::path::PathBuf) -> std::path::PathBuf {
    let taken = |path: &std::path::Path| path.exists() || path.with_extension("wav").exists();
    if !taken(&base) {
        return base;
    }
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("take").to_owned();
    (1..)
        .map(|n| base.with_file_name(format!("{stem}_{n}.{}", harmonigraph_take::EXTENSION)))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded counter always finds a free name")
}

/// Where takes go. `LATTICE_TAKE_DIR` overrides; the default is a fixed,
/// findable place, because a DAW's environment usually has neither the
/// variable nor a useful working directory.
fn take_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("LATTICE_TAKE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    home_dir().join("Music").join("Harmonigraph Takes")
}
