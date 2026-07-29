//! Recording a take: capturing everything the visualization is a
//! function of, so it can be re-rendered offline into a video.
//!
//! # Why this is a button and not automatic
//!
//! The first version armed itself when nice-plug reported
//! `ProcessMode::Offline`, on the theory that "exporting audio also
//! exports a take". Bitwig disproved it: an export produced a take
//! containing parameter values and 37 `AllOff`s and no notes at all,
//! while the lattice visibly lit up throughout. The only way both are
//! true is that the pass carrying the notes was **not** the pass flagged
//! offline — so the host runs some short offline probe and then renders
//! in realtime mode.
//!
//! Rather than reverse-engineer which pass is the real one, recording is
//! now explicit: a toggle in the Video pane, armed by the user, working
//! in any process mode. That also makes the good workflow possible —
//! play the piece once, as you would anyway, and render the video from
//! the take afterwards. The export never has to cooperate.
//!
//! # Time
//!
//! Events are stamped with **transport position**, not a plugin-local
//! clock, and only recorded while the transport is rolling. That means a
//! take lines up with a bounce of the same song with no offset to work
//! out — the two are measured from the same zero. It also means arming
//! the toggle while stopped records nothing until you hit play, which is
//! what the status line is for.
//!
//! Hosts that report no transport fall back to a plugin-local sample
//! count, so the standalone-style "just record what you play" case still
//! works; the take then starts at zero whenever recording was armed.
//!
//! # Threading
//!
//! The audio thread must not open files, allocate, or lock. It only
//! pushes plain `Copy` records into a ring and reads one atomic. A writer
//! thread — started once, for the plugin's lifetime — drains the ring and
//! takes open/close commands over a channel from the GUI thread.
//!
//! Unlike the note ring feeding the GUI, which drops on backpressure by
//! design (a stalled meter must never stall audio), **a dropped take
//! record is a silently wrong video**, so overflow is counted and
//! surfaced in the UI rather than ignored.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use harmonigraph_core::notes::NoteEventKind;
use harmonigraph_ui::params::ParamKey;
use parking_lot::Mutex;

/// Ring capacity. Sized for a fast offline render rather than for a
/// frame: even at 20x realtime a dense piece is only a few thousand
/// records a second, and the writer thread drains continuously.
const TAKE_RING_CAPACITY: usize = 1 << 16;

/// How long the writer thread sleeps when it finds the ring empty.
const DRAIN_IDLE: std::time::Duration = std::time::Duration::from_millis(20);

/// Capacity of the audio ring, in interleaved samples. Generous on
/// purpose: during an offline export audio arrives many times faster than
/// realtime, and unlike the spectrum's ring — which drops by design,
/// because a stalled meter must never stall audio — dropping here would
/// put a silent hole in the finished video.
const AUDIO_RING_CAPACITY: usize = 1 << 20;

/// One recorded thing, in a form the audio thread can push without
/// allocating. Converted to a `harmonigraph_take::Record` on the writer thread.
#[derive(Clone, Copy)]
pub enum Entry {
    Note { t: f64, channel: u8, note: u8, kind: NoteEventKind },
    /// `key` is an index into [`ParamKey::ALL`] — an id string would mean
    /// allocating on the audio thread.
    Param { t: f64, key: usize, value: f32 },
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
    Start(Box<harmonigraph_take::Header>, std::path::PathBuf, Option<AudioSpec>),
    /// Close the file, and — if asked — render it to video.
    Stop(Option<Box<RenderRequest>>),
}

/// What to run once a take is complete.
///
/// The plugin does not render video; `harmonigraph-offline` does, with a
/// headless GPU device and an ffmpeg pipe, neither of which belongs in a
/// real-time audio plugin. This just launches it, off the audio thread
/// and off the GUI thread, so a long render never touches the DAW.
pub struct RenderRequest {
    pub program: std::path::PathBuf,
    /// Bounced audio to mux in and feed the spectrum, if any.
    pub audio: Option<String>,
    /// `--align` value (take-time the audio starts), if set; else auto-align.
    pub align: Option<String>,
    /// A persist blob passed as `--ui-state`, overriding the take's record-time
    /// look — set for "Render now" so post-record settings reach the video;
    /// `None` for auto-render (which uses the take's own recorded look).
    pub ui_state: Option<String>,
    /// Output pixels, from the Video pane's Aspect and Resolution.
    ///
    /// Passed rather than left to the renderer's own default because that
    /// default knows the take's aspect but not which resolution was picked
    /// beside it.
    pub size: [u32; 2],
    /// Whether to bake the whole-song playhead spectrogram rather than the
    /// live scrolling one.
    ///
    /// `None` leaves it to the take's own recorded setting, which is what the
    /// Video pane's Spectrogram row writes — so the choice reaches the render
    /// through the take rather than through a flag. `Some(true)` forces it on;
    /// there is no forcing it off, because the renderer ORs the flag with the
    /// take's setting and a flag cannot subtract.
    pub playhead: Option<bool>,
}

impl RenderRequest {
    /// The render that runs when a take finishes. A finished take always renders
    /// now — its own recorded audio as the spectrogram, playhead on — so this is
    /// unconditional; the `Option` is kept only for `Control::stop`'s signature.
    /// Uses the take's own recorded look.
    pub fn from_config(config: &harmonigraph_ui::RenderConfig) -> Option<RenderRequest> {
        Some(Self::build(config, None))
    }

    /// Build a request for an explicit "Render now": always built, and it
    /// carries the CURRENT `ui_state` blob so the render reflects the frame,
    /// bounce, and offset dialed in *after* recording — not the take's
    /// record-time snapshot.
    pub fn render_now(config: &harmonigraph_ui::RenderConfig, ui_state: String) -> RenderRequest {
        Self::build(config, Some(ui_state))
    }

    /// A blank renderer path means "use the default" rather than an empty
    /// argument the renderer would reject.
    fn build(config: &harmonigraph_ui::RenderConfig, ui_state: Option<String>) -> RenderRequest {
        let program = if config.renderer_path.trim().is_empty() {
            default_renderer_path()
        } else {
            std::path::PathBuf::from(config.renderer_path.trim())
        };
        RenderRequest {
            program,
            // Bounced audio is shelved: every render uses the take's own
            // recording as soundtrack and spectrum, aligned by construction — so
            // no --audio replacement and no --align override.
            audio: None,
            align: None,
            ui_state,
            size: config.frame.pixels(config.short_edge),
            // Not forced: the take carries the Video pane's Spectrogram choice
            // and the renderer reads it, so passing `--playhead` here would
            // OR itself over a "Live" the user had picked and the row would
            // control nothing.
            playhead: None,
        }
    }
}

/// The user's home directory, or `.` when `HOME` is unset — the base for the
/// renderer and takes locations below.
fn home_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// Where `update-plugin.sh` installs the renderer, and where the plugin
/// looks when the path setting is left empty. A fixed location beats
/// guessing at the host's working directory or the bundle's own path.
pub fn default_renderer_path() -> std::path::PathBuf {
    home_dir().join("Library/Application Support/Harmonigraph/harmonigraph-offline")
}

/// The audio-thread half: push entries, gated by an atomic the GUI owns.
pub struct Recorder {
    producer: rtrb::Producer<Entry>,
    /// Interleaved input samples, when the take is recording audio too.
    audio: rtrb::Producer<f32>,
    /// Set by the GUI alongside `armed`.
    with_audio: Arc<AtomicBool>,
    armed: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    /// Last value written per parameter, so only changes are recorded.
    /// Reset to NaN on arm so the first block of a take always writes a
    /// full set — a take that inherited "no change since last time" would
    /// replay with default tuning.
    last_params: [f32; ParamKey::ALL.len()],
    was_armed: bool,
    /// Transport position of the previous block, for jump detection.
    last_position: Option<f64>,
    /// Published for the GUI: is the transport actually moving?
    rolling: Arc<AtomicBool>,
    /// Whether this pass has already declared its audio start.
    audio_started: bool,
    /// Set by the GUI when [`RenderTrigger::AtLoopEnd`] is chosen: end the
    /// take on the first loop wrap rather than splitting into another pass.
    stop_at_loop_end: Arc<AtomicBool>,
    /// Published for the GUI: the loop wrapped under AtLoopEnd, so the take is
    /// done — the GUI reads this, stops, and renders the one pass.
    hit_loop_end: Arc<AtomicBool>,
    /// Local latch: once the loop end has been hit, record nothing more until
    /// re-armed, so the wrapped pass never reaches the file.
    finished: bool,
    /// Whether the transport has actually rolled FORWARD since arming. Under
    /// AtLoopEnd a backward jump only counts as the loop end once this is set —
    /// otherwise the very first backward jump (the transport snapping to the
    /// loop/play start when you hit play) would end the take before it recorded
    /// a single block.
    advanced: bool,
}

impl Recorder {
    pub fn is_armed(&mut self) -> bool {
        let armed = self.armed.load(Ordering::Relaxed);
        if armed && !self.was_armed {
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
            self.last_position = None;
            self.audio_started = false;
            self.finished = false;
            self.advanced = false;
            self.hit_loop_end.store(false, Ordering::Relaxed);
        }
        self.was_armed = armed;
        armed
    }

    fn push(&mut self, entry: Entry) {
        if self.producer.push(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note(&mut self, t: f64, channel: u8, note: u8, kind: NoteEventKind) {
        self.push(Entry::Note { t, channel, note, kind });
    }

    pub fn wants_audio(&self) -> bool {
        self.with_audio.load(Ordering::Relaxed)
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
    pub fn audio(&mut self, block: &mut dyn Iterator<Item = f32>, samples: usize) {
        let room = self.audio.slots().min(samples);
        if room < samples {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        if room == 0 {
            return;
        }
        if let Ok(chunk) = self.audio.write_chunk_uninit(room) {
            chunk.fill_from_iter(block.take(room));
        }
    }

    /// Note where the transport is, and answer whether it is rolling —
    /// i.e. whether this block's events belong in the take. Called once
    /// per block with the block's song position and the host's own
    /// `playing` flag.
    ///
    /// **Rolling is the union of "the position advanced" and "the host
    /// says playing", not the flag alone.** During an offline render some
    /// hosts report `playing = false` — nothing is being played, after
    /// all — and trusting the flag would silently record nothing for the
    /// whole export. Conversely a host that reports `playing` before its
    /// position starts moving still gets its first block captured. Only
    /// when both say no does a block get skipped, which is exactly a
    /// parked transport.
    ///
    /// A backward jump means a loop wrapped or the playhead was dragged,
    /// so the take splits. The threshold ignores a host's own jitter
    /// around a loop point; a real wrap is far larger.
    pub fn observe_transport(&mut self, position: f64, playing: bool) -> bool {
        // Once the loop end has ended the take (AtLoopEnd), record nothing more
        // until a fresh arm clears the latch.
        if self.finished {
            return false;
        }
        const BACKWARD_JUMP: f64 = 0.05;
        let rolling = match self.last_position {
            Some(last) if position < last - BACKWARD_JUMP => {
                // A backward jump means the transport looped back, snapped to
                // the loop/play start as playback began, or the playhead was
                // dragged.
                //
                // Under AtLoopEnd a wrap that comes AFTER the take has rolled
                // forward (`advanced`) IS the take: one loop has been recorded,
                // so latch done and tell the GUI to stop + render — WITHOUT
                // pushing NewPass, because the writer opens the next pass
                // eagerly and a split here would leave the empty second file as
                // the one that renders. Keyed off the wrap itself, not the
                // host's loop range: hosts (Bitwig included) don't flag the loop
                // as active to the plugin, so nih-plug's loop_range stays None.
                // The cost is that a manual rewind mid-take also ends it — fine
                // for a mode you opt into specifically for looped recording.
                //
                // But a backward jump BEFORE any forward motion is just the
                // transport arriving at the loop/play start (the playhead was
                // parked past it). Ending there would finish the take with
                // nothing recorded — an empty file and a broken render. So
                // instead begin the pass here: no NewPass (AtLoopEnd only ever
                // wants one file), no end.
                if self.stop_at_loop_end.load(Ordering::Relaxed) {
                    if self.advanced {
                        self.finished = true;
                        self.hit_loop_end.store(true, Ordering::Relaxed);
                        self.last_position = Some(position);
                        self.rolling.store(false, Ordering::Relaxed);
                        return false;
                    }
                    self.last_params = [f32::NAN; ParamKey::ALL.len()];
                    self.audio_started = false;
                    true
                } else {
                    self.push(Entry::NewPass);
                    // A new file starts empty, so every parameter must be
                    // written again or the new pass replays with whatever the
                    // previous one happened to end on. The next pass's audio
                    // also starts somewhere new.
                    self.last_params = [f32::NAN; ParamKey::ALL.len()];
                    self.audio_started = false;
                    true
                }
            }
            Some(last) => {
                if position > last {
                    self.advanced = true;
                }
                playing || position > last
            }
            // Nothing to compare on the first block; the flag is all
            // there is.
            None => playing,
        };
        self.last_position = Some(position);
        self.rolling.store(rolling, Ordering::Relaxed);
        rolling
    }

    /// Record any parameter that moved. Called once per block: nice-plug
    /// is not configured for sample-accurate automation, so block
    /// granularity is exactly as precise as the plugin itself is.
    pub fn params(&mut self, t: f64, values: [f32; ParamKey::ALL.len()]) {
        for (i, value) in values.into_iter().enumerate() {
            if self.last_params[i] != value {
                self.last_params[i] = value;
                self.push(Entry::Param { t, key: i, value });
            }
        }
    }
}

/// The GUI-thread half: start and stop recording, and report what
/// happened. Cloneable so the editor can hold it.
#[derive(Clone)]
pub struct Control {
    commands: mpsc::Sender<Command>,
    armed: Arc<AtomicBool>,
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
    /// Mirror of [`RenderTrigger::AtLoopEnd`] for the audio thread.
    stop_at_loop_end: Arc<AtomicBool>,
    /// Set by the audio thread when a loop wrapped under AtLoopEnd: the take is
    /// done and the GUI should stop + render it.
    hit_loop_end: Arc<AtomicBool>,
    /// How far the background render has got, for the Video pane's bar.
    progress: Arc<Progress>,
}

impl Control {
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Tell the audio thread whether to end the take at the first loop wrap
    /// (the [`RenderTrigger::AtLoopEnd`] mode). Called every GUI frame.
    pub fn set_stop_at_loop_end(&self, on: bool) {
        self.stop_at_loop_end.store(on, Ordering::Relaxed);
    }

    /// Whether the audio thread has reached the loop end and ended the take —
    /// the GUI's cue to stop recording and render the one pass.
    pub fn hit_loop_end(&self) -> bool {
        self.hit_loop_end.load(Ordering::Relaxed)
    }

    /// Whether the audio thread last saw the transport moving.
    pub fn is_rolling(&self) -> bool {
        self.rolling.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> String {
        self.status.lock().clone()
    }

    /// The take most recently finished this session, if any.
    pub fn last_take(&self) -> Option<std::path::PathBuf> {
        self.last_take.lock().clone()
    }

    /// Render the last finished take now, in the background, with `request`
    /// (which carries the current look, bounce, and offset).
    pub fn render_now(&self, request: RenderRequest) {
        match self.last_take() {
            Some(path) => {
                spawn_render(request, path, self.status.clone(), self.progress.clone())
            }
            None => *self.status.lock() = "no take recorded yet to render".into(),
        }
    }

    /// How far the render running in the background has got, or `None` when
    /// none is. Read every GUI frame; see [`Progress`].
    pub fn render_progress(&self) -> Option<harmonigraph_ui::RenderProgress> {
        self.progress.read()
    }

    /// Begin a take. `ui_state` is the persist blob that decides how the
    /// replay will look; `sample_rate` stamps the header. `audio`
    /// records the input bus alongside the notes.
    pub fn start(&self, sample_rate: f32, ui_state: String, audio: bool) {
        if self.is_recording() {
            return;
        }
        let dir = take_dir();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            *self.status.lock() = format!("cannot create {}: {err}", dir.display());
            return;
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("take-{stamp}.{}", harmonigraph_take::EXTENSION));
        let header = harmonigraph_take::Header {
            sample_rate,
            ui_state: Some(ui_state),
            source: "harmonigraph".into(),
            ..Default::default()
        };

        self.dropped.store(0, Ordering::Relaxed);
        self.with_audio.store(audio, Ordering::Relaxed);
        let spec = audio.then_some(AudioSpec { sample_rate, channels: 2 });
        if self.commands.send(Command::Start(Box::new(header), path, spec)).is_err() {
            *self.status.lock() = "take writer thread is gone".into();
            return;
        }
        self.recording.store(true, Ordering::Relaxed);
        self.rolling.store(false, Ordering::Relaxed);
        // Clear a previous take's loop-end latch so it can't end this one before
        // the transport even rolls. The audio thread also clears it on arm.
        self.hit_loop_end.store(false, Ordering::Relaxed);
        self.armed.store(true, Ordering::Relaxed);
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
        // Disarm first, so the audio thread stops pushing before the
        // writer is told to close.
        self.armed.store(false, Ordering::Relaxed);
        self.with_audio.store(false, Ordering::Relaxed);
        let _ = self.commands.send(Command::Stop(render.map(Box::new)));
        self.recording.store(false, Ordering::Relaxed);
    }

    /// Called each GUI frame while recording, so the status line reflects
    /// what the audio thread is actually doing.
    pub fn tick(&self, rolling: bool, events: u64) {
        if !self.is_recording() {
            return;
        }
        let dropped = self.dropped.load(Ordering::Relaxed);
        *self.status.lock() = if dropped > 0 {
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

/// Where takes go. `LATTICE_TAKE_DIR` overrides; the default is a fixed,
/// findable place, because a DAW's environment usually has neither the
/// variable nor a useful working directory.
fn take_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("LATTICE_TAKE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    home_dir().join("Music").join("Harmonigraph Takes")
}

/// Build the ring and start the writer thread. Called once, from the
/// plugin's `Default`, so neither arming nor disarming ever has to touch
/// the audio thread's producer.
pub fn channel() -> (Recorder, Control) {
    let (producer, mut consumer) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    let (audio_producer, mut audio_consumer) = rtrb::RingBuffer::new(AUDIO_RING_CAPACITY);
    let (commands, orders) = mpsc::channel::<Command>();
    let armed = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let recording = Arc::new(AtomicBool::new(false));
    let rolling = Arc::new(AtomicBool::new(false));
    let with_audio = Arc::new(AtomicBool::new(false));
    let stop_at_loop_end = Arc::new(AtomicBool::new(false));
    let hit_loop_end = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(String::new()));
    let last_take = Arc::new(Mutex::new(None));
    let progress = Arc::new(Progress::default());

    let thread_status = status.clone();
    let thread_last_take = last_take.clone();
    let thread_progress = progress.clone();
    let _ = std::thread::Builder::new()
        .name("harmonigraph-take-writer".into())
        .spawn(move || {
            let mut open: Option<Open> = None;
            loop {
                match orders.try_recv() {
                    Ok(Command::Start(header, path, spec)) => {
                        open = Open::create(*header, path, 1, spec, &thread_status);
                    }
                    Ok(Command::Stop(render)) => {
                        // Drain what the audio thread already queued
                        // before closing, or the tail of the take is lost.
                        drain(&mut consumer, &mut open, &thread_status);
                        let finished = open.take().map(|o| o.finish());
                        if let Some(path) = &finished {
                            *thread_last_take.lock() = Some(path.clone());
                        }
                        if let (Some(path), Some(render)) = (finished, render) {
                            spawn_render(
                                *render,
                                path,
                                thread_status.clone(),
                                thread_progress.clone(),
                            );
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        drain(&mut consumer, &mut open, &thread_status);
                        return;
                    }
                }
                let had_records = drain(&mut consumer, &mut open, &thread_status);
                let had_audio = drain_audio(&mut audio_consumer, &mut open);
                if !had_records && !had_audio {
                    std::thread::sleep(DRAIN_IDLE);
                }
            }
        });

    (
        Recorder {
            producer,
            armed: armed.clone(),
            dropped: dropped.clone(),
            last_params: [f32::NAN; ParamKey::ALL.len()],
            was_armed: false,
            last_position: None,
            rolling: rolling.clone(),
            audio_started: false,
            audio: audio_producer,
            with_audio: with_audio.clone(),
            stop_at_loop_end: stop_at_loop_end.clone(),
            hit_loop_end: hit_loop_end.clone(),
            finished: false,
            advanced: false,
        },
        Control {
            commands,
            armed,
            dropped,
            status,
            last_take,
            recording,
            rolling,
            with_audio,
            stop_at_loop_end,
            hit_loop_end,
            progress,
        },
    )
}

/// Move queued audio into the WAV. Separate from [`drain`] because the
/// volume is different by orders of magnitude: one ring read per pass
/// rather than per sample.
fn drain_audio(consumer: &mut rtrb::Consumer<f32>, open: &mut Option<Open>) -> bool {
    let available = consumer.slots();
    if available == 0 {
        return false;
    }
    let Ok(chunk) = consumer.read_chunk(available) else { return false };
    if let Some(audio) = open.as_mut().and_then(|o| o.audio.as_mut()) {
        let (first, second) = chunk.as_slices();
        let _ = audio.write(first);
        let _ = audio.write(second);
    }
    chunk.commit_all();
    true
}

/// The file currently being written, and what it takes to open the next
/// one when the transport loops.
struct Open {
    writer: harmonigraph_take::Writer,
    header: harmonigraph_take::Header,
    /// The first pass's path; later passes append `-2`, `-3`, ...
    base: std::path::PathBuf,
    pass: u32,
    /// The WAV recorded beside this pass, if audio was asked for.
    audio: Option<harmonigraph_take::WavWriter>,
    spec: Option<AudioSpec>,
}

impl Open {
    fn create(
        mut header: harmonigraph_take::Header,
        base: std::path::PathBuf,
        pass: u32,
        spec: Option<AudioSpec>,
        status: &Mutex<String>,
    ) -> Option<Open> {
        let path = Self::path_for(&base, pass);

        // The WAV opens first, so its name can go in the take's header —
        // which is the take's first line and cannot be revised later.
        let audio = spec.and_then(|spec| {
            let wav = path.with_extension("wav");
            match harmonigraph_take::WavWriter::create(&wav, spec.sample_rate, spec.channels) {
                Ok(writer) => {
                    header.audio_file =
                        wav.file_name().and_then(|n| n.to_str()).map(str::to_owned);
                    Some(writer)
                }
                Err(err) => {
                    *status.lock() = format!("cannot write {}: {err}", wav.display());
                    None
                }
            }
        });

        match harmonigraph_take::Writer::create(&path, &header) {
            Ok(writer) => {
                *status.lock() = if pass <= 1 {
                    format!("recording to {}", path.display())
                } else {
                    format!("pass {pass} -> {}", path.display())
                };
                Some(Open { writer, header, base, pass, audio, spec })
            }
            Err(err) => {
                *status.lock() = format!("cannot write {}: {err}", path.display());
                None
            }
        }
    }

    fn path_for(base: &std::path::Path, pass: u32) -> std::path::PathBuf {
        if pass <= 1 {
            base.to_path_buf()
        } else {
            let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
            base.with_file_name(format!("{stem}-{pass}.{}", harmonigraph_take::EXTENSION))
        }
    }

    /// Close both files and hand back the take's path.
    fn finish(self) -> std::path::PathBuf {
        let path = self.path();
        if let Some(audio) = self.audio {
            let _ = audio.finish();
        }
        drop(self.writer);
        path
    }

    /// The file this pass is writing to.
    fn path(&self) -> std::path::PathBuf {
        Self::path_for(&self.base, self.pass)
    }

    /// Close this pass's files and open the next pass's.
    fn next_pass(self, status: &Mutex<String>) -> Option<Open> {
        let Open { mut header, base, pass, writer, audio, spec } = self;
        if let Some(audio) = audio {
            let _ = audio.finish();
        }
        drop(writer);
        // Each pass records its own audio from its own start, so the
        // previous pass's alignment must not be inherited.
        header.audio_start = None;
        Open::create(header, base, pass + 1, spec, status)
    }
}

/// Move everything queued into the writer (discarding it if none is
/// open). Returns whether anything was there.
fn drain(
    consumer: &mut rtrb::Consumer<Entry>,
    open: &mut Option<Open>,
    status: &Mutex<String>,
) -> bool {
    let mut any = false;
    while let Ok(entry) = consumer.pop() {
        any = true;
        if matches!(entry, Entry::NewPass) {
            if let Some(current) = open.take() {
                *open = current.next_pass(status);
            }
            continue;
        }
        if let Entry::AudioStart(t) = entry {
            // Rewrite the header now that the WAV's alignment is known.
            // The format is line-oriented and the reader takes the LAST
            // Header record, so a corrected one simply supersedes the
            // first — no seeking, no fixed-width fields.
            if let Some(current) = open.as_mut() {
                current.header.audio_start = Some(t);
                let header = current.header.clone();
                let _ = current.writer.write(&harmonigraph_take::Record::Header(header));
            }
            continue;
        }
        let Some(writer) = open.as_mut().map(|o| &mut o.writer) else { continue };
        let _ = match entry {
            Entry::Note { t, channel, note, kind } => writer.note(harmonigraph_take::NoteRecord {
                t,
                channel,
                note,
                kind: match kind {
                    NoteEventKind::On { velocity } => harmonigraph_take::NoteKind::On { velocity },
                    NoteEventKind::Off => harmonigraph_take::NoteKind::Off,
                    NoteEventKind::Tuning { semitones } => {
                        harmonigraph_take::NoteKind::Tuning { semitones }
                    }
                    NoteEventKind::AllOff => harmonigraph_take::NoteKind::AllOff,
                },
            }),
            Entry::Param { t, key, value } => writer.param(harmonigraph_take::ParamRecord {
                t,
                id: ParamKey::ALL[key].id().to_string(),
                value,
            }),
            // Both handled above; the writer never sees them.
            Entry::NewPass | Entry::AudioStart(_) => Ok(()),
        };
    }
    any
}

/// Frames done and frames planned for the render(s) in flight, published by
/// the thread following the renderer's stderr and read by the GUI each frame.
///
/// `in_flight` is a COUNT rather than a flag because two renders can overlap —
/// "Render now" pressed while an auto-render is still going — and a flag would
/// have the first one to finish clear the bar out from under the second.
#[derive(Default)]
struct Progress {
    done: AtomicU64,
    /// 0 until the renderer announces how many frames it is composing.
    total: AtomicU64,
    in_flight: AtomicU64,
}

impl Progress {
    /// A render is starting: clear the last one's counts, then join the flight.
    /// Release-ordered against [`read`](Self::read)'s acquire, so a bar can
    /// never appear over the previous render's numbers.
    fn begin(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.in_flight.fetch_add(1, Ordering::Release);
    }

    fn end(&self) {
        self.in_flight.fetch_sub(1, Ordering::Release);
    }

    fn read(&self) -> Option<harmonigraph_ui::RenderProgress> {
        (self.in_flight.load(Ordering::Acquire) > 0).then(|| harmonigraph_ui::RenderProgress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        })
    }
}

/// What one segment of the renderer's stderr says about how far it has got.
enum Report {
    /// How many frames the render is composing, from the line it opens with.
    Total(u64),
    /// Frames written of frames planned, from the counter it rewrites as it
    /// goes.
    Frames { done: u64, total: u64 },
}

/// Read a progress report out of one line of `harmonigraph-offline`'s stderr,
/// if that is what it is.
///
/// **This parses another binary's human-readable output**, which is a contract
/// worth naming: the renderer opens with `... -> 5400 frames at 60 fps ...`
/// and then rewrites `  120/5400 frames (2%)` in place. Both are matched here
/// off the count that precedes ` frames`. The alternative — a `--progress`
/// flag emitting something machine-shaped — fails far worse when the installed
/// renderer is older than the plugin: it would reject the unknown flag and
/// render nothing at all, where a format this no longer recognizes just leaves
/// the bar empty and everything else working.
fn parse_report(segment: &str) -> Option<Report> {
    // The count is the last token before ` frames`, so a take path with a
    // slash (or a space) in it has nothing to say here.
    let token = segment.split_once(" frames")?.0.split_whitespace().next_back()?;
    match token.split_once('/') {
        Some((done, total)) => {
            Some(Report::Frames { done: done.parse().ok()?, total: total.parse().ok()? })
        }
        None => Some(Report::Total(token.parse().ok()?)),
    }
}

/// Longest stderr run with no separator in it that is worth keeping. Past this
/// the segment is not a line the renderer meant to print, and buffering it
/// only costs memory.
const STDERR_SEGMENT_CAP: usize = 8 * 1024;

/// Follow the renderer's stderr to its end, publishing progress as it arrives,
/// and hand back the last line that ISN'T progress — which is the one worth
/// showing if the render then fails.
///
/// Read continuously rather than collected at the end (what `Command::output`
/// would do) for two reasons: a frame counter is only useful while the render
/// is still running, and the renderer — plus the ffmpeg it inherits this pipe
/// to — would eventually block on a pipe nobody is draining.
///
/// Split on `\r` as well as `\n`: the counter is REWRITTEN in place on one
/// terminal line, so newlines alone would deliver the whole run of it as a
/// single line, once, at the end.
fn follow(mut stderr: impl std::io::Read, progress: &Progress) -> String {
    let mut buffer = [0u8; 4096];
    let mut segment: Vec<u8> = Vec::new();
    let mut last = String::new();
    let take = |segment: &mut Vec<u8>, last: &mut String| {
        let text = String::from_utf8_lossy(segment);
        let text = text.trim();
        match parse_report(text) {
            Some(Report::Frames { done, total }) => {
                progress.done.store(done, Ordering::Relaxed);
                progress.total.store(total, Ordering::Relaxed);
            }
            Some(Report::Total(total)) => progress.total.store(total, Ordering::Relaxed),
            // Diagnostics, warnings, the renderer's own error: keep the last
            // one for the status line.
            None if !text.is_empty() => *last = text.to_owned(),
            None => {}
        }
        segment.clear();
    };
    loop {
        let read = match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            // A signal arriving mid-read is not the end of the render, and
            // treating it as one would freeze the bar and truncate the
            // diagnostics for a render still going perfectly well.
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &byte in &buffer[..read] {
            if byte == b'\r' || byte == b'\n' {
                take(&mut segment, &mut last);
            } else {
                segment.push(byte);
                if segment.len() >= STDERR_SEGMENT_CAP {
                    take(&mut segment, &mut last);
                }
            }
        }
    }
    // Whatever the renderer left unterminated on its way out.
    take(&mut segment, &mut last);
    last
}

/// Run the renderer on the finished take, on a thread of its own so a
/// long render neither blocks the writer nor the DAW. The video lands
/// next to the take.
fn spawn_render(
    request: RenderRequest,
    take_path: std::path::PathBuf,
    status: Arc<Mutex<String>>,
    progress: Arc<Progress>,
) {
    let _ = std::thread::Builder::new()
        .name("harmonigraph-take-render".into())
        .spawn(move || {
            let out = take_path.with_extension("mp4");
            // A "Render now" carries the current look as a persist blob; write
            // it beside the take and pass --ui-state so post-record settings
            // override the take's record-time snapshot. Removed after the run.
            let ui_state_file = request.ui_state.as_ref().and_then(|blob| {
                let path = take_path.with_extension("rendernow.ron");
                std::fs::write(&path, blob).ok().map(|()| path)
            });

            let mut command = std::process::Command::new(&request.program);
            command.arg(&take_path).arg("--out").arg(&out);
            if let Some(audio) = &request.audio {
                command.arg("--audio").arg(audio);
            }
            if let Some(align) = &request.align {
                command.arg("--align").arg(align);
            }
            if let Some(file) = &ui_state_file {
                command.arg("--ui-state").arg(file);
            }
            let [w, h] = request.size;
            command.arg("--size").arg(format!("{w}x{h}"));
            // Only when something forces it. The renderer turns the whole-song
            // playhead on for `--playhead` OR the take's own recorded setting,
            // so a flag passed unconditionally here is one the Video pane's
            // Spectrogram row can never turn back off — which is exactly what
            // it was, and why picking "Live" did nothing. Left alone, the
            // take's setting is the whole answer, in both directions.
            if request.playhead == Some(true) {
                command.arg("--playhead");
            }

            // stdout is the renderer's `--dump-layout` channel and nothing
            // else; stderr carries everything this cares about, so pipe that
            // one and follow it.
            command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());

            *status.lock() = format!("rendering {}...", out.display());
            let spawned = command.spawn();
            let cleanup = || {
                if let Some(file) = &ui_state_file {
                    let _ = std::fs::remove_file(file);
                }
            };
            let mut child = match spawned {
                Ok(child) => child,
                Err(err) => {
                    cleanup();
                    *status.lock() = format!(
                        "could not run {}: {err} — check the Renderer path",
                        request.program.display()
                    );
                    return;
                }
            };

            progress.begin();
            let last = child.stderr.take().map(|pipe| follow(pipe, &progress)).unwrap_or_default();
            let result = child.wait();
            progress.end();
            cleanup();

            match result {
                Ok(exit) if exit.success() => {
                    *status.lock() = format!("rendered {}", out.display());
                }
                // The renderer's own diagnostics are far more useful than the
                // exit code, and this is the only place a plugin user will
                // ever see them.
                Ok(_) => *status.lock() = format!("render failed: {last}"),
                Err(err) => *status.lock() = format!("render failed: {err}"),
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_ui::RenderConfig;

    #[test]
    fn a_finished_take_always_renders() {
        // Auto-render is not gated: stopping a take always kicks off a
        // render (recorded audio + playhead), so from_config is always `Some`.
        let config = RenderConfig { auto_render: false, ..Default::default() };
        assert!(RenderRequest::from_config(&config).is_some());
    }

    /// A blank renderer path must fall back to the default rather than becoming
    /// an empty argument the renderer would reject.
    #[test]
    fn blank_settings_fall_back_rather_than_passing_empty_arguments() {
        let config = RenderConfig { renderer_path: "  ".into(), ..Default::default() };
        let request = RenderRequest::from_config(&config).unwrap();
        assert_eq!(request.program, default_renderer_path());
        assert_eq!(request.audio, None);
        assert_eq!(request.size, [1920, 1080]);
    }

    /// The pane's Aspect and Resolution are the whole of what sizes the video.
    #[test]
    fn the_frame_sizes_the_render() {
        let portrait_4k = RenderConfig {
            frame: harmonigraph_ui::RenderFrame {
                aspect_w: 9,
                aspect_h: 16,
                ..Default::default()
            },
            short_edge: 2160,
            ..Default::default()
        };
        let request = RenderRequest::from_config(&portrait_4k).unwrap();
        assert_eq!(request.size, [2160, 3840]);
    }

    /// The renderer is never told to use the whole-song playhead.
    ///
    /// It ORs `--playhead` with the take's own recorded setting, so a flag
    /// passed here can only add. Passing it unconditionally — which is what
    /// this used to do — made the Video pane's "Live" choice unreachable: the
    /// pane wrote `playhead: false` into the take and the flag turned it
    /// straight back on. Leaving it unset is what lets the row decide both
    /// ways, so this holds the request to saying nothing.
    #[test]
    fn the_take_decides_the_spectrogram_not_a_forced_flag() {
        for playhead in [false, true] {
            let config = RenderConfig { playhead, ..Default::default() };
            assert_eq!(RenderRequest::from_config(&config).unwrap().playhead, None);
            let now = RenderRequest::render_now(&config, "(dummy)".into());
            assert_eq!(now.playhead, None, "Render now must not force it either");
        }
    }

    /// Verbatim shapes from `harmonigraph-offline`'s stderr — the opening line
    /// that announces the frame count, and the counter it rewrites as it goes.
    #[test]
    fn the_renderers_own_output_is_what_puts_numbers_on_the_bar() {
        let header = "/Users/yan/Music/Harmonigraph Takes/take-1.take: 16.5s of events \
                      -> 990 frames at 60 fps, 1920x1080 @ 1.50x -> take-1.mp4";
        assert!(matches!(parse_report(header), Some(Report::Total(990))));
        assert!(matches!(
            parse_report("  120/990 frames (12%)"),
            Some(Report::Frames { done: 120, total: 990 })
        ));

        // Everything else is a diagnostic, and belongs in the status line
        // instead of quietly moving the bar. A path with a slash in it is the
        // one that could be mistaken for a done/total pair.
        assert!(parse_report("harmonigraph-offline: no take file given").is_none());
        assert!(parse_report("note: no scratch recording, assuming take zero").is_none());
        assert!(parse_report("/Users/yan/odd frames/take.take: could not be read").is_none());
        assert!(parse_report("").is_none());
    }

    /// A whole run of a real render's stderr, captured verbatim from
    /// `harmonigraph-offline` (the paths shortened, nothing else): the opening
    /// line, the counter rewritten in place four times, and the closing line.
    ///
    /// The `\r`s are the point. They mean the counter is one terminal line
    /// being overwritten, so splitting on newlines alone delivers the whole run
    /// of it as a single line, once, at the end — a progress bar that fills
    /// only when the render is already over.
    const REAL_RENDER_STDERR: &str = "probe.take: 1.6s of events -> 108 frames at 30 fps, \
         320x180 @ 1.00x -> probe.rgba\n\
         \r  30/108 frames (28%)\r  60/108 frames (56%)\r  90/108 frames (83%)\
         \r  108/108 frames (100%)\n\
         done: 108 frames -> probe.rgba\n";

    #[test]
    fn the_rewritten_counter_reaches_the_bar_as_the_render_goes() {
        let progress = Progress::default();
        progress.begin();
        follow(REAL_RENDER_STDERR.as_bytes(), &progress);
        // 108, not 30: the closing `done: 108 frames` line names a total and
        // says nothing about frames written, so it must not reset the count
        // the render finished on either.
        assert_eq!(
            progress.read(),
            Some(harmonigraph_ui::RenderProgress { done: 108, total: 108 })
        );

        progress.end();
        assert_eq!(progress.read(), None, "no bar once nothing is rendering");
    }

    /// What the status line shows when a render fails is the renderer's own
    /// last word — which means the counters have to be passed over on the way
    /// to it, however recently they were printed.
    #[test]
    fn a_failed_renders_last_word_is_kept_over_the_counters() {
        let stream = "take.take: 5.0s of events -> 300 frames at 60 fps, 640x360 @ 1.00x \
                      -> take.mp4\n\
                      \r  30/300 frames (10%)\r  60/300 frames (20%)\n\
                      harmonigraph-offline: ffmpeg exited with status 1\n";
        let progress = Progress::default();
        progress.begin();
        assert_eq!(
            follow(stream.as_bytes(), &progress),
            "harmonigraph-offline: ffmpeg exited with status 1"
        );
    }

    /// A render killed mid-frame leaves its last counter unterminated. It is
    /// still the truest thing known about how far it got, so the tail of the
    /// stream counts even without a separator to end it.
    #[test]
    fn an_unterminated_last_counter_still_counts() {
        let progress = Progress::default();
        progress.begin();
        follow("x.take: -> 300 frames at 60 fps\n\r  240/300 frames (80%)".as_bytes(), &progress);
        assert_eq!(
            progress.read(),
            Some(harmonigraph_ui::RenderProgress { done: 240, total: 300 })
        );
    }

    /// Two renders can overlap — "Render now" pressed while the auto-render
    /// from a just-finished take is still going. The first to end must not
    /// take the bar away from the one still running.
    #[test]
    fn overlapping_renders_keep_the_bar_until_the_last_one_ends() {
        let progress = Progress::default();
        progress.begin();
        progress.begin();
        progress.end();
        assert!(progress.read().is_some(), "one render is still in flight");
        progress.end();
        assert_eq!(progress.read(), None);
    }

    #[test]
    fn the_default_renderer_path_is_where_update_plugin_installs_it() {
        let path = default_renderer_path();
        assert!(path.ends_with("Harmonigraph/harmonigraph-offline"), "{path:?}");
    }

    #[test]
    fn at_loop_end_ends_the_take_on_the_first_wrap_without_splitting() {
        let (mut rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        ctrl.set_stop_at_loop_end(true);
        assert!(rec.is_armed(), "arming clears last_position and the done latch");

        // One loop's worth of forward motion.
        assert!(rec.observe_transport(0.0, true));
        assert!(rec.observe_transport(1.0, true));
        assert!(rec.observe_transport(2.0, true));
        assert!(!ctrl.hit_loop_end(), "still mid-loop");

        // The transport wraps back to the loop start: end the take here, and
        // signal the GUI — do NOT keep rolling into a second pass.
        assert!(!rec.observe_transport(0.0, true), "the wrap ends the take");
        assert!(ctrl.hit_loop_end(), "GUI is told to stop and render the pass");

        // Latched: nothing rolls again until a fresh arm.
        assert!(!rec.observe_transport(1.0, true));
    }

    #[test]
    fn a_wrap_without_at_loop_end_splits_and_keeps_rolling() {
        let (mut rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        // stop_at_loop_end stays off — the default OnDisarm/looping behavior.
        assert!(rec.is_armed());
        assert!(rec.observe_transport(0.0, true));
        assert!(rec.observe_transport(2.0, true));
        // The wrap starts a new pass but keeps recording, as before.
        assert!(rec.observe_transport(0.0, true), "a normal loop keeps going");
        assert!(!ctrl.hit_loop_end());
    }

    #[test]
    fn re_arming_clears_the_loop_end_latch() {
        let (mut rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        ctrl.set_stop_at_loop_end(true);
        assert!(rec.is_armed());
        assert!(rec.observe_transport(0.0, true));
        assert!(rec.observe_transport(2.0, true));
        assert!(!rec.observe_transport(0.0, true), "the wrap ends the first take");
        assert!(ctrl.hit_loop_end());

        // Disarm, then re-arm: the done latch and the loop-end flag clear, so
        // the next take records from scratch rather than starting finished.
        ctrl.armed.store(false, Ordering::Relaxed);
        assert!(!rec.is_armed());
        ctrl.armed.store(true, Ordering::Relaxed);
        assert!(rec.is_armed(), "re-arm");
        assert!(!ctrl.hit_loop_end(), "the latch cleared on re-arm");
        assert!(rec.observe_transport(0.0, true), "records again");
    }

    #[test]
    fn at_loop_end_ignores_the_jump_to_the_loop_start_when_playback_begins() {
        let (mut rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        ctrl.set_stop_at_loop_end(true);
        assert!(rec.is_armed());

        // Playhead parked PAST the loop start, transport stopped.
        assert!(!rec.observe_transport(5.0, false), "parked, not rolling");

        // Hit play: the transport snaps back to the loop start. This is the bug
        // that produced empty takes — it must NOT end the take, because nothing
        // has been recorded yet. It begins the pass instead.
        assert!(rec.observe_transport(0.0, true), "the jump-to-start begins the pass");
        assert!(!ctrl.hit_loop_end(), "the initial jump is not a loop end");

        // Now it rolls forward through the loop...
        assert!(rec.observe_transport(1.0, true));
        assert!(rec.observe_transport(2.0, true));

        // ...and THIS wrap, after real forward motion, is the loop end.
        assert!(!rec.observe_transport(0.0, true), "the real wrap ends the take");
        assert!(ctrl.hit_loop_end());
    }
}
