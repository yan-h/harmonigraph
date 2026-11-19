//! Audio-thread capture and GUI control share the recording protocol here.
//! The writer owns files and finalization; render jobs own subprocesses.
//! Keeping both endpoint structs here lets the writer construct their private
//! state without exposing fields or adding a second wiring API.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};

use harmonigraph_core::notes::NoteEventKind;
use harmonigraph_take::ParamKey;
use parking_lot::Mutex;

use crate::{configuration, publication};
use render_job::{spawn_render, Progress, RenderControl, RenderRequest};

mod lifecycle;
mod render_job;
mod writer;

pub use render_job::default_renderer_path;
pub use writer::channel;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use writer::testing;

use configuration::{RecordFence, RecordIntent};
use lifecycle::{Action, End, History, Observation, Policy, State};

/// Record-ring capacity. Expressive voices can produce several records per
/// audio block, multiplied by export speed. This bounded queue absorbs worker
/// scheduling delays; the separate canonical publication lane has its own cap.
const TAKE_RING_CAPACITY: usize = 1 << 16;

const FINISHING: &str = "finishing…";
const FINISHING_PREVIOUS: &str = "finishing the previous take…";
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
///
/// Everything here is on the take's presentation clock. A `Run` says where
/// that clock sits in the take's audio; the reader places every record by its
/// own time.
#[derive(Clone, Copy)]
pub enum Entry {
    /// A stretch that records without a pause opens at presentation time `at`,
    /// which sits at `song` in the take's audio.
    Run {
        at: f64,
        song: f64,
    },
    /// The open run's first sample that did not record, on the same clock.
    RunEnd {
        at: f64,
    },
    Configuration(harmonigraph_take::ConfigurationRecord),
    /// Exactly this committed audio prefix belongs at this point in the stream.
    AudioSamples(usize),
    /// `key` is an index into [`ParamKey::ALL`] — an id string would mean
    /// allocating on the audio thread.
    Param {
        t: f64,
        key: usize,
        value: f32,
    },
}

// [`TAKE_RING_CAPACITY`] of these are allocated up front and stay resident for
// the life of the plugin instance, so the cell's size IS a per-instance memory
// cost: 96 B × 65,536 is exactly 6 MiB, before the audio ring's 4 MiB beside
// it. Nothing else here reads that number, which is what makes it worth
// pinning where `publication.rs` pins its own cell — a variant that doubled it
// would cost megabytes an instance with nothing on screen to say so.
//
// `Configuration` is the widest variant and the one that will move: an 80-byte
// `ConfigurationRecord`, which carries the whole resolved configuration by value
// because the audio thread cannot allocate a box for it.
const _: () = assert!(std::mem::size_of::<Entry>() <= 96);
const _: () = assert!(std::mem::align_of::<Entry>() <= 8);

/// What the writer thread needs to open a WAV beside the take.
#[derive(Clone, Copy)]
pub struct AudioSpec {
    pub sample_rate: f32,
    pub channels: u16,
}

enum Command {
    Start(Box<harmonigraph_take::Header>, std::path::PathBuf, AudioSpec),
    /// Close the file once what it recorded is written, and render it.
    Stop,
}

/// The user's home directory, or `.` when `HOME` is unset — the base for the
/// renderer and takes locations below.
fn home_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// The stop bar requested by the GUI. Completion belongs to the take's
/// single end-reason latch, just like a Loop end wrap.
/// NaN represents off so bar zero remains a valid target.
struct StopAtBar {
    bar: AtomicU64,
}

impl Default for StopAtBar {
    fn default() -> Self {
        StopAtBar { bar: AtomicU64::new(f64::NAN.to_bits()) }
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

/// What a take must not inherit from the take before it, and the one place it
/// is cleared.
///
/// One struct for the same reason [`StopAtBar`] is one: these are cleared
/// together or not at all, and both halves of the recorder hold all four. They
/// are cleared TWICE per take, on purpose — the audio thread clears them on the
/// arming edge ([`Recorder::update_armed`]) and the GUI clears them when it
/// sends `Start` ([`Control::start`]) — because either can go first, and a take
/// that armed before its command landed would otherwise read as already under
/// way. What is not on purpose is two hand-written copies of the same reset
/// stores, which is how one of them comes to be missing a latch the other
/// gained (#895).
#[derive(Default)]
struct TakeLatches {
    /// Notes played under the current take's arm once its first run opened. The
    /// status line reads this independently of transport progress: audio-only
    /// takes and delayed notes also roll.
    captured: AtomicU64,
    /// Whether this take has recorded a block yet, published for the GUI's
    /// Transport-stop countdown (`Control::has_rolled`).
    rolled: AtomicBool,
    /// One exclusive lifecycle completion, published for automatic Stop.
    /// Zero means still open; the other values are `End`.
    end: AtomicU8,
    /// The bar the GUI wants the take to end at.
    /// See [`StopAtBar`] and [`Recorder::observe_bar`].
    stop_at_bar: StopAtBar,
}

impl TakeLatches {
    fn end(&self) -> Option<End> {
        match self.end.load(Ordering::Relaxed) {
            1 => Some(End::Rewind),
            2 => Some(End::Bar),
            _ => None,
        }
    }

    /// Clear what the previous take left behind, so its end reason cannot end
    /// this one before the transport even rolls — nor its note count, which
    /// would read as this take being under way.
    ///
    /// The stop BAR itself is not here: it is the GUI's setting for the take
    /// about to run, written every frame, and clearing it would disarm the
    /// trigger at the moment it is needed. The completion reason belongs to
    /// the finished take.
    fn clear(&self) {
        self.captured.store(0, Ordering::Relaxed);
        self.rolled.store(false, Ordering::Relaxed);
        self.end.store(0, Ordering::Relaxed);
    }
}

/// The audio-thread half: push entries, gated by an atomic the GUI owns.
pub struct Recorder {
    /// Pins the writer independently from all GUI Control clones, so a take
    /// still recording after editor teardown keeps its writer.
    _writer_lifetime: mpsc::Sender<Command>,
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
    /// The epoch of the take last seen armed.
    armed_epoch: u64,
    producer: rtrb::Producer<Entry>,
    /// Interleaved input samples, recorded beside every take's notes.
    audio: rtrb::Producer<f32>,
    dropped: Arc<AtomicU64>,
    /// Last value written per parameter, so only changes are recorded.
    /// Reset to NaN on arm so the take's first block writes a full set — a
    /// take that inherited "no change since last time" would replay with the
    /// default camera.
    last_params: [f32; ParamKey::ALL.len()],
    lifecycle: State,
    /// Independent position/duration and bar observations, including rejected scrubs.
    history: History,
    /// Published for the GUI: is the transport actually moving?
    rolling: Arc<AtomicBool>,
    /// The serial of the last `Run` this recorder opened: each one takes the
    /// next, whatever closed the one before it.
    run: u64,
    /// The `Run` open on the ring. The one record of whether this block
    /// records. See [`Recorder::record`].
    open: Option<OpenRun>,
    /// Whether this take has opened a run, so its notes count from there: the
    /// On transport stop countdown reads the count, and a note played before
    /// playback must not start it.
    run_opened: bool,
    /// Set by the GUI under `AtLoopEnd`, whose take is over the moment the
    /// loop wraps: a backward jump after forward motion ends it there rather
    /// than recording on.
    end_at_rewind: Arc<AtomicBool>,
    /// What the take in progress has latched, shared with the [`Control`] that
    /// reads it and clears it. See [`TakeLatches`].
    latches: Arc<TakeLatches>,
}

impl Recorder {
    /// The audio-thread half over these rings and lanes, with fresh shared
    /// state for the [`Control`] or fixture beside it to clone out, and every
    /// per-take field at the value a new instance starts from.
    ///
    /// The one place the struct is spelled: the writer's `channel` and both
    /// test fixtures used to write it out whole, so each new field was three
    /// edits (#1129, #901, #850, #849). Its one caller is the writer's
    /// `endpoints`.
    fn new(
        producer: rtrb::Producer<Entry>,
        audio: rtrb::Producer<f32>,
        publication: publication::Publisher,
        display: publication::Publisher,
        writer_lifetime: mpsc::Sender<Command>,
    ) -> Recorder {
        Recorder {
            _writer_lifetime: writer_lifetime,
            fence: Arc::default(),
            publication,
            display,
            outage: publication::Lanes::default(),
            armed_epoch: 0,
            producer,
            audio,
            dropped: Arc::default(),
            last_params: [f32::NAN; ParamKey::ALL.len()],
            lifecycle: State::Disarmed,
            history: History::default(),
            rolling: Arc::default(),
            run: 0,
            open: None,
            run_opened: false,
            end_at_rewind: Arc::default(),
            latches: Arc::default(),
        }
    }

    /// What EACH lane can still take. There is deliberately no combined
    /// number: a full display ring must not gate a snapshot an otherwise
    /// healthy writer is waiting for, and the minimum of the two did exactly
    /// that (#712).
    pub fn publication_free(&self) -> publication::Lanes<usize> {
        publication::Lanes { take: self.publication.free(), display: self.display.free() }
    }
    /// Consume a real publication serial on both lanes. Each queues its own
    /// gap immediately, so an absent drainer cannot hide this missing cut.
    pub fn publication_lost(&mut self, time: f64) {
        self.publication.discarded(time);
        self.display.discarded(time);
        self.outage = publication::Lanes::both(true);
        self.publication_result(Err(publication::PublishError::Lost));
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
    /// The same delta on both lanes, with an outcome for each. There is no
    /// combined `Result`: a lane that lost the report owes a snapshot on that
    /// lane alone, and the caller has to say which.
    pub fn publish_note(
        &mut self,
        note: harmonigraph_core::canonical::NoteDelta,
    ) -> publication::Lanes<Result<(), publication::PublishError>> {
        let take = self.publication.note(note);
        // Only the take lane's own outcome accounts the take. A display lane
        // that overflowed still returns Err on its own half, so the caller
        // arms a snapshot there, but it must not touch the file.
        self.publication_result(take);
        // A note played while the take is armed, once its first run has
        // opened. A reset is no note of the take's.
        if take.is_ok()
            && self.lifecycle.armed()
            && self.run_opened
            && !matches!(note.event.kind, NoteEventKind::SourceReset | NoteEventKind::SessionReset)
        {
            self.latches.captured.fetch_add(1, Ordering::Relaxed);
        }
        let display = self.display.note(note);
        self.outage.take |= take == Err(publication::PublishError::Lost);
        self.outage.display |= display == Err(publication::PublishError::Lost);
        publication::Lanes { take, display }
    }

    /// One lane at a time, so a display ring nobody drains cannot hold a
    /// snapshot back from the take, or the take's from the display (#712).
    pub fn publish_baseline(
        &mut self,
        lane: publication::Lane,
        baseline: &harmonigraph_core::canonical::SourceBaseline,
    ) -> Result<(), publication::PublishError> {
        match lane {
            publication::Lane::Take => {
                let result = self.publication.baseline(baseline);
                self.publication_result(result);
                result
            }
            publication::Lane::Display => self.display.baseline(baseline),
        }
    }

    /// What a take-lane publication outcome costs the file.
    ///
    /// A hole in the note history is NOT a recording failure (#712): the take
    /// keeps capturing, finalises normally and exports with a warning, and the
    /// gap the lane queued on its reserved cell is what carries that warning
    /// into the file as an `IncompleteRecord`. Refusal is left to failures
    /// that are actually fatal to the recording — audio, ownership and I/O,
    /// which reach the fence through [`Recorder::fail_configuration`] and the
    /// writer's paths — plus `Invalid` here, which is the one publication
    /// outcome with NO gap to describe it and so the one that would otherwise
    /// leave a silent hole.
    fn publication_result(&self, result: Result<(), publication::PublishError>) {
        match result {
            Ok(()) | Err(publication::PublishError::Busy) => {}
            Err(publication::PublishError::Lost) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(publication::PublishError::Invalid) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                if self.lifecycle.armed() || self.fence.finishing.load(Ordering::Acquire) {
                    self.fence.fail();
                }
            }
        }
    }

    /// [`is_armed_at`](Self::is_armed_at) the intent as it stands now, for a
    /// caller with no callback boundary of its own to have captured it at.
    pub fn is_armed(&mut self) -> bool {
        self.is_armed_at(self.capture_recording_intent())
    }

    /// Use the arm/disarm intent captured at the enclosing callback boundary,
    /// so a concurrent stop cannot cut off that callback's remaining audio.
    ///
    /// A new epoch armed is an arming edge even if no callback saw the
    /// previous take disarmed: a Stop and a Start between two callbacks.
    pub fn is_armed_at(&mut self, intent: RecordIntent) -> bool {
        if intent.is_armed() && intent.epoch() != self.armed_epoch {
            self.update_armed(false);
            self.armed_epoch = intent.epoch();
        }
        self.update_armed(intent.is_armed())
    }

    fn update_armed(&mut self, armed: bool) -> bool {
        let (action, _) = self.advance(Observation::Armed(armed));
        if action == Action::Arm {
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
            self.run_opened = false;
            self.latches.clear();
        }
        armed
    }

    fn push(&mut self, entry: Entry) {
        if self.producer.push(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.fence.fail();
        }
    }

    /// The run this block records in, or `None` when it records nothing.
    ///
    /// Each new run owes the take the notes already sounding: whatever began
    /// while nothing recorded — before arming, or in a pause — would otherwise
    /// reach it only as releases, and replay draws nothing for those (#1129).
    /// A caller remembers the run it last paid and pays again when this
    /// differs, so the debt is paid once per run however many blocks ask. The
    /// Hub pays it with a snapshot of every row.
    pub fn open_run(&self) -> Option<u64> {
        self.open.map(|open| open.serial)
    }

    /// Whether this block records.
    pub fn recording(&self) -> bool {
        self.open.is_some()
    }

    /// Close or open this block's run. `origin` is where the block sits in the
    /// take's audio when it records, and `at` is its first sample on the
    /// presentation clock. The transport observation gives an origin only to
    /// an armed take that rolls, and a block with no transport position gives
    /// its local clock to any armed take.
    ///
    /// A run opened before the take was last armed ends here, and the block
    /// opens a new one: a Stop and a Start between two callbacks.
    pub fn record(&mut self, at: f64, origin: Option<f64>) {
        if self.open.is_some_and(|open| origin.is_none() || open.take != self.armed_epoch) {
            self.run_end(at);
        }
        if let (Some(song), None) = (origin, self.open) {
            self.run(at, song);
        }
    }

    /// Every armed take records audio, until the take has failed.
    pub fn wants_audio(&self) -> bool {
        self.lifecycle.armed() && !self.fence.failed.load(Ordering::Acquire)
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
    /// i.e. whether this block's events belong in the take — with where the
    /// block sits in the take's audio. Called once per block with the block's
    /// song position, the host's own `playing` flag, and this block's frame
    /// count divided by its sample rate.
    ///
    /// The answer is the AUDIO's position, not the host's: the transport's
    /// position at the take's first block, and the end of the audio already
    /// recorded after that. A resume from a pause then continues the WAV, and
    /// so does a loop wrap or a seek; the host's position would put the notes
    /// after any of them beside a soundtrack that does not have them there
    /// (#1760).
    ///
    /// A playing host records immediately. A stopped host records only when
    /// its position advances by the PREVIOUS callback's duration, within 50%
    /// either way (inclusive). This allows host timing/reporting variation and
    /// variable callback sizes without accepting arbitrary forward scrubs as
    /// offline export progress. The first stopped observation only seeds history;
    /// rejected jumps also update it, so an export can start at a new position.
    /// A tiny manual scrub in that interval is indistinguishable from export:
    /// it records one block. Repeated accepted tiny scrubs record one block
    /// each.
    ///
    /// A seek, or a loop wrap, records on from the end of the audio already
    /// recorded. Parked observations preserve that audio end, so
    /// seek-and-resume has the same behavior as seeking during playback.
    ///
    /// Any stopped backward movement is a rewind, including the restore of a
    /// single accepted export block, which records nothing. Playing hosts
    /// retain the 50 ms backward jitter allowance used for loop detection.
    /// Under `end_at_rewind` (Loop end) a rewind after forward motion ends the
    /// take; under every other trigger the take records on.
    pub fn observe_transport(
        &mut self,
        position: f64,
        playing: bool,
        duration: f64,
    ) -> Option<f64> {
        let (action, origin) = self.advance(Observation::Transport { position, playing, duration });
        let rolling = action == Action::Record;
        if rolling {
            self.latches.rolled.store(true, Ordering::Relaxed);
        }
        self.rolling.store(rolling, Ordering::Relaxed);
        origin.filter(|_| rolling)
    }

    /// End the take if this block played THROUGH the stop bar, and answer
    /// whether it did. Called once per block with the transport's bar position,
    /// BEFORE [`observe_transport`](Self::observe_transport): a block that ends
    /// the take here contributes nothing, so the cut lands on a block boundary
    /// at or before the bar rather than a block after it.
    ///
    /// The bar is counted from ZERO at the song's start, which is the base the
    /// transport reports; the arranger's 1-based number is converted on the way
    /// in by `RenderConfig::stop_at_bar`.
    ///
    /// **The test is a CROSSING, not a level.** A take armed with the playhead
    /// already past the bar never sees a position below it, so it never fires
    /// and simply runs until you disarm — the same graceful nothing
    /// `AtLoopEnd` does with
    /// looping off. A level test would instead end it on the first block, with
    /// an empty file and a render of nothing, which is #569's failure exactly.
    ///
    /// **A crossing also has to be a small step.** A playhead DRAGGED past the
    /// bar crosses it as surely as playback does, and the two are
    /// indistinguishable in this API: an offline export reports
    /// `playing = false` throughout while its position climbs, so the host's
    /// own flag cannot separate them (#569 eliminated that). What does separate
    /// them is size — playback advances by a block, a drag by seconds — so a
    /// crossing wider than one bar is read as a drag and only moves
    /// bar history. The residual case is a drag that lands within a bar of where
    /// it started and straddles the stop bar; it ends the take early, visibly
    /// (Record take switches off), and re-arming is the whole of the repair.
    pub fn observe_bar(&mut self, bar: Option<f64>) -> bool {
        self.advance(Observation::Bar(bar)).0 == Action::Complete(End::Bar)
    }

    /// Snapshot shared policy and publish decisions outside the pure transition.
    /// Callback and configuration ownership are deliberately unaffected here.
    fn advance(&mut self, observation: Observation) -> (Action, Option<f64>) {
        let policy = Policy {
            end_at_rewind: self.end_at_rewind.load(Ordering::Relaxed),
            stop_bar: self.latches.stop_at_bar.get(),
        };
        let next = lifecycle::transition(self.lifecycle, self.history, observation, policy);
        self.lifecycle = next.state;
        self.history = next.history;
        if let Action::Complete(end) = next.action {
            self.latches.end.store(end as u8, Ordering::Relaxed);
            self.rolling.store(false, Ordering::Relaxed);
        }
        (next.action, next.origin)
    }

    /// Capture a callback boundary's arm/disarm intent. A Stop after it cannot
    /// cut off that callback's audio: the run ends only when a later boundary
    /// observes the disarm.
    pub fn capture_recording_intent(&self) -> RecordIntent {
        let intent = self.fence.intent();
        #[cfg(feature = "test-support")]
        self.fence.boundary_pause.reach();
        intent
    }
    pub fn recording_epoch(&self) -> u64 {
        self.fence.epoch()
    }
    pub fn fail_configuration(&self) {
        self.fence.fail();
    }
    /// Open a run whose first sample sits at presentation time `at` and at
    /// `song` in the take's audio.
    fn run(&mut self, at: f64, song: f64) {
        self.run = self.run.wrapping_add(1);
        self.open = Some(OpenRun { serial: self.run, take: self.armed_epoch });
        self.run_opened = true;
        self.push(Entry::Run { at, song });
    }
    /// Close the open run, if one is, at the presentation time of its first
    /// sample that did not record.
    pub fn run_end(&mut self, at: f64) {
        if self.open.take().is_some() {
            self.push(Entry::RunEnd { at });
        }
    }
    pub fn configuration(
        &mut self,
        t: f64,
        resolved: harmonigraph_core::configuration::ResolvedConfig,
    ) {
        self.push(Entry::Configuration(harmonigraph_take::ConfigurationRecord::new(t, resolved)));
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

/// A `Run` on the ring that no `RunEnd` has followed yet.
#[derive(Clone, Copy)]
struct OpenRun {
    /// Its serial, which the Hub pays a snapshot per (#1129).
    serial: u64,
    /// The epoch of the take armed when it opened.
    take: u64,
}

/// A plugin destroyed mid-take fails it: nothing will close its run or
/// publish what it still held. A stopped take seals only once its run's end is
/// written (#1776), so one whose run is still open fails too: still armed at
/// deactivate, so `close_stopped` closed nothing, then disarmed while inactive.
impl Drop for Recorder {
    fn drop(&mut self) {
        if self.open.is_some() || self.fence.intent().is_armed() {
            self.fence.fail();
        }
    }
}

/// The GUI-thread half: start and stop recording, and report what
/// happened. Cloneable so the editor can hold it.
#[derive(Clone)]
pub struct Control {
    stop_wakeup: Option<Arc<dyn Fn() + Send + Sync>>,
    display: Arc<Mutex<Option<publication::Consumer>>>,
    fence: Arc<RecordFence>,
    commands: mpsc::Sender<Command>,
    dropped: Arc<AtomicU64>,
    /// One line for the UI, owned by whichever side last had news.
    status: Arc<Mutex<String>>,
    /// Path of the take most recently finished this session — the target for
    /// [`queue_export`](Self::queue_export).
    last_take: Arc<Mutex<Option<std::path::PathBuf>>>,
    recording: Arc<AtomicBool>,
    /// Set by the audio thread; the GUI's only honest view of whether
    /// the transport is moving.
    rolling: Arc<AtomicBool>,
    /// Mirror for the audio thread of whether a backward jump ends the take.
    end_at_rewind: Arc<AtomicBool>,
    /// What the take in progress has latched, shared with the [`Recorder`] that
    /// sets it. See [`TakeLatches`].
    latches: Arc<TakeLatches>,
    /// How far the background render has got, for the Video pane's bar.
    progress: Arc<Progress>,
    /// Shared by every render this Control starts, so a new request cancels
    /// the one in flight rather than racing it. See [`RenderControl`].
    render: Arc<RenderControl>,
}

impl Control {
    /// Install before sharing this control. Stop only requests service; the
    /// owner must establish a joined lifecycle boundary before closing audio.
    pub fn set_stop_wakeup(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.stop_wakeup = Some(Arc::new(wake));
    }

    pub fn take_display(&self) -> Option<publication::Consumer> {
        self.display.lock().take()
    }
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Tell the audio thread whether a backward jump ends the take rather than
    /// recording on — true only for `AtLoopEnd`, whose take is one loop.
    /// Called at every poll of the take's end: each input worker step, and
    /// each editor frame.
    pub fn set_end_at_rewind(&self, on: bool) {
        self.end_at_rewind.store(on, Ordering::Relaxed);
    }

    /// Whether this take accepted a transport block, including audio-only
    /// exports and blocks whose notes are still awaiting publication.
    pub fn has_rolled(&self) -> bool {
        self.latches.rolled.load(Ordering::Relaxed)
    }

    /// Notes the current take has captured, by either path into it.
    pub fn captured(&self) -> u64 {
        self.latches.captured.load(Ordering::Relaxed)
    }

    /// Whether the audio thread saw the transport go backwards and ended the
    /// take — the GUI's cue to stop recording and render it.
    pub fn hit_rewind(&self) -> bool {
        self.latches.end() == Some(End::Rewind)
    }

    /// The bar to end the take at, or `None` for every trigger but
    /// `AtBar`. Called at every poll of the take's end, like
    /// [`set_end_at_rewind`](Self::set_end_at_rewind), so a mid-take change
    /// of mind reaches the audio thread.
    pub fn set_stop_bar(&self, bar: Option<f64>) {
        self.latches.stop_at_bar.set(bar);
    }

    /// Whether the audio thread played the take through its stop bar and ended
    /// it there — the GUI's cue to stop recording and render.
    pub fn hit_stop_bar(&self) -> bool {
        self.latches.end() == Some(End::Bar)
    }

    /// An audio-owned completion remains authoritative even if the GUI's
    /// trigger changes before it observes the completed prefix.
    pub fn has_ended(&self) -> bool {
        self.latches.end().is_some()
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

    /// Append an independent export with settings already captured by the caller.
    ///
    /// `appearance` is the document selected at queue time (current, saved or
    /// recorded), and the renderer takes the output size from it alone: `size`
    /// is only what the Exports list shows for the job.
    pub fn queue_export(&self, path: std::path::PathBuf, appearance: String, size: [u32; 2]) {
        let request = RenderRequest { appearance: Some(appearance) };
        spawn_render(request, path, Some(size), self.progress.clone(), self.render.clone());
    }
    pub fn export_jobs(&self) -> Vec<harmonigraph_take::render::ExportJob> {
        self.render.snapshots(&self.progress)
    }
    pub fn cancel_export(&self, id: u64) {
        self.render.cancel_job(id);
    }
    pub fn retry_export(&self, id: u64) {
        self.render.retry(id, self.progress.clone());
    }
    pub fn clear_finished_exports(&self) {
        self.render.clear_finished();
    }
    pub fn shutdown_exports(&self) {
        self.render.shutdown();
    }

    /// Begin a take. `appearance` is the appearance document that decides how the
    /// replay will look; `sample_rate` stamps the header and the WAV the
    /// selected audio stream is recorded to alongside the notes.
    pub fn start(&self, sample_rate: f32, appearance: String) {
        if self.is_recording() {
            return;
        }
        if self.fence.failed.load(Ordering::Acquire) {
            *self.status.lock() =
                "recording incomplete — reload the plugin before starting another take".into();
            return;
        }
        {
            // Completion uses this same lock before releasing the fence, so
            // a rejected Start cannot publish "finishing" after success.
            let mut status = self.status.lock();
            if self.fence.finishing.load(Ordering::Acquire) {
                *status = FINISHING_PREVIOUS.into();
                return;
            }
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
        let path = base;
        let header = header_for(sample_rate, appearance);

        self.dropped.store(0, Ordering::Relaxed);
        let spec = AudioSpec { sample_rate, channels: TAKE_CHANNELS as u16 };
        let Some(epoch) =
            self.fence.epoch().checked_add(1).filter(|epoch| *epoch <= RecordIntent::MAX_EPOCH)
        else {
            *self.status.lock() = "recording epoch exhausted".into();
            return;
        };
        if self.commands.send(Command::Start(Box::new(header), path, spec)).is_err() {
            *self.status.lock() = "take writer thread is gone".into();
            return;
        }
        self.recording.store(true, Ordering::Relaxed);
        self.rolling.store(false, Ordering::Relaxed);
        // The audio thread clears these too, on the arming edge; see
        // [`TakeLatches`] for why both ends do it.
        self.latches.clear();
        // Finishing barred Start until every old armed callback retired.
        self.fence.arm(epoch);
        *self.status.lock() = "waiting for playback".into();
    }

    /// Stop recording and render the finished take to video.
    ///
    /// The render is launched by the writer thread, after it has closed
    /// the file — the only place that knows the take is actually complete.
    /// It renders the take's own recorded look; a take that failed is never
    /// rendered.
    pub fn stop(&self) {
        if !self.is_recording() {
            return;
        }
        // Stop is intent: a callback that observed armed still finishes its
        // block, audio included, and the writer seals once its run ends.
        self.fence.finishing.store(true, Ordering::Release);
        self.fence.disarm();
        let _ = self.commands.send(Command::Stop);
        self.recording.store(false, Ordering::Relaxed);
        if let Some(wake) = &self.stop_wakeup {
            wake();
        }
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
            format!("recording · {events} events")
        } else if events > 0 {
            format!("paused · {events} events")
        } else {
            "waiting for playback".into()
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

/// Where takes go, and so where their videos land. `LATTICE_TAKE_DIR`
/// overrides; the default is a fixed, findable place, because a DAW's
/// environment usually has neither the variable nor a useful working
/// directory.
pub fn take_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("LATTICE_TAKE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    home_dir().join("Music").join("Harmonigraph Takes")
}
