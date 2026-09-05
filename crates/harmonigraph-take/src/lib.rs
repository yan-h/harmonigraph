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
//! standalone harness while you play) and read by `harmonigraph-offline`.
//!
//! # Format
//!
//! One RON-encoded [`Record`] per line, appendable and streamable:
//!
//! ```text
//! Header((version:3,sample_rate:48000.0,...))
//! Note((t:0.5,source:0,channel:0,note:60,kind:On((velocity:0.8))))
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

pub mod canonical;
pub use canonical::{CanonicalRecord, IncompleteRecord};
pub mod configuration;
pub mod params;
pub use configuration::ConfigurationRecord;
pub mod render;

pub use params::{ParamKey, MAX_TUNING_OFFSET};
pub use render::{LatticeSide, RenderConfig, RenderFrame, RenderProgress, RenderTrigger};

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

/// Bumped when a change would make an older reader misread a take.
/// [`Take::read`] accepts exactly this version. Version 1 lacked source/reset
/// scope; version 2 lacked resolved configuration boundaries; version 3 lacked
/// canonical baselines, sample provenance and publication gaps. Refusing an old
/// header prevents its final record from looking like an interrupted write.
/// There are no compatibility shims.
pub const FORMAT_VERSION: u32 = 4;

/// Conventional file extension. Not enforced anywhere.
pub const EXTENSION: &str = "take";

/// What a take opens with: everything constant for the whole recording.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Header {
    pub version: u32,
    /// The audio clock the event times are in.
    pub sample_rate: f32,
    /// Transport position (samples from song start) of time 0, when the
    /// host told us. Lets the take be lined up against a bounced WAV that
    /// starts somewhere else.
    pub start_samples: Option<u64>,
    /// The shell's UI state blob (`SharedState::save_persist`) as of the
    /// recording: view settings, camera, spectrum/roll config. This is
    /// what makes a replay *look* like what was on screen.
    ///
    /// In the plugin this is only up to date if the editor window was
    /// closed before the project was saved — the same trap
    /// `read-plugin-state.py` documents.
    pub ui_state: Option<String>,
    /// Editor size in logical points when recorded, as a hint for
    /// choosing the render aspect ratio.
    pub window_points: Option<(f32, f32)>,
    /// Free-form: which shell wrote this, and out of what.
    pub source: String,
    /// File name (not path) of the audio recorded with this take, if
    /// any. A sibling of the take file, so the pair can be moved
    /// together. The renderer uses it when no audio is given explicitly.
    pub audio_file: Option<String>,
    /// Take time corresponding to the audio's first sample.
    ///
    /// Recording usually starts at song position 0, making this 0 — but
    /// arming mid-song does not, and without it the spectrum and the
    /// muxed track would both sit at the wrong place by exactly however
    /// far in you started.
    pub audio_start: Option<f64>,
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
            audio_file: None,
            audio_start: None,
        }
    }
}

/// A note event, mirroring `harmonigraph_core::NoteEventKind`. Mirrored rather
/// than reused even though this crate does depend on core: `harmonigraph-core`
/// is MIT/Apache and must not gain a serde dependency (see `ci.sh`), so it
/// cannot derive the impls this needs — and the take format must be free to
/// outlive an internal enum, which reusing one would forfeit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum NoteKind {
    On {
        velocity: f32,
    },
    #[default]
    Off,
    /// Per-note tuning offset in semitones (MPE / CLAP note expression).
    Tuning {
        semitones: f32,
    },
    /// Release this record's source; channel and note are ignored.
    SourceReset,
    /// Release every source; source, channel and note are ignored.
    SessionReset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteRecord {
    /// Seconds on the audio clock, from the start of the recording.
    pub t: f64,
    /// Canonical stream identity, with 0 reserved for observed direct input.
    /// This is not a saved runtime session, epoch or source-incarnation token.
    pub source: u64,
    pub channel: u8,
    pub note: u8,
    pub kind: NoteKind,
}

// Both writers and both replay modes share these conversions. Controls stay
// ordered among note deltas, at their original timestamps. Future complete
// baseline controls must extend this same stream, not a separate state feed.
impl From<harmonigraph_core::NoteEvent> for NoteRecord {
    fn from(event: harmonigraph_core::NoteEvent) -> Self {
        use harmonigraph_core::NoteEventKind as Core;
        Self {
            t: event.time,
            source: event.source.0,
            channel: event.channel,
            note: event.note,
            kind: match event.kind {
                Core::On { velocity } => NoteKind::On { velocity },
                Core::Off => NoteKind::Off,
                Core::Tuning { semitones } => NoteKind::Tuning { semitones },
                Core::SourceReset => NoteKind::SourceReset,
                Core::SessionReset => NoteKind::SessionReset,
            },
        }
    }
}

impl From<NoteRecord> for harmonigraph_core::NoteEvent {
    fn from(record: NoteRecord) -> Self {
        use harmonigraph_core::{NoteEventKind as Core, SourceId};
        Self {
            time: record.t,
            source: SourceId(record.source),
            channel: record.channel,
            note: record.note,
            kind: match record.kind {
                NoteKind::On { velocity } => Core::On { velocity },
                NoteKind::Off => Core::Off,
                NoteKind::Tuning { semitones } => Core::Tuning { semitones },
                NoteKind::SourceReset => Core::SourceReset,
                NoteKind::SessionReset => Core::SessionReset,
            },
        }
    }
}

/// One automatable parameter changing value. `id` is the host-facing
/// parameter id ([`ParamKey::id`]), so a take reads next to a project file
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
    Configuration(ConfigurationRecord),
    Canonical(CanonicalRecord),
    Incomplete(IncompleteRecord),
}

/// A whole take, read into memory. Takes are small — a busy ten-minute
/// piece is a few hundred kilobytes — so the reader does not stream.
#[derive(Clone, Debug, Default)]
pub struct Take {
    pub header: Header,
    /// Note events in the order they were recorded (which is time order:
    /// the audio thread stamps them from a monotonic sample counter).
    pub events: Vec<CanonicalRecord>,
    /// Parameter changes, time-ordered. Only *changes* are recorded, so
    /// the value at any moment is the last record at or before it.
    pub params: Vec<ParamRecord>,
    pub configurations: Vec<ConfigurationRecord>,
    /// The final line was incomplete, so the recording was cut off mid
    /// write — a killed export, a crash. Everything before it is intact
    /// and usable; callers should say so rather than pretend the take is
    /// whole.
    pub truncated: bool,
    pub incomplete: Option<IncompleteRecord>,
}

/// Why a take could not be read.
#[derive(Debug)]
pub enum ReadError {
    Io(std::io::Error),
    /// A line did not parse. Carries the 1-based line number.
    Parse(usize, ron::error::SpannedError),
    /// The file did not start with a Header record.
    MissingHeader,
    /// Written in an unsupported older or newer format.
    Version(u32),
    InvalidCanonical(usize),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "{e}"),
            ReadError::Parse(line, e) => write!(f, "line {line}: {e}"),
            ReadError::MissingHeader => write!(f, "no Header record (is this a take file?)"),
            ReadError::InvalidCanonical(line) => {
                write!(f, "line {line}: invalid complete canonical record")
            }
            ReadError::Version(v) => {
                write!(f, "take is format version {v}, this build understands {FORMAT_VERSION}")
            }
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
        let mut event_lines = Vec::new();
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
                    if header.version != FORMAT_VERSION {
                        return Err(ReadError::Version(header.version));
                    }
                    take.header = header;
                    have_header = true;
                }
                Record::Note(note) => {
                    let record = CanonicalRecord::Note(note);
                    record.validate().map_err(|_| ReadError::InvalidCanonical(i + 1))?;
                    event_lines.push(i + 1);
                    take.events.push(record);
                }
                Record::Canonical(record) => {
                    record.validate().map_err(|_| ReadError::InvalidCanonical(i + 1))?;
                    if let CanonicalRecord::Gap(gap) = &record {
                        take.incomplete = Some(IncompleteRecord {
                            first_publication: gap.first,
                            last_publication: gap.last,
                            reason: gap.reason,
                        });
                    }
                    event_lines.push(i + 1);
                    take.events.push(record);
                }
                Record::Incomplete(incomplete) => take.incomplete = Some(incomplete),
                Record::Param(param) => take.params.push(param),
                Record::Configuration(config) => take.configurations.push(config),
            }
        }
        if !have_header {
            return Err(ReadError::MissingHeader);
        }
        // Sort by time. Records are *written* in the order the audio
        // thread saw them, which is time order only while the transport
        // runs forward — a loop rollover or a jump makes the file
        // non-monotonic, and a replay that walks it in file order would
        // then stall on the first out-of-order record and deliver
        // everything after it late. Stable, so simultaneous events keep
        // the order they were played in (note-off before the note-on that
        // replaces it, and so on).
        let mut ordered: Vec<_> = take.events.drain(..).zip(event_lines).collect();
        ordered.sort_by(|(a, _), (b, _)| a.time().total_cmp(&b.time()));
        let mut validation = harmonigraph_core::NoteTracker::new();
        for (record, line) in ordered {
            record.apply(&mut validation).map_err(|_| ReadError::InvalidCanonical(line))?;
            take.events.push(record);
        }
        take.params.sort_by(|a, b| a.t.total_cmp(&b.t));
        take.configurations.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(take)
    }

    /// Seconds from the first recorded event to the last. Zero for a take
    /// with nothing in it.
    pub fn duration(&self) -> f64 {
        let last_note = self
            .events
            .iter()
            .map(|event| match event {
                CanonicalRecord::Gap(g) => g.through,
                _ => event.time(),
            })
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        let last_param = self.params.last().map(|p| p.t).unwrap_or(0.0);
        last_note.max(last_param).max(self.configurations.last().map_or(0.0, |c| c.t))
    }

    /// When recording actually began: the earliest event of any kind, or
    /// `None` for an empty take.
    ///
    /// Params count, unlike in a "first note" reading, and they are what makes
    /// this reliable: the first block after arming writes every parameter,
    /// so a take opens with a full snapshot stamped at the capture point
    /// whether or not anything was played. A take of a passage with sound but
    /// no MIDI — an audio part, a held drone, a pad recorded as audio — has
    /// no notes to anchor to, and anchoring to notes would send it back to
    /// song zero and the empty opening this exists to remove.
    ///
    /// Event times are the host's TRANSPORT position, so this is a song
    /// position, not a delay after the record button. A take of a passage
    /// starting a minute into the arrangement has its first event at 60-odd
    /// seconds, and everything before that is guaranteed empty — nothing was
    /// captured there to draw.
    pub fn first_event(&self) -> Option<f64> {
        let first_note = self.events.first().map(CanonicalRecord::time);
        let first_param = self.params.first().map(|p| p.t);
        [first_note, first_param, self.configurations.first().map(|c| c.t)]
            .into_iter()
            .flatten()
            .reduce(f64::min)
    }

    /// Derived note-only inspection. Replay consumes `events`, including every
    /// baseline/gap in its original equal-time order.
    pub fn notes(&self) -> impl Iterator<Item = NoteRecord> + '_ {
        self.events.iter().filter_map(CanonicalRecord::note)
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
    pub fn create(path: impl AsRef<std::path::Path>, header: &Header) -> std::io::Result<Writer> {
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

    pub fn canonical(&mut self, record: CanonicalRecord) -> std::io::Result<()> {
        record.validate().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid canonical record")
        })?;
        self.write(&Record::Canonical(record))
    }

    pub fn incomplete(&mut self, record: IncompleteRecord) -> std::io::Result<()> {
        self.write(&Record::Incomplete(record))
    }

    pub fn configuration(&mut self, config: ConfigurationRecord) -> std::io::Result<()> {
        self.write(&Record::Configuration(config))
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

    /// The two note-wide times — the Fade and the mark Delay — are read
    /// against each other, so the Fade has to present like the view setting
    /// above it: a linear bar, and the unit on the NUMBER rather than in the
    /// name. Pinned because the bar itself is built from these answers and
    /// nothing else would notice them drifting.
    #[test]
    fn the_fade_bar_reads_like_the_note_times_beside_it() {
        assert!(!ParamKey::Fade.logarithmic(), "a linear second, like the Delay");
        assert_eq!(ParamKey::Fade.label(), "Note fade", "the unit is not in the name");
        // The SCALE the other two answers are premised on, and the one thing
        // here with a reach past the bar: this range is what the host exposes
        // to an automation lane, so moving it re-reads every lane recorded
        // against it. The linear travel above is only defensible while a
        // Fade's whole range is one second — at a hundred, its usable settings
        // really would be crushed into the bar's first percent.
        assert_eq!(*ParamKey::Fade.range().start(), 0.0);
        assert_eq!(
            *ParamKey::Fade.range().end(),
            1.0,
            "one second, the same as the Delay bar above it",
        );
        assert_eq!(ParamKey::Fade.unit(), (1000.0, " ms", 0));
        assert!(ParamKey::Tolerance.logarithmic());
        assert_eq!(ParamKey::Tolerance.unit(), (1.0, "¢", 3));
    }

    /// Where the render starts is anchored to this, so it has to answer for a
    /// take with no notes: a passage carrying sound but no MIDI. Arming writes
    /// a full parameter snapshot regardless, so the params are what keep the
    /// anchor from falling back to song zero and reopening the empty-video bug.
    #[test]
    fn first_event_survives_a_take_with_no_notes() {
        let take = |notes: Vec<NoteRecord>, params: Vec<ParamRecord>| Take {
            header: Header::default(),
            events: notes.into_iter().map(CanonicalRecord::Note).collect(),
            params,
            configurations: Vec::new(),
            truncated: false,
            incomplete: None,
        };
        let note = |t| NoteRecord {
            source: 0,
            t,
            channel: 0,
            note: 60,
            kind: NoteKind::On { velocity: 0.8 },
        };
        let param = |t| ParamRecord { t, id: "pitch-class-fade".into(), value: 1.0 };

        // Notes and params: the earlier wins, and it is the param snapshot
        // that arming wrote — which is the real capture point.
        assert_eq!(take(vec![note(5.87)], vec![param(5.48)]).first_event(), Some(5.48));
        // Sound but no MIDI: the snapshot is the only anchor there is.
        assert_eq!(take(vec![], vec![param(5.48)]).first_event(), Some(5.48));
        // Notes but no params, which a hand-built take could be.
        assert_eq!(take(vec![note(5.87)], vec![]).first_event(), Some(5.87));
        // Nothing at all.
        assert_eq!(take(vec![], vec![]).first_event(), None);
    }

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
            NoteRecord {
                source: 0,
                t: 0.0,
                channel: 0,
                note: 60,
                kind: NoteKind::On { velocity: 0.8 },
            },
            NoteRecord {
                source: 0,
                t: 0.5,
                channel: 0,
                note: 60,
                kind: NoteKind::Tuning { semitones: -0.5 },
            },
            NoteRecord { source: 0, t: 1.0, channel: 0, note: 60, kind: NoteKind::Off },
            NoteRecord { source: 0, t: 2.0, channel: 3, note: 0, kind: NoteKind::SessionReset },
        ];
        let params = vec![ParamRecord { t: 0.0, id: "pitch-class-fade".into(), value: 2.5 }];
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
        assert_eq!(take.notes().collect::<Vec<_>>(), notes);
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
        assert_eq!(take.notes().collect::<Vec<_>>(), notes[..notes.len() - 1]);
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

    /// A transport loop writes records out of order. The reader has to
    /// put them back, or the replay stalls on the first rollover and
    /// delivers the whole rest of the take in one frame.
    #[test]
    fn records_are_sorted_by_time_however_they_were_written() {
        let header = Header::default();
        let out_of_order = [5.0, 1.0, 7.0, 0.5, 3.0];
        let mut text = ron::to_string(&Record::Header(header)).unwrap();
        text.push('\n');
        for t in out_of_order {
            let note = NoteRecord { source: 0, t, channel: 0, note: 60, kind: NoteKind::Off };
            text.push_str(&ron::to_string(&Record::Note(note)).unwrap());
            text.push('\n');
            let param = ParamRecord { t, id: "pitch-class-fade".into(), value: t as f32 };
            text.push_str(&ron::to_string(&Record::Param(param)).unwrap());
            text.push('\n');
        }
        let take = Take::parse(std::io::Cursor::new(text.as_bytes())).unwrap();
        let note_times: Vec<f64> = take.notes().map(|n| n.t).collect();
        let param_times: Vec<f64> = take.params.iter().map(|p| p.t).collect();
        assert_eq!(note_times, vec![0.5, 1.0, 3.0, 5.0, 7.0]);
        assert_eq!(param_times, vec![0.5, 1.0, 3.0, 5.0, 7.0]);
    }

    /// Simultaneous events must keep the order they were played in: a
    /// note-off and the note-on replacing it land on the same timestamp,
    /// and swapping them would leave the voice silent.
    #[test]
    fn simultaneous_records_keep_their_written_order() {
        let header = Header::default();
        let mut text = ron::to_string(&Record::Header(header)).unwrap();
        text.push('\n');
        for kind in [NoteKind::Off, NoteKind::On { velocity: 0.5 }] {
            let note = NoteRecord { source: 0, t: 2.0, channel: 0, note: 60, kind };
            text.push_str(&ron::to_string(&Record::Note(note)).unwrap());
            text.push('\n');
        }
        let take = Take::parse(std::io::Cursor::new(text.as_bytes())).unwrap();
        assert_eq!(take.notes().next().unwrap().kind, NoteKind::Off);
        assert!(matches!(take.notes().nth(1).unwrap().kind, NoteKind::On { .. }));
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
        assert!(take.notes().next().is_none());
    }

    #[test]
    fn a_file_without_a_header_is_rejected() {
        let note = NoteRecord { source: 0, t: 0.0, channel: 0, note: 60, kind: NoteKind::Off };
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

    #[test]
    fn invalid_complete_final_baseline_and_out_of_order_cut_are_refused() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::{NoteEvent, SourceId};
        let row = VoiceBaseline {
            note: 60,
            velocity: 0.8,
            pitch_microcents: 6_000_000_000,
            ..Default::default()
        };
        let baseline = SourceBaseline::new(
            SourceId::DIRECT,
            1,
            2.0,
            0.0,
            5,
            true,
            &[row],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let mut record = canonical::BaselineRecord::from(&baseline);
        record.voices = vec![row.into(); 65];
        let header = ron::to_string(&Record::Header(Header::default())).unwrap();
        let invalid =
            ron::to_string(&Record::Canonical(CanonicalRecord::Baseline(Box::new(record))))
                .unwrap();
        let text = format!("{header}\n{invalid}");
        assert!(matches!(
            Take::parse(std::io::Cursor::new(text)),
            Err(ReadError::InvalidCanonical(2))
        ));
        let frame = ron::to_string(&Record::Canonical(CanonicalRecord::from_event(
            CanonicalEvent::Baseline(&baseline),
        )))
        .unwrap();
        let mut delta: NoteDelta = NoteEvent::on(3.0, SourceId::DIRECT, 0, 60, 0.8).into();
        delta.sequence = 4;
        let later = ron::to_string(&Record::Canonical(CanonicalRecord::from_event(
            CanonicalEvent::Note(delta),
        )))
        .unwrap();
        assert!(matches!(
            Take::parse(std::io::Cursor::new(format!("{header}\n{frame}\n{later}"))),
            Err(ReadError::InvalidCanonical(3))
        ));
    }

    #[test]
    fn an_old_header_is_refused_before_a_final_record_can_look_truncated() {
        for version in [0, 1, 2, 3] {
            for last in ["Note((t:0.0,channel:0,note:60,kind:On((velocity:0.8))))", "Note((t:"] {
                let text = format!("Header((version:{version},sample_rate:48000.0))\n{last}");
                let error = Take::parse(std::io::Cursor::new(text)).unwrap_err();
                assert!(matches!(error, ReadError::Version(v) if v == version));
                assert!(error.to_string().contains(&format!("format version {version}")));
            }
        }
    }
}

/// Streams a 32-bit-float WAV alongside a take.
///
/// A take's notes tell you what the lattice does; the audio tells you
/// what it sounded like, and the renderer needs both — one to draw the
/// spectrum, the other to put in the video. Recording it here rather
/// than asking for a separate DAW bounce is what makes the whole thing
/// one gesture.
///
/// Float rather than 16-bit PCM: no conversion, no dither, and nothing
/// to clip if the bus is hot. The size fields are patched on close, so a
/// file from a crashed session is still readable by anything that
/// tolerates a short RIFF size — and `harmonigraph-offline`'s reader
/// deliberately does.
pub struct WavWriter {
    file: std::fs::File,
    channels: u16,
    frames: u32,
}

impl WavWriter {
    /// 32-bit IEEE float.
    const BITS: u16 = 32;
    const FORMAT_FLOAT: u16 = 3;
    const HEADER_BYTES: u32 = 44;

    pub fn create(
        path: impl AsRef<std::path::Path>,
        sample_rate: f32,
        channels: u16,
    ) -> std::io::Result<WavWriter> {
        let mut file = std::fs::File::create(path)?;
        let channels = channels.max(1);
        let rate = sample_rate.max(1.0) as u32;
        let block_align = channels * (Self::BITS / 8);

        let mut header = Vec::with_capacity(Self::HEADER_BYTES as usize);
        header.extend(b"RIFF");
        header.extend(0u32.to_le_bytes()); // patched by finish()
        header.extend(b"WAVE");
        header.extend(b"fmt ");
        header.extend(16u32.to_le_bytes());
        header.extend(Self::FORMAT_FLOAT.to_le_bytes());
        header.extend(channels.to_le_bytes());
        header.extend(rate.to_le_bytes());
        header.extend((rate * u32::from(block_align)).to_le_bytes());
        header.extend(block_align.to_le_bytes());
        header.extend(Self::BITS.to_le_bytes());
        header.extend(b"data");
        header.extend(0u32.to_le_bytes()); // patched by finish()
        std::io::Write::write_all(&mut file, &header)?;

        Ok(WavWriter { file, channels, frames: 0 })
    }

    /// Append interleaved samples. A partial frame at the end of a chunk
    /// is impossible in practice (blocks are whole frames) and would
    /// desync the channels, so the count is taken in whole frames.
    pub fn write(&mut self, interleaved: &[f32]) -> std::io::Result<()> {
        if interleaved.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(interleaved.len() * 4);
        for sample in interleaved {
            bytes.extend(sample.to_le_bytes());
        }
        std::io::Write::write_all(&mut self.file, &bytes)?;
        self.frames += (interleaved.len() / usize::from(self.channels)) as u32;
        Ok(())
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Patch the two size fields and close.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.patch()
    }

    fn patch(&mut self) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let data_bytes = self.frames * u32::from(self.channels) * u32::from(Self::BITS / 8);
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(Self::HEADER_BYTES - 8 + data_bytes).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&data_bytes.to_le_bytes())?;
        self.file.flush()
    }
}

impl Drop for WavWriter {
    fn drop(&mut self) {
        // A take cut short by a crash still gets valid sizes if the
        // process unwinds at all; `finish` is the path that reports why
        // when it doesn't.
        let _ = self.patch();
    }
}

#[cfg(test)]
mod wav_tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("take-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_written_wav_has_the_sizes_and_samples_it_claims() {
        let path = temp("basic.wav");
        {
            let mut writer = WavWriter::create(&path, 48_000.0, 2).unwrap();
            writer.write(&[0.5, -0.5, 0.25, -0.25]).unwrap();
            writer.write(&[1.0, -1.0]).unwrap();
            assert_eq!(writer.frames(), 3);
            writer.finish().unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(data, 3 * 2 * 4, "3 stereo float frames");
        assert_eq!(riff as usize, bytes.len() - 8);
        // Format tag 3 = IEEE float, 32-bit.
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 32);
        // The samples round-trip bit-exactly, which is the point of float.
        let first = f32::from_le_bytes(bytes[44..48].try_into().unwrap());
        assert_eq!(first, 0.5);
    }

    /// Dropping without `finish` still patches the sizes: a session that
    /// ends unexpectedly should leave a playable file, not a header of
    /// zeros claiming an empty stream.
    #[test]
    fn dropping_the_writer_still_patches_the_sizes() {
        let path = temp("dropped.wav");
        {
            let mut writer = WavWriter::create(&path, 44_100.0, 1).unwrap();
            writer.write(&[0.1, 0.2, 0.3]).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 3 * 4);
    }
}
