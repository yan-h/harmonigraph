//! Recording a take while the host renders offline.
//!
//! When Bitwig (or any host) exports audio, it runs the plugin faster
//! than realtime with no GUI attached. That is the cheapest possible
//! moment to capture everything the visualization is a function of: the
//! note stream arrives sample-accurate, the parameters carry their
//! automation, and nothing is dropped for want of a GUI to drain it.
//! Afterwards `lattice-offline` replays the take into a video at whatever
//! frame rate and resolution you like, and the DAW's own bounce supplies
//! both the audio track and the spectrum.
//!
//! So: **exporting audio also exports a take.** No button, nothing to
//! remember to arm — nice-plug hands us `ProcessMode::Offline` in
//! `initialize`, and the host re-initializes the plugin whenever that
//! changes, so the arming is exact.
//!
//! The audio thread must not touch a file, so it only pushes plain Copy
//! records into a ring; a writer thread drains it. Unlike the note ring
//! feeding the GUI — which drops on backpressure by design, because a
//! stalled meter should never stall audio — **a dropped take record is a
//! silently wrong video**, so overflow is counted and reported rather
//! than ignored.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lattice_core::notes::NoteEventKind;
use lattice_ui::params::ParamKey;

/// Ring capacity. Sized for a fast export rather than for a frame: at
/// 20x realtime a dense piece still only produces a few thousand records
/// a second, and the writer thread drains continuously.
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

/// The audio-thread half: a producer and a drop counter.
pub struct Recorder {
    producer: rtrb::Producer<Entry>,
    dropped: Arc<AtomicU64>,
    /// Last value written per parameter, so only changes are recorded.
    last_params: [f32; ParamKey::ALL.len()],
}

impl Recorder {
    /// Push one entry, counting it if the ring is full.
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

/// The writer thread's handle. Dropping it stops the thread and finishes
/// the file.
pub struct Session {
    stop: Arc<std::sync::atomic::AtomicBool>,
    dropped: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
    path: std::path::PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            eprintln!(
                "MIDI Lattice 3D: take {} is INCOMPLETE — {dropped} records were dropped. \
                 Re-export, or raise TAKE_RING_CAPACITY.",
                self.path.display()
            );
        } else {
            eprintln!("MIDI Lattice 3D: take written to {}", self.path.display());
        }
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

/// Start recording. Returns the audio-thread producer and the session
/// handle; `None` if the file could not be opened, which must never stop
/// the export itself.
pub fn start(sample_rate: f32, ui_state: String) -> Option<(Recorder, Session)> {
    let dir = take_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("MIDI Lattice 3D: cannot create {}: {err}", dir.display());
        return None;
    }
    // Seconds since the epoch: monotonic enough to sort by, and it does
    // not collide when a project is exported twice in a row.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("take-{stamp}.{}", lattice_take::EXTENSION));

    let header = lattice_take::Header {
        sample_rate,
        ui_state: Some(ui_state),
        source: "midi_lattice_3d (offline render)".into(),
        ..Default::default()
    };
    let mut writer = match lattice_take::Writer::create(&path, &header) {
        Ok(writer) => writer,
        Err(err) => {
            eprintln!("MIDI Lattice 3D: cannot write {}: {err}", path.display());
            return None;
        }
    };

    let (producer, mut consumer) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("lattice-take-writer".into())
            .spawn(move || loop {
                let mut idle = true;
                while let Ok(entry) = consumer.pop() {
                    idle = false;
                    let _ = match entry {
                        Entry::Note { t, channel, note, kind } => {
                            writer.note(lattice_take::NoteRecord {
                                t,
                                channel,
                                note,
                                kind: match kind {
                                    NoteEventKind::On { velocity } => {
                                        lattice_take::NoteKind::On { velocity }
                                    }
                                    NoteEventKind::Off => lattice_take::NoteKind::Off,
                                    NoteEventKind::Tuning { semitones } => {
                                        lattice_take::NoteKind::Tuning { semitones }
                                    }
                                    NoteEventKind::AllOff => lattice_take::NoteKind::AllOff,
                                },
                            })
                        }
                        Entry::Param { t, key, value } => writer.param(lattice_take::ParamRecord {
                            t,
                            id: ParamKey::ALL[key].id().to_string(),
                            value,
                        }),
                    };
                }
                // Stop only after a pass that found nothing left, so the
                // tail of the export always makes it to disk.
                if idle {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(DRAIN_IDLE);
                }
            })
            .ok()?
    };

    Some((
        Recorder {
            producer,
            dropped: dropped.clone(),
            last_params: [f32::NAN; ParamKey::ALL.len()],
        },
        Session { stop, dropped, thread: Some(thread), path },
    ))
}
