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

/// A take's WAV is always stereo, whatever the host's input bus carries. The
/// spec handed to the writer, the reservation in [`Recorder::audio`] and the
/// interleaving in `process` all read this one number: if any of them
/// disagrees, the header describes frames the data does not have, and nothing
/// downstream checks.
pub(crate) const TAKE_CHANNELS: usize = 2;

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
    /// look — set for "Re-render take" so post-record settings reach the video;
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

    /// Build a request for an explicit "Re-render take": always built, and it
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
        let room = crate::interleaved_reservation(
            self.audio.slots(),
            samples / TAKE_CHANNELS,
            TAKE_CHANNELS,
        );
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
    /// Shared by every render this Control starts, so a new request cancels
    /// the one in flight rather than racing it. See [`RenderControl`].
    render: Arc<RenderControl>,
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
                spawn_render(
                    request,
                    path,
                    self.status.clone(),
                    self.progress.clone(),
                    self.render.clone(),
                )
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
        let spec = audio.then_some(AudioSpec { sample_rate, channels: TAKE_CHANNELS as u16 });
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
    let render = Arc::new(RenderControl::default());

    let thread_status = status.clone();
    let thread_last_take = last_take.clone();
    let thread_progress = progress.clone();
    let thread_render = render.clone();
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
                                thread_render.clone(),
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
            render,
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
/// `in_flight` is a COUNT rather than a flag, though [`RenderControl`] now
/// serialises renders so it only ever reaches one. It is the cheaper way to be
/// wrong: a flag cleared by whichever render finished first would blank the
/// bar out from under a running one, and counting cannot.
#[derive(Default)]
struct Progress {
    done: AtomicU64,
    /// 0 until the renderer announces how many frames it is composing.
    total: AtomicU64,
    in_flight: AtomicU64,
}

/// What a render in flight can be reached by, so the next request can cancel
/// it instead of running alongside it.
///
/// Two renders of one take write one video, and there is no useful way to
/// merge that — so a second request is treated as a correction of the first,
/// not an addition to it. That is nearly always what it is: the same take,
/// with a setting changed since.
#[derive(Default)]
struct RenderControl {
    /// Bumped by every request, so no two runs share a number — which is what
    /// keeps each run's partial output under a name of its own.
    generation: AtomicU64,
    /// The newest generation claimed for each take.
    ///
    /// Keyed by take, not global, because "superseded" is a claim about ONE
    /// video. Every finished take renders and every take is its own file, so
    /// consecutive recordings are two requests that want two different videos,
    /// and a global newest-wins would have the second silently discard the
    /// first (see
    /// [`a_render_of_another_take_waits_rather_than_replacing_this_one`]).
    /// Only a second request naming the SAME take is the same video twice,
    /// which is the case "Re-render take" makes.
    ///
    /// An entry is dropped when its run finishes, so this holds one key per
    /// take being rendered rather than one per take of the session.
    claims: Mutex<std::collections::HashMap<std::path::PathBuf, u64>>,
    /// The renderer process now running and the take it is rendering, for a
    /// later request to kill if it is about the same video.
    ///
    /// The child lives here rather than on its own thread's stack precisely so
    /// another thread can reach it; the render thread borrows it back to reap
    /// it once its stderr has closed.
    child: Mutex<Option<InFlight>>,
    /// Held for the length of a render, so renders run one at a time — a
    /// replacement waits for the cancelled run's process to be reaped rather
    /// than merely asked to stop, and a render of another take waits its turn
    /// instead of putting a second ffmpeg beside the first.
    running: Mutex<()>,
}

/// The renderer process now running, and which take's video it is producing.
struct InFlight {
    take: std::path::PathBuf,
    child: std::process::Child,
}

/// Hands a take's claim back when its run ends, by whichever of the render
/// thread's several exits it takes — including the two that stand down before
/// spawning anything. A claim left behind would make the next request for that
/// take look superseded before it started.
struct Claim<'a> {
    control: &'a RenderControl,
    take: &'a std::path::Path,
    generation: u64,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.control.release(self.take, self.generation);
    }
}

impl RenderControl {
    /// Claim the flight for `take`, superseding any earlier claim on it.
    fn claim(&self, take: &std::path::Path) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.claims.lock().insert(take.to_path_buf(), generation);
        generation
    }

    /// Give up `generation`'s claim on `take`, if it is still the standing one.
    /// A newer request has already replaced it otherwise, and dropping that
    /// would tell the newer run it had been superseded by nobody.
    fn release(&self, take: &std::path::Path, generation: u64) {
        let mut claims = self.claims.lock();
        if claims.get(take) == Some(&generation) {
            claims.remove(take);
        }
    }

    /// Kill the render in flight if it is rendering `take`. Returns having
    /// *asked*: the process is reaped by its own thread, which the replacement
    /// waits for through [`running`](Self::running).
    ///
    /// A render of some other take is left alone — it is producing a different
    /// video, and nothing about this request says that one is unwanted.
    fn cancel_in_flight(&self, take: &std::path::Path) {
        if let Some(flight) = self.child.lock().as_mut() {
            if flight.take == take {
                let _ = flight.child.kill();
            }
        }
    }

    /// Whether a newer request for the SAME take has arrived since
    /// `generation` claimed it.
    fn superseded(&self, take: &std::path::Path, generation: u64) -> bool {
        self.claims.lock().get(take) != Some(&generation)
    }
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
    control: Arc<RenderControl>,
) {
    // Claim the flight before spawning anything: a second request for THIS
    // take cancels the one running rather than joining it. Two renders of one
    // take write one video, and the newer request is always the wanted one —
    // it is the same take with settings changed since.
    //
    // A request for another take makes no such claim. It queues on `running`
    // instead and renders when its turn comes.
    let generation = control.claim(&take_path);
    // On the CALLER's thread, so the cancellation lands before the replacement
    // starts queueing behind a run that now has no reason to finish.
    control.cancel_in_flight(&take_path);

    let _ = std::thread::Builder::new()
        .name("harmonigraph-take-render".into())
        .spawn(move || {
            let _claim = Claim { control: &control, take: &take_path, generation };
            // Wait out the run being cancelled, so its process is reaped
            // before this one starts. Held for the whole render, which is what
            // makes "one render at a time" true rather than hoped for — and
            // what a render of ANOTHER take queues on instead of cancelling.
            let _flight = control.running.lock();
            if control.superseded(&take_path, generation) {
                // Another request arrived while this one queued. It is already
                // waiting on the same lock, and rendering here would only be
                // work to throw away.
                return;
            }

            let out = take_path.with_extension("mp4");
            // Written under a name of this run's own, and moved onto `out`
            // only once it has succeeded.
            //
            // Killing the renderer does not kill the ffmpeg it is piping to —
            // that is a grandchild, and it outlives the kill by however long
            // finalizing takes. Sharing one output path with it is how a
            // cancelled render corrupts the video that replaces it. A path per
            // run means the straggler writes somewhere nobody is reading, and
            // the file at `out` is only ever produced whole, by rename.
            let partial = take_path.with_extension(format!("rendering-{generation}.mp4"));
            // A "Re-render take" carries the current look as a persist blob; write
            // it beside the take and pass --ui-state so post-record settings
            // override the take's record-time snapshot. Per-run for the same
            // reason as `partial`, and removed after the run.
            let ui_state_file = request.ui_state.as_ref().and_then(|blob| {
                let path = take_path.with_extension(format!("rendernow-{generation}.ron"));
                std::fs::write(&path, blob).ok().map(|()| path)
            });

            let mut command = std::process::Command::new(&request.program);
            command.arg(&take_path).arg("--out").arg(&partial);
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
                let _ = std::fs::remove_file(&partial);
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
            let stderr = child.stderr.take();
            // Hand the process over so a later request can reach it. Under the
            // same lock a canceller takes, and re-checking the generation
            // inside it: a request that arrived between the spawn and here
            // found no child to kill, so this is where that one gets killed
            // instead of running to completion unnoticed.
            {
                let mut in_flight = control.child.lock();
                if control.superseded(&take_path, generation) {
                    let _ = child.kill();
                }
                *in_flight = Some(InFlight { take: take_path.clone(), child });
            }
            // Ends at EOF on the pipe, which a kill brings about immediately.
            let last = stderr.map(|pipe| follow(pipe, &progress)).unwrap_or_default();
            let result = match control.child.lock().take() {
                Some(mut flight) => flight.child.wait(),
                // Unreachable in practice: nothing else takes the child, only
                // kills it. Reported rather than unwrapped, since a render
                // thread panicking in a DAW is not worth the tidier code.
                None => Err(std::io::Error::other("the render process went missing")),
            };
            progress.end();

            // A cancelled render has nothing to say: its replacement is
            // already running and the failure is one we caused on purpose.
            // Its partial output goes, and the status line stays the new
            // render's.
            if control.superseded(&take_path, generation) {
                cleanup();
                return;
            }

            match result {
                Ok(exit) if exit.success() => {
                    // Whole, and only now under the name anything else reads.
                    match std::fs::rename(&partial, &out) {
                        Ok(()) => *status.lock() = format!("rendered {}", out.display()),
                        Err(err) => {
                            *status.lock() =
                                format!("rendered, but could not move into place: {err}")
                        }
                    }
                    if let Some(file) = &ui_state_file {
                        let _ = std::fs::remove_file(file);
                    }
                }
                // The renderer's own diagnostics are far more useful than the
                // exit code, and this is the only place a plugin user will
                // ever see them.
                Ok(_) => {
                    cleanup();
                    *status.lock() = format!("render failed: {last}");
                }
                Err(err) => {
                    cleanup();
                    *status.lock() = format!("render failed: {err}");
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_ui::RenderConfig;

    /// A [`Recorder`] whose rings the test keeps the far end of.
    ///
    /// [`channel`] hands both consumers to the writer thread, which drains them
    /// within `DRAIN_IDLE` — so a test that asked what the audio thread had
    /// actually PUSHED would be racing that thread for the answer, and would
    /// usually lose. Holding the consumers here is what makes the take's
    /// CONTENTS assertable rather than only the counters beside them, and those
    /// are different claims: `audio` can reserve correctly, count nothing
    /// dropped, and write not one sample.
    struct Bench {
        rec: Recorder,
        entries: rtrb::Consumer<Entry>,
        samples: rtrb::Consumer<f32>,
        armed: Arc<AtomicBool>,
        stop_at_loop_end: Arc<AtomicBool>,
        hit_loop_end: Arc<AtomicBool>,
        dropped: Arc<AtomicU64>,
    }

    impl Bench {
        fn new() -> Bench {
            let (producer, entries) = rtrb::RingBuffer::new(1024);
            let (audio, samples) = rtrb::RingBuffer::new(1024);
            let armed = Arc::new(AtomicBool::new(false));
            let stop_at_loop_end = Arc::new(AtomicBool::new(false));
            let hit_loop_end = Arc::new(AtomicBool::new(false));
            let dropped = Arc::new(AtomicU64::new(0));
            Bench {
                rec: Recorder {
                    producer,
                    audio,
                    with_audio: Arc::new(AtomicBool::new(false)),
                    armed: armed.clone(),
                    dropped: dropped.clone(),
                    last_params: [f32::NAN; ParamKey::ALL.len()],
                    was_armed: false,
                    last_position: None,
                    rolling: Arc::new(AtomicBool::new(false)),
                    audio_started: false,
                    stop_at_loop_end: stop_at_loop_end.clone(),
                    hit_loop_end: hit_loop_end.clone(),
                    finished: false,
                    advanced: false,
                },
                entries,
                samples,
                armed,
                stop_at_loop_end,
                hit_loop_end,
                dropped,
            }
        }

        /// Take the arming edge, as `process` does on a take's first block.
        fn arm(&mut self) {
            self.armed.store(true, Ordering::Relaxed);
            assert!(self.rec.is_armed(), "the arming edge");
        }

        fn stop_at_loop_end(&self) {
            self.stop_at_loop_end.store(true, Ordering::Relaxed);
        }

        fn hit_loop_end(&self) -> bool {
            self.hit_loop_end.load(Ordering::Relaxed)
        }

        /// Everything pushed since the last call, rendered as comparable
        /// strings — [`Entry`] is a `Copy` payload for the audio thread and
        /// carries no `Debug` of its own.
        fn pushed(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(entry) = self.entries.pop() {
                out.push(match entry {
                    Entry::Note { t, channel, note, kind } => {
                        let kind = match kind {
                            NoteEventKind::On { .. } => "on",
                            NoteEventKind::Off => "off",
                            NoteEventKind::Tuning { .. } => "tuning",
                            NoteEventKind::AllOff => "alloff",
                        };
                        format!("note {note} ch{channel} {kind} @{t}")
                    }
                    Entry::Param { t, key, value } => format!("param {key}={value} @{t}"),
                    Entry::AudioStart(t) => format!("audio-start @{t}"),
                    Entry::NewPass => "new-pass".to_owned(),
                });
            }
            out
        }

        /// Every sample written to the audio ring since the last call.
        fn written(&mut self) -> Vec<f32> {
            let mut out = Vec::new();
            while let Ok(sample) = self.samples.pop() {
                out.push(sample);
            }
            out
        }
    }

    /// `audio` reserves in whole FRAMES, not samples. This ring is
    /// stereo-interleaved and lands in a WAV, so half a frame written here
    /// swaps that file's channels from there on — permanently, and the writer
    /// cannot notice, because it derives its frame count by dividing the bytes
    /// it was handed.
    ///
    /// An odd `samples` is what a caller hands over the moment it sizes a block
    /// by the host's channel count instead of [`TAKE_CHANNELS`]. The tail
    /// sample has to be dropped rather than written out of phase, and counted
    /// as the hole in the recording that it is.
    ///
    /// This pins the BLOCK-size half of the reservation, which is the half that
    /// does the work: the ring here is only ever written in whole frames, so
    /// its free count cannot go odd on its own. The free-space half is proved
    /// exhaustively over on `interleaved_reservation`, since driving this ring
    /// to an odd occupancy would mean reaching past the writer thread.
    #[test]
    fn recorded_audio_reserves_whole_frames_and_counts_the_dropped_tail() {
        let (mut rec, ctrl) = channel();
        ctrl.recording.store(true, Ordering::Relaxed);

        // Even, into an empty ring: the whole block fits, nothing is dropped.
        rec.audio(&mut std::iter::repeat_n(0.25f32, 8), 8);
        ctrl.tick(true, 0);
        assert!(
            !ctrl.status().contains("DROPPED"),
            "an even block fits whole, but got: {}",
            ctrl.status()
        );

        // Odd: the last sample is half a frame and must not reach the ring.
        rec.audio(&mut std::iter::repeat_n(0.25f32, 9), 9);
        ctrl.tick(true, 0);
        assert!(
            ctrl.status().contains("DROPPED"),
            "an odd tail must be dropped, not written out of phase; got: {}",
            ctrl.status()
        );
    }

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
    /// passed here can only add. Passing it unconditionally would make the
    /// Video pane's "Live" choice unreachable: the pane writes
    /// `playhead: false` into the take and the flag would turn it straight
    /// back on. Leaving it unset is what lets the row decide both
    /// ways, so this holds the request to saying nothing.
    #[test]
    fn the_take_decides_the_spectrogram_not_a_forced_flag() {
        for playhead in [false, true] {
            let config = RenderConfig { playhead, ..Default::default() };
            assert_eq!(RenderRequest::from_config(&config).unwrap().playhead, None);
            let now = RenderRequest::render_now(&config, "(dummy)".into());
            assert_eq!(now.playhead, None, "Re-render take must not force it either");
        }
    }

    /// A second request for the same take supersedes the first, which is what
    /// makes "Render now" during a render mean restart rather than a second
    /// render writing the same video — and a request for a DIFFERENT take
    /// supersedes nothing, because it is not about that video at all.
    #[test]
    fn a_later_request_supersedes_the_one_in_flight() {
        let control = RenderControl::default();
        let take = std::path::Path::new("/takes/take-1.take");
        let other = std::path::Path::new("/takes/take-2.take");
        let first = control.claim(take);
        assert!(!control.superseded(take, first), "the only request in flight is current");

        let second = control.claim(take);
        assert!(control.superseded(take, first), "the first must stand down");
        assert!(!control.superseded(take, second), "the second is now the live one");

        // Another take's request stands beside it rather than over it.
        let elsewhere = control.claim(other);
        assert!(!control.superseded(take, second), "take-2's request is not about take-1");
        assert!(!control.superseded(other, elsewhere));

        // Generations are not reused, so a render cannot be revived by a
        // later one happening to land on its number.
        let third = control.claim(take);
        assert!(third > elsewhere && elsewhere > second && second > first);
        assert!(control.superseded(take, first) && control.superseded(take, second));

        // A finished run gives its claim back, and a stale one cannot take
        // away the claim that replaced it.
        control.release(take, second);
        assert!(!control.superseded(take, third), "a stale release must not unseat the live run");
        control.release(take, third);
        assert!(control.claims.lock().get(take).is_none(), "the finished take is not kept");
    }

    /// Cancelling with nothing running is a no-op rather than a panic: the
    /// first render of a session takes exactly that path, since every request
    /// cancels before it spawns.
    #[test]
    fn cancelling_an_idle_render_control_does_nothing() {
        let control = RenderControl::default();
        control.cancel_in_flight(std::path::Path::new("/nowhere/take-1.take"));
        assert!(control.child.lock().is_none());
    }

    /// Spin until `done` or the budget runs out, so the concurrency tests
    /// below wait on the thing they mean rather than on a fixed sleep.
    #[cfg(unix)]
    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    /// Pressing "Re-render take" during a render kills the one in flight instead
    /// of running a second alongside it.
    ///
    /// Against a real process, because that is the whole claim: the generation
    /// bookkeeping above proves only that the arithmetic is right, and the
    /// thing that goes wrong — two renderers writing one video — lives
    /// entirely in the part that spawns and kills.
    #[cfg(unix)]
    #[test]
    fn a_second_request_kills_the_render_in_flight() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("harmonigraph-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        // Stands in for the renderer: ignores its arguments and outlives the
        // test, so the only way it ends is being killed.
        let fake = dir.join("slow-renderer");
        std::fs::write(&fake, "#!/bin/sh\nsleep 300\n").expect("write fake renderer");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let take = dir.join("take-1.take");
        let control = Arc::new(RenderControl::default());
        let status = Arc::new(Mutex::new(String::new()));
        let progress = Arc::new(Progress::default());
        let start = || {
            spawn_render(
                RenderRequest {
                    program: fake.clone(),
                    audio: None,
                    align: None,
                    ui_state: None,
                    size: [16, 16],
                    playhead: None,
                },
                take.clone(),
                status.clone(),
                progress.clone(),
                control.clone(),
            )
        };

        start();
        wait_until("the first render to start", || control.child.lock().is_some());
        let first = control.child.lock().as_ref().map(|f| f.child.id()).expect("a first child");

        start();
        // The second cannot run until the first has been reaped, so seeing a
        // different process here is the cancellation having completed rather
        // than merely been asked for.
        wait_until("the second render to replace the first", || {
            control.child.lock().as_ref().is_some_and(|f| f.child.id() != first)
        });

        // The cancelled render must not have reported anything: its failure
        // was one we caused, and its replacement owns the status line.
        assert!(
            !status.lock().contains("failed"),
            "a cancelled render reported failure: {}",
            status.lock()
        );
        // And exactly one render is in flight, not two.
        assert!(progress.read().is_some(), "the bar still shows a render");

        control.cancel_in_flight(&take);
        wait_until("the last render to be reaped", || control.child.lock().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A render of ANOTHER take queues behind the one in flight rather than
    /// killing it.
    ///
    /// Auto-render fires for every finished take (see
    /// [`a_finished_take_always_renders`]) and each take is a new file, so
    /// recording twice in a row is two requests naming two different videos.
    /// Cancelling on that would drop the first take's video on the floor
    /// silently — a superseded run cleans up its partial and returns without
    /// touching the status line, so there is nothing on screen to say the
    /// video is never coming, and `last_take` has already moved on so
    /// "Re-render take" cannot reach it either.
    #[cfg(unix)]
    #[test]
    fn a_render_of_another_take_waits_rather_than_replacing_this_one() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("harmonigraph-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let fake = dir.join("slow-renderer");
        std::fs::write(&fake, "#!/bin/sh\nexec sleep 300\n").expect("write fake renderer");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let control = Arc::new(RenderControl::default());
        let status = Arc::new(Mutex::new(String::new()));
        let progress = Arc::new(Progress::default());
        let start = |take: std::path::PathBuf| {
            spawn_render(
                RenderRequest {
                    program: fake.clone(),
                    audio: None,
                    align: None,
                    ui_state: None,
                    size: [16, 16],
                    playhead: None,
                },
                take,
                status.clone(),
                progress.clone(),
                control.clone(),
            )
        };

        start(dir.join("take-1.take"));
        wait_until("the first take's render to start", || control.child.lock().is_some());
        let first = control.child.lock().as_ref().map(|f| f.child.id()).expect("a first child");

        // A second take finishes while the first is still rendering.
        start(dir.join("take-2.take"));
        // Long enough that a cancellation would have landed: the existing
        // same-take test sees the replacement inside this window.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            control.child.lock().as_ref().map(|f| f.child.id()),
            Some(first),
            "take-1's render was killed by take-2's, so take-1.mp4 is never produced"
        );
        assert!(
            !status.lock().contains("failed"),
            "the surviving render reported failure: {}",
            status.lock()
        );

        control.cancel_in_flight(&dir.join("take-1.take"));
        wait_until("the queued render to take over", || {
            control.child.lock().as_ref().is_some_and(|f| f.child.id() != first)
        });
        control.cancel_in_flight(&dir.join("take-2.take"));
        wait_until("the last render to be reaped", || control.child.lock().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Verbatim shapes from `harmonigraph-offline`'s stderr — the opening line
    /// that announces the frame count, and the counter it rewrites as it goes.
    #[test]
    fn the_renderers_own_output_is_what_puts_numbers_on_the_bar() {
        let header = "/music/Harmonigraph Takes/take-1.take: 16.5s of events \
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
        assert!(parse_report("/music/odd frames/take.take: could not be read").is_none());
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

    /// The bar survives a render ending while another is in flight.
    ///
    /// `RenderControl` serialises renders, so nothing should reach two — but
    /// the count is what makes that a belt rather than the only strap, and a
    /// flag cleared by whichever finished first would blank the bar out from
    /// under a running render. This holds the counting to that.
    #[test]
    fn the_bar_lasts_until_the_last_render_ends() {
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

    /// Rolling is the UNION of "the position advanced" and "the host says
    /// playing", so a position that moves records the block whatever the flag
    /// says.
    ///
    /// During an offline export some hosts report `playing = false` — nothing
    /// is being played, after all. Reading the flag alone would record nothing
    /// for the whole export and leave a take that replays as an empty lattice,
    /// with nothing anywhere to say why.
    #[test]
    fn a_position_that_advances_rolls_even_when_the_host_says_it_is_not_playing() {
        let mut b = Bench::new();
        b.arm();
        // Nothing to compare against on the first block, so the flag IS all
        // there is, and it says no.
        assert!(!b.rec.observe_transport(0.0, false));
        assert!(
            b.rec.observe_transport(1.0, false),
            "the position advanced, so the block belongs in the take"
        );
        assert!(b.rec.observe_transport(2.0, false));
    }

    /// The other half of the union, and its floor: a host that reports
    /// `playing` before its position has moved still gets its first block
    /// captured, and only a block where BOTH say no is skipped — which is
    /// exactly a parked transport.
    #[test]
    fn only_a_parked_transport_stops_a_block_being_recorded() {
        let mut b = Bench::new();
        b.arm();
        assert!(!b.rec.observe_transport(4.0, false));
        assert!(
            !b.rec.observe_transport(4.0, false),
            "neither the position nor the flag: this is a parked transport"
        );
        assert!(b.rec.observe_transport(4.0, true), "the flag alone still counts");
    }

    /// AtLoopEnd has to survive the playhead being parked for MORE THAN ONE
    /// block before playback starts.
    ///
    /// `advanced` is what separates the transport snapping back to the loop
    /// start from a loop actually wrapping, and only real forward motion may
    /// set it. A transport standing still republishes the same position every
    /// block — which is what a parked playhead does for as long as it is parked
    /// — and counting that as motion would arm the latch before anything was
    /// recorded, so the jump to the loop start would end the take: an empty
    /// file, and a render of nothing.
    #[test]
    fn a_playhead_parked_for_several_blocks_has_not_advanced() {
        let mut b = Bench::new();
        b.arm();
        b.stop_at_loop_end();

        // Parked past the loop start, transport stopped, for several blocks.
        for _ in 0..4 {
            assert!(!b.rec.observe_transport(5.0, false), "parked");
        }

        // Hit play: the transport snaps back to the loop start. Nothing has
        // been recorded yet, so this begins the pass rather than ending it.
        assert!(b.rec.observe_transport(0.0, true), "the jump-to-start begins the pass");
        assert!(!b.hit_loop_end(), "a parked playhead has not advanced");
        // Beginning the pass is not splitting it: AtLoopEnd only ever wants one
        // file, and a `NewPass` here would leave the take's notes in the second
        // one while the first — the one that renders — stayed empty.
        let begun = b.pushed();
        assert!(begun.is_empty(), "the jump-to-start must not split the take: {begun:?}");

        // Real forward motion, and only then does a wrap mean the loop end.
        assert!(b.rec.observe_transport(1.0, true));
        assert!(!b.rec.observe_transport(0.0, true), "the real wrap ends the take");
        assert!(b.hit_loop_end());
    }

    /// The backward-jump threshold is there to ignore a host's own jitter
    /// around a loop point, so a step back smaller than it is not a wrap.
    ///
    /// Splitting on jitter would cut the take into a second file mid-phrase,
    /// and under AtLoopEnd it would end the take outright — both at a position
    /// the user asked nothing of. The same threshold is what keeps a transport
    /// standing still from reading as a wrap.
    #[test]
    fn a_step_back_smaller_than_the_threshold_is_jitter_rather_than_a_wrap() {
        let mut b = Bench::new();
        b.arm();
        assert!(b.rec.observe_transport(1.00, true));
        assert!(b.rec.observe_transport(1.04, true));
        // 0.02 back, under BACKWARD_JUMP: the host is jittering, not looping.
        assert!(b.rec.observe_transport(1.02, true));
        let jitter = b.pushed();
        assert!(jitter.is_empty(), "jitter must not split the take, but pushed {jitter:?}");

        // A real wrap is far larger, and does split it.
        assert!(b.rec.observe_transport(0.0, true));
        assert_eq!(b.pushed(), ["new-pass"]);
    }

    /// A plain wrap splits the take into a second file; an AtLoopEnd wrap does
    /// not.
    ///
    /// The writer opens the next pass eagerly, so a `NewPass` pushed at the
    /// moment the take ends would leave the empty second file as the one that
    /// renders. The two wraps are identical from the transport's side and only
    /// the mode tells them apart, so each is held to pushing what it should and
    /// nothing besides.
    #[test]
    fn a_plain_wrap_splits_the_file_and_a_loop_end_wrap_does_not() {
        let mut b = Bench::new();
        b.arm();
        assert!(b.rec.observe_transport(0.0, true));
        assert!(b.rec.observe_transport(2.0, true));
        assert!(b.rec.observe_transport(0.0, true), "a plain wrap keeps recording");
        assert_eq!(b.pushed(), ["new-pass"], "the plain wrap opens the next pass");

        let mut b = Bench::new();
        b.arm();
        b.stop_at_loop_end();
        assert!(b.rec.observe_transport(0.0, true));
        assert!(b.rec.observe_transport(2.0, true));
        assert!(!b.rec.observe_transport(0.0, true), "the loop end ends the take");
        let split = b.pushed();
        assert!(
            split.is_empty(),
            "a split here leaves the empty second file as the one that renders: {split:?}"
        );
    }

    /// The notes are what the take is FOR, and they reach the ring with the
    /// time, channel, and kind they were given.
    #[test]
    fn notes_reach_the_ring_with_their_time_and_channel() {
        let mut b = Bench::new();
        b.arm();
        b.rec.note(0.5, 3, 60, NoteEventKind::On { velocity: 0.8 });
        b.rec.note(1.5, 3, 60, NoteEventKind::Off);
        b.rec.note(2.0, 0, 0, NoteEventKind::AllOff);
        assert_eq!(
            b.pushed(),
            ["note 60 ch3 on @0.5", "note 60 ch3 off @1.5", "note 0 ch0 alloff @2"]
        );
    }

    /// `mark_audio_start` is idempotent per PASS: the first call fixes where
    /// the WAV sits against the notes, and that pass's header cannot be revised
    /// afterwards. A wrap begins a new pass, whose audio starts somewhere else.
    #[test]
    fn the_audio_start_is_declared_once_per_pass() {
        let mut b = Bench::new();
        b.arm();
        b.rec.mark_audio_start(0.25);
        b.rec.mark_audio_start(0.30);
        b.rec.mark_audio_start(9.0);
        assert_eq!(b.pushed(), ["audio-start @0.25"], "the first call is the one that counts");

        // The wrap re-arms it, because the next pass's audio starts elsewhere.
        assert!(b.rec.observe_transport(0.0, true));
        assert!(b.rec.observe_transport(2.0, true));
        assert!(b.rec.observe_transport(0.0, true));
        b.rec.mark_audio_start(7.5);
        assert_eq!(b.pushed(), ["new-pass", "audio-start @7.5"]);
    }

    /// Only parameters that MOVED are recorded — a full set every block would
    /// bury the take — and a wrap rewrites all of them, because the new pass's
    /// file starts empty and would otherwise replay with whatever the previous
    /// pass happened to end on.
    #[test]
    fn params_record_only_changes_and_a_wrap_rewrites_every_one() {
        let mut b = Bench::new();
        b.arm();
        let mut values = [0.0f32; ParamKey::ALL.len()];
        b.rec.params(0.0, values);
        assert_eq!(
            b.pushed().len(),
            ParamKey::ALL.len(),
            "arming resets to NaN, so a take's first block writes a full set"
        );

        b.rec.params(1.0, values);
        let unmoved = b.pushed();
        assert!(unmoved.is_empty(), "an unmoved parameter is not re-recorded: {unmoved:?}");

        values[0] = 0.5;
        b.rec.params(2.0, values);
        assert_eq!(b.pushed(), ["param 0=0.5 @2"], "only the one that moved");

        // The wrap opens an empty file, so every parameter is written again
        // even though none of them changed.
        assert!(b.rec.observe_transport(0.0, true));
        assert!(b.rec.observe_transport(2.0, true));
        assert!(b.rec.observe_transport(0.0, true));
        b.rec.params(3.0, values);
        let after = b.pushed();
        assert_eq!(
            after.len(),
            ParamKey::ALL.len() + 1,
            "the new pass, then a full set for it: {after:?}"
        );
        assert_eq!(after[0], "new-pass");
    }

    /// The reserved samples actually reach the ring.
    ///
    /// The counter beside them is a different claim: `audio` could reserve the
    /// right amount, count nothing dropped, and write nothing at all, and every
    /// assertion about `dropped` would still hold while the WAV came out empty.
    #[test]
    fn reserved_audio_is_actually_written() {
        let mut b = Bench::new();
        b.arm();
        b.rec.audio(&mut [0.25f32, -0.25, 0.5, -0.5].into_iter(), 4);
        assert_eq!(b.written(), [0.25, -0.25, 0.5, -0.5]);
        assert_eq!(b.dropped.load(Ordering::Relaxed), 0);

        // An odd block still writes its whole frames; only the half frame on
        // the end is dropped.
        b.rec.audio(&mut [1.0f32, -1.0, 0.75].into_iter(), 3);
        assert_eq!(b.written(), [1.0, -1.0], "the whole frame lands, the half frame does not");
        assert_eq!(b.dropped.load(Ordering::Relaxed), 1);
    }

    /// The take's state resets on the arming EDGE, not on the armed LEVEL.
    ///
    /// `process` calls `is_armed` every block, so a reset keyed off "armed"
    /// rather than "newly armed" would fire on every block of the take:
    /// `last_position` would forget the previous block, so no wrap could ever
    /// be detected, and the loop-end latch would clear as fast as it was set.
    #[test]
    fn re_checking_armed_mid_take_does_not_reset_the_take() {
        let mut b = Bench::new();
        b.arm();
        b.stop_at_loop_end();
        assert!(b.rec.observe_transport(0.0, true));
        assert!(b.rec.is_armed(), "still armed, one block later");
        assert!(b.rec.observe_transport(2.0, true));
        assert!(b.rec.is_armed());

        // The wrap is still seen as one, and still ends the take.
        assert!(!b.rec.observe_transport(0.0, true), "the wrap ends the take");
        assert!(b.hit_loop_end());
        assert!(b.rec.is_armed());
        assert!(b.hit_loop_end(), "the latch survives the next block's arm check");
        assert!(!b.rec.observe_transport(1.0, true), "and the take stays finished");
    }

    /// The status line separates a take that is capturing from one that is
    /// waiting, and from one that captured something and then stopped.
    ///
    /// It is the only feedback there is that arming while the transport is
    /// parked records nothing — "paused (0 events)" and "armed — waiting" look
    /// alike but mean opposite things about whether the take has anything in
    /// it.
    #[test]
    fn the_status_line_separates_waiting_from_paused_from_recording() {
        let (_rec, ctrl) = channel();
        ctrl.recording.store(true, Ordering::Relaxed);

        ctrl.tick(false, 0);
        assert_eq!(ctrl.status(), "armed — waiting for the transport to roll");

        ctrl.tick(true, 12);
        assert_eq!(ctrl.status(), "recording — 12 events");

        // Rolled, then stopped: what it caught is worth saying, and it is not
        // the same as never having started.
        ctrl.tick(false, 12);
        assert_eq!(ctrl.status(), "paused (12 events) — transport stopped");
    }

    /// `stop` is only for a take that is running, and it stops the audio
    /// capture along with the notes.
    ///
    /// The Video pane reaches it from more than one place — the toggle, and the
    /// loop-end handler that fires when the audio thread latches `hit_loop_end`
    /// — so it has to be safe to call twice. Disarming twice is harmless, but
    /// sending a second `Stop` is not: the writer would close a file it had
    /// already closed and launch a second render of the take the first `Stop`
    /// is still finishing.
    #[test]
    fn stopping_a_take_that_is_not_running_does_nothing() {
        let (rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        ctrl.with_audio.store(true, Ordering::Relaxed);
        ctrl.stop(None);
        assert!(ctrl.armed.load(Ordering::Relaxed), "not recording, so there is nothing to stop");
        assert!(rec.wants_audio(), "and nothing to stop reading the input bus for");

        ctrl.recording.store(true, Ordering::Relaxed);
        ctrl.stop(None);
        assert!(!ctrl.armed.load(Ordering::Relaxed), "a running take disarms");
        assert!(!ctrl.is_recording());
        assert!(!rec.wants_audio(), "and the audio thread stops reading the input bus");
    }

    /// The rings the plugin ships with have room in them.
    ///
    /// A record that does not fit IS counted and surfaced — but the surfacing
    /// is a status line the user has to be looking at, and a ring sized to
    /// nothing would drop every record of every take while the pane went on
    /// saying it was recording. The capacities are the only thing between that
    /// and a silently empty video, and nothing else here reads them.
    #[test]
    fn the_shipped_rings_have_room_for_what_the_audio_thread_pushes() {
        let (mut rec, ctrl) = channel();
        ctrl.recording.store(true, Ordering::Relaxed);
        rec.note(0.0, 0, 60, NoteEventKind::On { velocity: 1.0 });
        rec.audio(&mut std::iter::repeat_n(0.0f32, 512), 512);
        ctrl.tick(true, 1);
        assert!(!ctrl.status().contains("DROPPED"), "a shipped ring dropped: {}", ctrl.status());
    }

    /// What the audio thread saw is what the GUI reads back.
    ///
    /// `rolling` is published by `observe_transport` and reached only through
    /// `Control` — it is the GUI's one honest view of whether the transport is
    /// moving, and both the status line and the "waiting for the transport"
    /// message hang off it. A getter wired to the wrong atomic would leave the
    /// pane confidently describing a take that is not being recorded.
    #[test]
    fn the_gui_reads_back_the_rolling_the_audio_thread_published() {
        let (mut rec, ctrl) = channel();
        ctrl.armed.store(true, Ordering::Relaxed);
        assert!(rec.is_armed());

        assert!(!rec.observe_transport(3.0, false));
        assert!(!ctrl.is_rolling(), "parked");

        assert!(rec.observe_transport(4.0, false));
        assert!(ctrl.is_rolling(), "the position advanced");

        assert!(!rec.observe_transport(4.0, false));
        assert!(!ctrl.is_rolling(), "parked again");
    }

    /// "Re-render take" with nothing recorded yet says so, rather than
    /// appearing to work.
    #[test]
    fn re_rendering_with_no_take_yet_explains_itself() {
        let (_rec, ctrl) = channel();
        assert_eq!(ctrl.last_take(), None, "nothing recorded this session");
        ctrl.render_now(RenderRequest::render_now(&RenderConfig::default(), "(dummy)".into()));
        assert_eq!(ctrl.status(), "no take recorded yet to render");

        // And the finished take the writer thread reports is the one the button
        // reaches for.
        let path = std::path::PathBuf::from("/takes/take-1.take");
        *ctrl.last_take.lock() = Some(path.clone());
        assert_eq!(ctrl.last_take(), Some(path));
    }

    /// The Video pane's bar reads the counter the render thread writes.
    #[test]
    fn the_panes_bar_reads_the_render_threads_progress() {
        let (_rec, ctrl) = channel();
        assert_eq!(ctrl.render_progress(), None, "no bar while nothing is rendering");
        ctrl.progress.begin();
        ctrl.progress.done.store(7, Ordering::Relaxed);
        ctrl.progress.total.store(9, Ordering::Relaxed);
        assert_eq!(
            ctrl.render_progress(),
            Some(harmonigraph_ui::RenderProgress { done: 7, total: 9 })
        );
        ctrl.progress.end();
        assert_eq!(ctrl.render_progress(), None);
    }

    /// The first pass keeps the take's own name; later passes are suffixed.
    ///
    /// `path_for` belongs to the writer thread but is pure, so unlike the rest
    /// of that half it is reachable from here — and worth reaching, because
    /// getting the boundary backwards would have pass 1 write `-1` beside the
    /// name its header advertises, and pass 2 write over the file pass 1 had
    /// just finished.
    #[test]
    fn the_first_pass_keeps_the_takes_name_and_later_passes_are_suffixed() {
        let base = std::path::Path::new("/takes/take-1700000000.take");
        assert_eq!(Open::path_for(base, 1), base);
        assert_eq!(Open::path_for(base, 2), std::path::Path::new("/takes/take-1700000000-2.take"));
        assert_eq!(Open::path_for(base, 3), std::path::Path::new("/takes/take-1700000000-3.take"));
    }

    /// A signal arriving mid-read is not the end of the render, and a real
    /// error is.
    ///
    /// `Interrupted` means a signal landed — and one does, every time
    /// `cancel_in_flight` kills a child — not that the renderer stopped
    /// talking. Treating it as the end would freeze the bar partway and leave
    /// the status line holding whatever had arrived before the signal, for a
    /// render still going perfectly well. Treating a REAL error as a signal is
    /// the opposite mistake: the loop would go round again on a pipe that has
    /// nothing more to give.
    #[test]
    fn a_signal_mid_read_is_not_the_end_of_the_render() {
        /// A stderr pipe handing back a scripted sequence of chunks and the
        /// errors that arrive between them.
        struct Scripted {
            steps: std::vec::IntoIter<Result<&'static [u8], std::io::ErrorKind>>,
        }
        impl std::io::Read for Scripted {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                match self.steps.next() {
                    Some(Ok(chunk)) => {
                        out[..chunk.len()].copy_from_slice(chunk);
                        Ok(chunk.len())
                    }
                    Some(Err(kind)) => Err(std::io::Error::from(kind)),
                    None => Ok(0),
                }
            }
        }

        let progress = Progress::default();
        progress.begin();
        let signalled = Scripted {
            steps: vec![
                Ok(&b"x.take: -> 300 frames at 60 fps\n"[..]),
                Err(std::io::ErrorKind::Interrupted),
                Ok(&b"\r  240/300 frames (80%)\nharmonigraph-offline: ffmpeg died\n"[..]),
            ]
            .into_iter(),
        };
        assert_eq!(
            follow(signalled, &progress),
            "harmonigraph-offline: ffmpeg died",
            "everything after the signal still belongs to this render"
        );
        assert_eq!(
            progress.read(),
            Some(harmonigraph_ui::RenderProgress { done: 240, total: 300 })
        );

        // A real error ends the read, so nothing past it is consumed.
        let broken = Scripted {
            steps: vec![
                Ok(&b"harmonigraph-offline: writing frames\n"[..]),
                Err(std::io::ErrorKind::BrokenPipe),
                Ok(&b"harmonigraph-offline: never read\n"[..]),
            ]
            .into_iter(),
        };
        assert_eq!(
            follow(broken, &progress),
            "harmonigraph-offline: writing frames",
            "a broken pipe is the end, not something to read past"
        );
    }

    /// The default renderer path is ABSOLUTE.
    ///
    /// A relative one is resolved against the DAW's working directory, which is
    /// the thing a fixed location exists to avoid: a host can start the plugin
    /// from anywhere, so the same relative path finds the renderer on one
    /// machine and nothing at all on the next.
    #[test]
    fn the_default_renderer_path_is_absolute() {
        // The absoluteness comes from `HOME`; with it unset there is nothing to
        // be absolute against and `.` is the documented fallback.
        let Ok(home) = std::env::var("HOME") else { return };
        let path = default_renderer_path();
        assert!(path.is_absolute(), "resolved against the host's cwd: {path:?}");
        assert!(path.starts_with(&home), "{path:?} is not under {home}");
    }
}
