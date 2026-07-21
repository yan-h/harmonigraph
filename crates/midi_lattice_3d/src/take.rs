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
}

enum Command {
    Start(Box<lattice_take::Header>, std::path::PathBuf),
    Stop,
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
}

impl Recorder {
    pub fn is_armed(&mut self) -> bool {
        let armed = self.armed.load(Ordering::Relaxed);
        if armed && !self.was_armed {
            self.last_params = [f32::NAN; ParamKey::ALL.len()];
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

    pub fn stop(&self) {
        if !self.is_recording() {
            return;
        }
        // Disarm first, so the audio thread stops pushing before the
        // writer is told to close.
        self.armed.store(false, Ordering::Relaxed);
        let _ = self.commands.send(Command::Stop);
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
            let mut writer: Option<lattice_take::Writer> = None;
            loop {
                match orders.try_recv() {
                    Ok(Command::Start(header, path)) => {
                        match lattice_take::Writer::create(&path, &header) {
                            Ok(new) => {
                                writer = Some(new);
                                *thread_status.lock() =
                                    format!("recording to {}", path.display());
                            }
                            Err(err) => {
                                *thread_status.lock() =
                                    format!("cannot write {}: {err}", path.display());
                            }
                        }
                    }
                    Ok(Command::Stop) => {
                        // Drain what the audio thread already queued
                        // before closing, or the tail of the take is lost.
                        drain(&mut consumer, &mut writer);
                        writer = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        drain(&mut consumer, &mut writer);
                        return;
                    }
                }
                if !drain(&mut consumer, &mut writer) {
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
        },
        Control { commands, armed, dropped, status, recording },
    )
}

/// Move everything queued into the writer (discarding it if none is
/// open). Returns whether anything was there.
fn drain(
    consumer: &mut rtrb::Consumer<Entry>,
    writer: &mut Option<lattice_take::Writer>,
) -> bool {
    let mut any = false;
    while let Ok(entry) = consumer.pop() {
        any = true;
        let Some(writer) = writer.as_mut() else { continue };
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
        };
    }
    any
}
