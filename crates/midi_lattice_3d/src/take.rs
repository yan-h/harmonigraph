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
//! now explicit: a toggle in the View pane, armed by the user, working
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

use lattice_core::notes::NoteEventKind;
use lattice_ui::params::ParamKey;
use parking_lot::Mutex;

/// Ring capacity. Sized for a fast offline render rather than for a
/// frame: even at 20x realtime a dense piece is only a few thousand
/// records a second, and the writer thread drains continuously.
const TAKE_RING_CAPACITY: usize = 1 << 16;

/// How long the writer thread sleeps when it finds the ring empty.
const DRAIN_IDLE: std::time::Duration = std::time::Duration::from_millis(20);

/// One recorded thing, in a form the audio thread can push without
/// allocating. Converted to a `lattice_take::Record` on the writer thread.
#[derive(Clone, Copy)]
pub enum Entry {
    Note { t: f64, channel: u8, note: u8, kind: NoteEventKind },
    /// `key` is an index into [`ParamKey::ALL`] — an id string would mean
    /// allocating on the audio thread.
    Param { t: f64, key: usize, value: f32 },
    /// The transport jumped backwards: a loop wrapped, or the playhead
    /// was dragged. Everything after this belongs to a different pass
    /// through the song, so the writer starts a new file rather than
    /// interleaving two performances at the same song positions.
    NewPass,
}

enum Command {
    Start(Box<lattice_take::Header>, std::path::PathBuf),
    /// Close the file, and — if asked — render it to video.
    Stop(Option<Box<RenderRequest>>),
}

/// What to run once a take is complete.
///
/// The plugin does not render video; `lattice-offline` does, with a
/// headless GPU device and an ffmpeg pipe, neither of which belongs in a
/// real-time audio plugin. This just launches it, off the audio thread
/// and off the GUI thread, so a long render never touches the DAW.
pub struct RenderRequest {
    pub program: std::path::PathBuf,
    /// Bounced audio to mux in and feed the spectrum, if any.
    pub audio: Option<String>,
    /// Extra flags, already split.
    pub extra_args: Vec<String>,
}

impl RenderRequest {
    /// Build a request from the View pane's settings, or `None` if
    /// auto-render is off. Blank fields mean "use the default" rather
    /// than passing an empty argument, which the renderer would reject.
    pub fn from_config(config: &lattice_ui::RenderConfig) -> Option<RenderRequest> {
        if !config.auto_render {
            return None;
        }
        let program = if config.renderer_path.trim().is_empty() {
            default_renderer_path()
        } else {
            std::path::PathBuf::from(config.renderer_path.trim())
        };
        Some(RenderRequest {
            program,
            audio: Some(config.audio_path.trim())
                .filter(|path| !path.is_empty())
                .map(str::to_owned),
            // Whitespace split, no shell quoting: these are flags like
            // `--size 3840x2160`. A path with spaces belongs in the Audio
            // field, which is passed as a single argument.
            extra_args: config.extra_args.split_whitespace().map(str::to_owned).collect(),
        })
    }
}

/// Where `update-plugin.sh` installs the renderer, and where the plugin
/// looks when the path setting is left empty. A fixed location beats
/// guessing at the host's working directory or the bundle's own path.
pub fn default_renderer_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home)
        .join("Library/Application Support/MIDI Lattice 3D/lattice-offline")
}

/// The audio-thread half: push entries, gated by an atomic the GUI owns.
pub struct Recorder {
    producer: rtrb::Producer<Entry>,
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
}

impl Recorder {
    pub fn is_armed(&mut self) -> bool {
        let armed = self.armed.load(Ordering::Relaxed);
        if armed && !self.was_armed {
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
            self.last_position = None;
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

    /// Note where the transport is, and split the take if it jumped
    /// backwards. Called once per block with the block's song position.
    ///
    /// The threshold keeps a host's own jitter around a loop point from
    /// splitting a take; a real loop wrap or playhead drag is far larger.
    pub fn observe_transport(&mut self, t: f64) {
        const BACKWARD_JUMP: f64 = 0.05;
        if self.last_position.is_some_and(|last| t < last - BACKWARD_JUMP) {
            self.push(Entry::NewPass);
            // A new file starts empty, so every parameter must be
            // written again or the new pass replays with whatever the
            // previous one happened to end on.
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
        }
        self.last_position = Some(t);
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
    recording: Arc<AtomicBool>,
}

impl Control {
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> String {
        self.status.lock().clone()
    }

    /// Begin a take. `ui_state` is the persist blob that decides how the
    /// replay will look; `sample_rate` stamps the header.
    pub fn start(&self, sample_rate: f32, ui_state: String) {
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
        let path = dir.join(format!("take-{stamp}.{}", lattice_take::EXTENSION));
        let header = lattice_take::Header {
            sample_rate,
            ui_state: Some(ui_state),
            source: "midi_lattice_3d".into(),
            ..Default::default()
        };

        self.dropped.store(0, Ordering::Relaxed);
        if self.commands.send(Command::Start(Box::new(header), path)).is_err() {
            *self.status.lock() = "take writer thread is gone".into();
            return;
        }
        self.recording.store(true, Ordering::Relaxed);
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
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join("Music").join("MIDI Lattice 3D Takes")
}

/// Build the ring and start the writer thread. Called once, from the
/// plugin's `Default`, so neither arming nor disarming ever has to touch
/// the audio thread's producer.
pub fn channel() -> (Recorder, Control) {
    let (producer, mut consumer) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    let (commands, orders) = mpsc::channel::<Command>();
    let armed = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let recording = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(String::new()));

    let thread_status = status.clone();
    let _ = std::thread::Builder::new()
        .name("lattice-take-writer".into())
        .spawn(move || {
            let mut open: Option<Open> = None;
            loop {
                match orders.try_recv() {
                    Ok(Command::Start(header, path)) => {
                        open = Open::create(*header, path, 1, &thread_status);
                    }
                    Ok(Command::Stop(render)) => {
                        // Drain what the audio thread already queued
                        // before closing, or the tail of the take is lost.
                        drain(&mut consumer, &mut open, &thread_status);
                        let finished = open.take().map(|o| o.finish());
                        if let (Some(path), Some(render)) = (finished, render) {
                            spawn_render(*render, path, thread_status.clone());
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        drain(&mut consumer, &mut open, &thread_status);
                        return;
                    }
                }
                if !drain(&mut consumer, &mut open, &thread_status) {
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
        },
        Control { commands, armed, dropped, status, recording },
    )
}

/// The file currently being written, and what it takes to open the next
/// one when the transport loops.
struct Open {
    writer: lattice_take::Writer,
    header: lattice_take::Header,
    /// The first pass's path; later passes append `-2`, `-3`, ...
    base: std::path::PathBuf,
    pass: u32,
}

impl Open {
    fn create(
        header: lattice_take::Header,
        base: std::path::PathBuf,
        pass: u32,
        status: &Mutex<String>,
    ) -> Option<Open> {
        let path = if pass <= 1 {
            base.clone()
        } else {
            let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
            base.with_file_name(format!("{stem}-{pass}.{}", lattice_take::EXTENSION))
        };
        match lattice_take::Writer::create(&path, &header) {
            Ok(writer) => {
                *status.lock() = if pass <= 1 {
                    format!("recording to {}", path.display())
                } else {
                    format!("pass {pass} -> {}", path.display())
                };
                Some(Open { writer, header, base, pass })
            }
            Err(err) => {
                *status.lock() = format!("cannot write {}: {err}", path.display());
                None
            }
        }
    }

    /// Close the file and hand back the path that was written.
    fn finish(self) -> std::path::PathBuf {
        let path = self.path();
        drop(self.writer);
        path
    }

    /// The file this pass is writing to.
    fn path(&self) -> std::path::PathBuf {
        if self.pass <= 1 {
            self.base.clone()
        } else {
            let stem = self.base.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
            self.base.with_file_name(format!("{stem}-{}.{}", self.pass, lattice_take::EXTENSION))
        }
    }

    /// Close this file and open the next pass's.
    fn next_pass(self, status: &Mutex<String>) -> Option<Open> {
        let Open { header, base, pass, writer } = self;
        drop(writer);
        Open::create(header, base, pass + 1, status)
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
        let Some(writer) = open.as_mut().map(|o| &mut o.writer) else { continue };
        let _ = match entry {
            Entry::Note { t, channel, note, kind } => writer.note(lattice_take::NoteRecord {
                t,
                channel,
                note,
                kind: match kind {
                    NoteEventKind::On { velocity } => lattice_take::NoteKind::On { velocity },
                    NoteEventKind::Off => lattice_take::NoteKind::Off,
                    NoteEventKind::Tuning { semitones } => {
                        lattice_take::NoteKind::Tuning { semitones }
                    }
                    NoteEventKind::AllOff => lattice_take::NoteKind::AllOff,
                },
            }),
            Entry::Param { t, key, value } => writer.param(lattice_take::ParamRecord {
                t,
                id: ParamKey::ALL[key].id().to_string(),
                value,
            }),
            // Handled above; the writer never sees it.
            Entry::NewPass => Ok(()),
        };
    }
    any
}

/// Run the renderer on the finished take, on a thread of its own so a
/// long render neither blocks the writer nor the DAW. The video lands
/// next to the take.
fn spawn_render(
    request: RenderRequest,
    take_path: std::path::PathBuf,
    status: Arc<Mutex<String>>,
) {
    let _ = std::thread::Builder::new()
        .name("lattice-take-render".into())
        .spawn(move || {
            let out = take_path.with_extension("mp4");
            let mut command = std::process::Command::new(&request.program);
            command.arg(&take_path).arg("--out").arg(&out);
            if let Some(audio) = &request.audio {
                command.arg("--audio").arg(audio);
            }
            command.args(&request.extra_args);

            *status.lock() = format!("rendering {}...", out.display());
            match command.output() {
                Ok(done) if done.status.success() => {
                    *status.lock() = format!("rendered {}", out.display());
                }
                Ok(done) => {
                    // The renderer's own diagnostics are far more useful
                    // than the exit code, and this is the only place a
                    // plugin user will ever see them.
                    let stderr = String::from_utf8_lossy(&done.stderr);
                    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
                    *status.lock() = format!("render failed: {last}");
                }
                Err(err) => {
                    *status.lock() = format!(
                        "could not run {}: {err} — check the Renderer path",
                        request.program.display()
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_ui::RenderConfig;

    #[test]
    fn auto_render_off_means_no_request() {
        let config = RenderConfig { auto_render: false, ..Default::default() };
        assert!(RenderRequest::from_config(&config).is_none());
    }

    /// Blank fields must fall back, not become empty arguments — an
    /// empty `--audio ""` makes the renderer fail on a file that isn't
    /// there, which would be a baffling way for this to break.
    #[test]
    fn blank_settings_fall_back_rather_than_passing_empty_arguments() {
        let config = RenderConfig {
            auto_render: true,
            renderer_path: "  ".into(),
            audio_path: "   ".into(),
            extra_args: "   ".into(),
        };
        let request = RenderRequest::from_config(&config).unwrap();
        assert_eq!(request.program, default_renderer_path());
        assert_eq!(request.audio, None);
        assert!(request.extra_args.is_empty());
    }

    #[test]
    fn options_split_on_whitespace_and_paths_stay_whole() {
        let config = RenderConfig {
            auto_render: true,
            renderer_path: "/opt/lattice-offline".into(),
            audio_path: "/Users/yan/My Bounces/piece.wav".into(),
            extra_args: "--size 3840x2160   --layout side-by-side".into(),
        };
        let request = RenderRequest::from_config(&config).unwrap();
        assert_eq!(request.program, std::path::PathBuf::from("/opt/lattice-offline"));
        // One argument, spaces and all — it is passed directly, never
        // through a shell.
        assert_eq!(request.audio.as_deref(), Some("/Users/yan/My Bounces/piece.wav"));
        assert_eq!(
            request.extra_args,
            vec!["--size", "3840x2160", "--layout", "side-by-side"]
        );
    }

    #[test]
    fn the_default_renderer_path_is_where_update_plugin_installs_it() {
        let path = default_renderer_path();
        assert!(path.ends_with("MIDI Lattice 3D/lattice-offline"), "{path:?}");
    }
}
