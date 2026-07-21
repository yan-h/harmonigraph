//! A **take**: everything the visualization is a function of, recorded on
//! the audio clock so it can be replayed exactly.
//!
//! The point of this crate is to break the visualization free of realtime.
//! The display is a pure function of `(note events, parameters, tuning,
//! view, camera, now)` — no RNG, no wall clock — so if you record the
//! inputs once, you can re-render the output as many times as you like,
//! at any frame rate and any resolution, long after the music stopped.
//! That is what makes "play the piece, then make the video" possible
//! without the two happening at the same speed.
//!
//! A take is written by a shell (the plugin during a DAW export, the
//! standalone harness while you play) and read by `lattice-offline`.
//!
//! # Format
//!
//! One RON-encoded [`Record`] per line, appendable and streamable:
//!
//! ```text
//! Header((version:1,sample_rate:48000.0,...))
//! Note((t:0.5,channel:0,note:60,kind:On((velocity:0.8))))
//! Param((t:0.0,id:"pitch-class-fade",value:2.0))
//! ```
//!
//! Line-oriented rather than one big document for three reasons: the
//! writer can append as the export runs without holding the whole take in
//! memory; a take truncated by a crash loses only its last line; and a
//! take is greppable and hand-editable, which matters a lot the first
//! time a render comes out wrong.
//!
//! Times are **seconds on the audio clock**, counted from the start of
//! the recording, which is the same clock the plugin stamps its note
//! events with. They are deliberately NOT wall-clock or frame times: the
//! whole point is that the replay chooses its own frame rate.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

/// Bumped when a change would make an older reader misread a take.
/// [`Take::read`] refuses anything newer than it understands rather than
/// silently rendering something wrong.
pub const FORMAT_VERSION: u32 = 1;

/// Conventional file extension. Not enforced anywhere.
pub const EXTENSION: &str = "take";

/// What a take opens with: everything constant for the whole recording.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    /// The audio clock the event times are in.
    pub sample_rate: f32,
    /// Transport position (samples from song start) of time 0, when the
    /// host told us. Lets the take be lined up against a bounced WAV that
    /// starts somewhere else.
    #[serde(default)]
    pub start_samples: Option<u64>,
    /// The shell's UI state blob (`SharedState::save_persist`) as of the
    /// recording: view settings, camera, spectrum/roll config. This is
    /// what makes a replay *look* like what was on screen.
    ///
    /// In the plugin this is only up to date if the editor window was
    /// closed before the project was saved — the same trap
    /// `read-plugin-state.py` documents.
    #[serde(default)]
    pub ui_state: Option<String>,
    /// Editor size in logical points when recorded, as a hint for
    /// choosing the render aspect ratio.
    #[serde(default)]
    pub window_points: Option<(f32, f32)>,
    /// Free-form: which shell wrote this, and out of what.
    #[serde(default)]
    pub source: String,
}

impl Default for Header {
    fn default() -> Self {
        Header {
            version: FORMAT_VERSION,
            sample_rate: 48_000.0,
            start_samples: None,
            ui_state: None,
            window_points: None,
            source: String::new(),
        }
    }
}

/// A note event, mirroring `lattice_core::NoteEventKind`. Mirrored rather
/// than reused so this crate stays dependency-free in both directions:
/// `lattice-core` is MIT/Apache and must not gain a serde dependency (see
/// `ci.sh`), and the take format must be free to outlive an internal enum.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum NoteKind {
    On { velocity: f32 },
    Off,
    /// Per-note tuning offset in semitones (MPE / CLAP note expression).
    Tuning { semitones: f32 },
    /// Release everything (transport reset).
    AllOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteRecord {
    /// Seconds on the audio clock, from the start of the recording.
    pub t: f64,
    pub channel: u8,
    pub note: u8,
    pub kind: NoteKind,
}

/// One automatable parameter changing value. `id` is the host-facing
/// parameter id (`ParamKey::id`), so a take reads next to a project file
/// and survives an internal enum being reordered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamRecord {
    pub t: f64,
    pub id: String,
    pub value: f32,
}

/// One line of a take.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Record {
    Header(Header),
    Note(NoteRecord),
    Param(ParamRecord),
}

/// A whole take, read into memory. Takes are small — a busy ten-minute
/// piece is a few hundred kilobytes — so the reader does not stream.
#[derive(Clone, Debug, Default)]
pub struct Take {
    pub header: Header,
    /// Note events in the order they were recorded (which is time order:
    /// the audio thread stamps them from a monotonic sample counter).
    pub notes: Vec<NoteRecord>,
    /// Parameter changes, time-ordered. Only *changes* are recorded, so
    /// the value at any moment is the last record at or before it.
    pub params: Vec<ParamRecord>,
    /// The final line was incomplete, so the recording was cut off mid
    /// write — a killed export, a crash. Everything before it is intact
    /// and usable; callers should say so rather than pretend the take is
    /// whole.
    pub truncated: bool,
}

/// Why a take could not be read.
#[derive(Debug)]
pub enum ReadError {
    Io(std::io::Error),
    /// A line did not parse. Carries the 1-based line number.
    Parse(usize, ron::error::SpannedError),
    /// The file did not start with a Header record.
    MissingHeader,
    /// Written by a newer version of this format.
    Version(u32),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "{e}"),
            ReadError::Parse(line, e) => write!(f, "line {line}: {e}"),
            ReadError::MissingHeader => write!(f, "no Header record (is this a take file?)"),
            ReadError::Version(v) => write!(
                f,
                "take is format version {v}, this build understands {FORMAT_VERSION}"
            ),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        ReadError::Io(e)
    }
}

impl Take {
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Take, ReadError> {
        let file = std::fs::File::open(path)?;
        Take::parse(std::io::BufReader::new(file))
    }

    pub fn parse(input: impl BufRead) -> Result<Take, ReadError> {
        // Read the lines up front so the last one can be recognized: a
        // record that fails to parse *there* is a half-written line from
        // an interrupted export, which the format is line-oriented
        // specifically to survive. The same failure anywhere earlier is
        // real corruption and must not be waved through.
        let lines = input.lines().collect::<Result<Vec<String>, _>>()?;
        let mut take = Take::default();
        let mut have_header = false;
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            // Blank lines and `#` comments are ignored, so a take stays
            // hand-editable while debugging a render.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let record = match ron::from_str::<Record>(line) {
                Ok(record) => record,
                Err(_) if i + 1 == lines.len() => {
                    take.truncated = true;
                    break;
                }
                Err(e) => return Err(ReadError::Parse(i + 1, e)),
            };
            match record {
                Record::Header(header) => {
                    if header.version > FORMAT_VERSION {
                        return Err(ReadError::Version(header.version));
                    }
                    take.header = header;
                    have_header = true;
                }
                Record::Note(note) => take.notes.push(note),
                Record::Param(param) => take.params.push(param),
            }
        }
        if !have_header {
            return Err(ReadError::MissingHeader);
        }
        Ok(take)
    }

    /// Seconds from the first recorded event to the last. Zero for a take
    /// with nothing in it.
    pub fn duration(&self) -> f64 {
        let last_note = self.notes.last().map(|n| n.t).unwrap_or(0.0);
        let last_param = self.params.last().map(|p| p.t).unwrap_or(0.0);
        last_note.max(last_param)
    }
}

/// Appends records to a take file, one line at a time.
///
/// Deliberately dumb: no buffering beyond the OS, no background thread,
/// no batching. It is meant to be driven from a plain thread that drains
/// a ring buffer — **never** from an audio thread, which must not touch a
/// file at all. The shells own that handoff; this only knows how to write.
pub struct Writer {
    out: std::io::BufWriter<std::fs::File>,
}

impl Writer {
    /// Create (or truncate) `path` and write the header.
    pub fn create(
        path: impl AsRef<std::path::Path>,
        header: &Header,
    ) -> std::io::Result<Writer> {
        let file = std::fs::File::create(path)?;
        let mut writer = Writer { out: std::io::BufWriter::new(file) };
        writer.write(&Record::Header(header.clone()))?;
        Ok(writer)
    }

    /// Write one record, and flush it.
    ///
    /// Flushing every time is deliberate. A take is worth nothing if it
    /// is lost, and the ways a recording session ends are exactly the
    /// ways buffered data disappears: a killed export, a crashed host, a
    /// `process::exit` that skips every destructor. The line-oriented
    /// format promises that whatever reached the disk is readable — which
    /// is only true if lines actually reach the disk. The cost is one
    /// small write per event, against a few hundred events a second at
    /// the very worst.
    pub fn write(&mut self, record: &Record) -> std::io::Result<()> {
        // A record that cannot be encoded is a bug, not a runtime
        // condition — but a take is written during a long export, so drop
        // the line rather than take the whole render down with it.
        match ron::to_string(record) {
            Ok(line) => {
                writeln!(self.out, "{line}")?;
                self.out.flush()
            }
            Err(_) => Ok(()),
        }
    }

    pub fn note(&mut self, note: NoteRecord) -> std::io::Result<()> {
        self.write(&Record::Note(note))
    }

    pub fn param(&mut self, param: ParamRecord) -> std::io::Result<()> {
        self.write(&Record::Param(param))
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Header, Vec<NoteRecord>, Vec<ParamRecord>) {
        let header = Header {
            sample_rate: 44_100.0,
            start_samples: Some(1024),
            ui_state: Some("(some:\"ron\")".into()),
            window_points: Some((1000.0, 700.0)),
            source: "test".into(),
            ..Default::default()
        };
        let notes = vec![
            NoteRecord { t: 0.0, channel: 0, note: 60, kind: NoteKind::On { velocity: 0.8 } },
            NoteRecord { t: 0.5, channel: 0, note: 60, kind: NoteKind::Tuning { semitones: -0.5 } },
            NoteRecord { t: 1.0, channel: 0, note: 60, kind: NoteKind::Off },
            NoteRecord { t: 2.0, channel: 3, note: 0, kind: NoteKind::AllOff },
        ];
        let params =
            vec![ParamRecord { t: 0.0, id: "pitch-class-fade".into(), value: 2.5 }];
        (header, notes, params)
    }

    /// Write the sample take to its own file and read it back. Each
    /// caller gets a distinct path: the tests run in parallel, and a
    /// shared one raced (one test deleting the file another was reading).
    fn round_trip(name: &str) -> Take {
        let (header, notes, params) = sample();
        let dir = std::env::temp_dir().join(format!("take-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.take"));
        {
            let mut writer = Writer::create(&path, &header).unwrap();
            for note in &notes {
                writer.note(*note).unwrap();
            }
            for param in &params {
                writer.param(param.clone()).unwrap();
            }
        }
        let take = Take::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        take
    }

    #[test]
    fn a_take_round_trips_through_a_file() {
        let (header, notes, params) = sample();
        let take = round_trip("round-trip");
        assert_eq!(take.header.sample_rate, header.sample_rate);
        assert_eq!(take.header.start_samples, header.start_samples);
        assert_eq!(take.header.ui_state, header.ui_state);
        assert_eq!(take.header.window_points, header.window_points);
        assert_eq!(take.notes, notes);
        assert_eq!(take.params, params);
    }

    #[test]
    fn duration_is_the_last_thing_that_happened() {
        assert_eq!(round_trip("duration").duration(), 2.0);
    }

    /// The format is line-oriented precisely so a take cut short by a
    /// crashed export still renders everything up to the cut.
    #[test]
    fn a_truncated_take_keeps_every_whole_line() {
        let (header, notes, _) = sample();
        let mut text = ron::to_string(&Record::Header(header)).unwrap();
        text.push('\n');
        for note in &notes {
            text.push_str(&ron::to_string(&Record::Note(*note)).unwrap());
            text.push('\n');
        }
        // Chop mid-record, as a killed process would.
        let cut = text.len() - 12;
        let take = Take::parse(std::io::Cursor::new(&text.as_bytes()[..cut]))
            .unwrap_or_else(|e| panic!("truncated take should still read: {e}"));
        // The partial last line is the only casualty, and the take says so.
        assert_eq!(take.notes, notes[..notes.len() - 1]);
        assert!(take.truncated);
    }

    /// Damage anywhere but the end is real corruption — waving it
    /// through would render a piece with a hole in it and say nothing.
    #[test]
    fn a_broken_line_in_the_middle_is_an_error() {
        let (header, notes, _) = sample();
        let mut text = ron::to_string(&Record::Header(header)).unwrap();
        text.push('\n');
        text.push_str("Note((t:0.0,channel:  <- garbage\n");
        text.push_str(&ron::to_string(&Record::Note(notes[0])).unwrap());
        text.push('\n');
        assert!(matches!(
            Take::parse(std::io::Cursor::new(text.as_bytes())),
            Err(ReadError::Parse(2, _))
        ));
    }

    #[test]
    fn a_whole_take_is_not_flagged_as_truncated() {
        assert!(!round_trip("whole").truncated);
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let (header, _, _) = sample();
        let text = format!(
            "{}\n\n# a note about this take\n",
            ron::to_string(&Record::Header(header)).unwrap()
        );
        let take = Take::parse(std::io::Cursor::new(text.as_bytes())).unwrap();
        assert!(take.notes.is_empty());
    }

    #[test]
    fn a_file_without_a_header_is_rejected() {
        let note = NoteRecord { t: 0.0, channel: 0, note: 60, kind: NoteKind::Off };
        let text = ron::to_string(&Record::Note(note)).unwrap();
        assert!(matches!(
            Take::parse(std::io::Cursor::new(text.as_bytes())),
            Err(ReadError::MissingHeader)
        ));
    }

    /// A newer take must fail loudly rather than render something subtly
    /// wrong — the failure mode that would waste a whole render.
    #[test]
    fn a_take_from_the_future_is_refused() {
        let header = Header { version: FORMAT_VERSION + 1, ..Default::default() };
        let text = ron::to_string(&Record::Header(header)).unwrap();
        assert!(matches!(
            Take::parse(std::io::Cursor::new(text.as_bytes())),
            Err(ReadError::Version(_))
        ));
    }
}
