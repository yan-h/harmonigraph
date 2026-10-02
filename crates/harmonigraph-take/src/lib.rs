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
//! Header((version:6,sample_rate:48000.0,...))
//! Note((t:0.5,source:0,channel:0,note:60,kind:On(velocity:0.8)))
//! Param((t:0.0,id:"pitch-class-fade",value:2.0))
//! ```
//!
//! Line-oriented rather than one big document for three reasons: the
//! writer can append as the export runs without holding the whole take in
//! memory; a take truncated by a crash keeps every line written before it,
//! which is all but the writer's last unflushed batch of records; and a
//! take is greppable and hand-editable, which matters a lot the first
//! time a render comes out wrong.
//!
//! Times are **seconds of host transport position** — the song position,
//! not a count from the record button — which is the same clock the plugin
//! stamps its note events with. They are deliberately NOT wall-clock or
//! frame times: the whole point is that the replay chooses its own frame
//! rate.

pub mod canonical;
pub use canonical::{CanonicalRecord, IncompleteRecord};
pub mod configuration;
pub mod params;
pub use configuration::ConfigurationRecord;
pub mod render;

pub use params::{ParamKey, MAX_TUNING_OFFSET};
pub use render::{
    LatticeSide, RenderConfig, RenderFrame, RenderProgress, RenderTrigger, SpectrogramRender,
    STOP_BAR_RANGE,
};

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

/// Bumped when a change would make an older reader misread a take.
/// [`Take::read`] accepts exactly this version. Version 1 lacked source/reset
/// scope; version 2 lacked resolved configuration boundaries; version 3 lacked
/// canonical baselines, sample provenance and publication gaps. Refusing an old
/// header prevents its final record from looking like an interrupted write.
/// Version 4 carried full editor persistence instead of dedicated appearance.
/// Version 5 named a baseline's Show flag `participating`; read under the new
/// key `shown` it defaults to false and hides every source, so the rename is a
/// version rather than a silent blank render. There are no compatibility shims.
pub const FORMAT_VERSION: u32 = 6;

/// Conventional file extension. Not enforced anywhere.
pub const EXTENSION: &str = "take";

/// What a take opens with: everything constant for the whole recording.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Header {
    pub version: u32,
    /// The audio clock the event times are in.
    pub sample_rate: f32,
    /// Opaque serialized appearance at capture start. The UI owns its schema;
    /// take/record transport it without depending on UI or graphics types.
    pub appearance: Option<String>,
    /// Free-form: which shell wrote this, and out of what.
    pub source: String,
    /// File name (not path) of the audio recorded with this take, if
    /// any. A sibling of the take file, so the pair can be moved
    /// together. The renderer uses it for the spectrum and soundtrack.
    pub audio_file: Option<String>,
    /// Take time corresponding to the audio's first sample.
    ///
    /// Recording usually starts at song position 0, making this 0 — but
    /// arming mid-song does not, and without it the spectrum and the
    /// muxed track would both sit at the wrong place by exactly however
    /// far in you started.
    pub audio_start: Option<f64>,
}

impl Header {
    /// Read only the header when queueing a look, without loading a take's events.
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Self, ReadError> {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        // Appearance documents are small; cap malformed input on this UI path.
        for (index, line) in std::io::BufReader::new(file.take(8 * 1024 * 1024)).lines().enumerate()
        {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            return match ron::from_str::<Record>(line)
                .map_err(|e| ReadError::Parse(index + 1, e))?
            {
                Record::Header(header) if header.version == FORMAT_VERSION => Ok(header),
                Record::Header(header) => Err(ReadError::Version(header.version)),
                _ => Err(ReadError::MissingHeader),
            };
        }
        Err(ReadError::MissingHeader)
    }
}

impl Default for Header {
    fn default() -> Self {
        Header {
            version: FORMAT_VERSION,
            sample_rate: 48_000.0,
            appearance: None,
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
    /// Any other per-note expression, in the units the host sent it in.
    Expression {
        expression: ExpressionKind,
        value: f32,
    },
    /// Release this record's source; channel and note are ignored.
    SourceReset,
    /// Release every source; source, channel and note are ignored.
    SessionReset,
}

/// Mirrors `harmonigraph_core::Expression`, for the reason [`NoteKind`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpressionKind {
    Pressure,
    Gain,
    Timbre,
}

impl From<harmonigraph_core::Expression> for ExpressionKind {
    fn from(expression: harmonigraph_core::Expression) -> Self {
        use harmonigraph_core::Expression as Core;
        match expression {
            Core::Pressure => Self::Pressure,
            Core::Gain => Self::Gain,
            Core::Timbre => Self::Timbre,
        }
    }
}

impl From<ExpressionKind> for harmonigraph_core::Expression {
    fn from(expression: ExpressionKind) -> Self {
        match expression {
            ExpressionKind::Pressure => Self::Pressure,
            ExpressionKind::Gain => Self::Gain,
            ExpressionKind::Timbre => Self::Timbre,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteRecord {
    /// Seconds of host transport position (see the module doc).
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
                Core::Expression { expression, value } => {
                    NoteKind::Expression { expression: expression.into(), value }
                }
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
                NoteKind::Expression { expression, value } => {
                    Core::Expression { expression: expression.into(), value }
                }
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

/// A whole take, read into memory. Per-block expression records can make
/// long expressive performances large; parsing retains the complete history.
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
    /// The hole in the note history this take carries, if it has one: the
    /// FIRST gap the file names, whether it named it as an `Incomplete` marker
    /// or as a `Gap` among the events.
    ///
    /// First rather than last because that is the rule the WRITER already
    /// applies — `Open::mark_incomplete` in `harmonigraph-record` writes its
    /// marker once per recording and every later gap leaves it alone — and the
    /// two have to agree. They did not: the reader took the last line, so a
    /// take with two gaps was exported with a warning naming a different gap
    /// from the one its own marker held (#895, correctness item 4).
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

/// Check syntax independently of Record's typed deserialization. RON can
/// report an expected closing delimiter/comma at EOF instead of Error::Eof.
fn unfinished_ron(line: &str) -> bool {
    use ron::error::Error;
    let Ok(mut parser) = ron::Deserializer::from_str(line) else { return false };
    match <serde::de::IgnoredAny as serde::Deserialize>::deserialize(&mut parser) {
        // RON searches for the closing quote before consuming string contents,
        // so a genuine string EOF can leave a nonempty remainder.
        Err(Error::Eof | Error::ExpectedStringEnd) => true,
        Err(
            Error::ExpectedArrayEnd
            | Error::ExpectedMapEnd
            | Error::ExpectedStructLikeEnd
            | Error::ExpectedComma
            | Error::ExpectedMapColon
            | Error::ExpectedIdentifier,
        ) => parser.remainder().is_empty(),
        _ => false,
    }
}

impl Take {
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Take, ReadError> {
        let file = std::fs::File::open(path)?;
        Take::parse(std::io::BufReader::new(file))
    }

    pub fn parse(input: impl BufRead) -> Result<Take, ReadError> {
        // Keep one line of lookahead to recognize the physical last line: a
        // syntactically unfinished final record can be a half-written line.
        // A complete record with an invalid type/value is corruption even there.
        let mut lines = input.lines().enumerate().peekable();
        let mut take = Take::default();
        let mut have_header = false;
        let mut event_lines = Vec::new();
        while let Some((i, line)) = lines.next() {
            let line = line?;
            let line = line.trim();
            // Blank lines and `#` comments are ignored, so a take stays
            // hand-editable while debugging a render.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let record = match ron::from_str::<Record>(line) {
                Ok(record) => record,
                Err(_) if lines.peek().is_none() && unfinished_ron(line) => {
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
                        // First gap wins, like the writer's marker; see
                        // [`Take::incomplete`].
                        take.incomplete.get_or_insert(IncompleteRecord {
                            first_publication: gap.first,
                            last_publication: gap.last,
                            reason: gap.reason,
                        });
                    }
                    event_lines.push(i + 1);
                    take.events.push(record);
                }
                Record::Incomplete(incomplete) => {
                    take.incomplete.get_or_insert(incomplete);
                }
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
        let mut validation = harmonigraph_core::canonical::CanonicalOrder::default();
        for (record, line) in ordered {
            record.check_order(&mut validation).map_err(|_| ReadError::InvalidCanonical(line))?;
            take.events.push(record);
        }
        take.params.sort_by(|a, b| a.t.total_cmp(&b.t));
        take.configurations.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(take)
    }

    /// The transport position of the last recorded event (a gap counts to
    /// where it runs through): the take's absolute end, not its length. Zero
    /// for a take with nothing in it.
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
/// Buffers file writes, with flushing controlled by the caller.
/// It is meant to be driven from a plain thread that drains
/// a ring buffer — **never** from an audio thread, which must not touch a
/// file at all. The shells own that handoff; this only knows how to write.
pub struct Writer {
    out: std::io::BufWriter<std::fs::File>,
}

impl Writer {
    /// Create (or truncate) `path` and write the header.
    pub fn create(path: impl AsRef<std::path::Path>, header: &Header) -> std::io::Result<Writer> {
        Self::from_file(std::fs::File::create(path)?, header)
    }

    /// Initialize a caller-owned file, allowing exclusive creation at the recording boundary.
    pub fn from_file(file: std::fs::File, header: &Header) -> std::io::Result<Writer> {
        let mut writer = Writer { out: std::io::BufWriter::new(file) };
        writer.write(&Record::Header(header.clone()))?;
        writer.flush()?;
        Ok(writer)
    }

    /// Make the real buffered file writes fail when flushed, after any prefix
    /// already written. The replacement handle has no write permission.
    #[cfg(feature = "test-support")]
    pub fn make_read_only_for_test(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<()> {
        self.out.flush()?;
        self.out = std::io::BufWriter::new(std::fs::File::open(path)?);
        Ok(())
    }

    /// Buffer one record. The recorder flushes at each worker drain pass,
    /// rather than paying a file write for every per-block expression.
    /// Callers must flush before reading or reporting successful completion;
    /// dropping the writer also attempts a flush, but cannot report I/O errors.
    pub fn write(&mut self, record: &Record) -> std::io::Result<()> {
        // A record that cannot be encoded is a bug, not a runtime
        // condition — but a take is written during a long export, so drop
        // the line rather than take the whole render down with it.
        match ron::to_string(record) {
            Ok(line) => writeln!(self.out, "{line}"),
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
            appearance: Some("(some:\"ron\")".into()),
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
            NoteRecord {
                source: 0,
                t: 0.75,
                channel: 0,
                note: 60,
                kind: NoteKind::Expression { expression: ExpressionKind::Pressure, value: 0.4 },
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
        assert_eq!(take.header.appearance, header.appearance);
        assert_eq!(take.header.source, header.source);
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

        // A real parameter line cut inside a NONEMPTY string reaches RON's
        // ExpectedStringEnd path, which leaves the unterminated text unconsumed.
        let (header, notes, params) = sample();
        let parameter = ron::to_string(&Record::Param(params[0].clone())).unwrap();
        let inside_id = parameter.find("id:\"").unwrap() + "id:\"".len() + 3;
        let text = format!(
            "{}\n{}\n{}",
            ron::to_string(&Record::Header(header)).unwrap(),
            ron::to_string(&Record::Note(notes[0])).unwrap(),
            &parameter[..inside_id]
        );
        let take = Take::parse(std::io::Cursor::new(text)).unwrap();
        assert_eq!(take.notes().collect::<Vec<_>>(), notes[..1]);
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
    fn an_unfinished_record_before_trailing_lines_is_corruption() {
        let header = ron::to_string(&Record::Header(Header::default())).unwrap();
        for suffix in ["\n", "# trailing comment\n"] {
            let text = format!("{header}\nNote((t:0.0,\n{suffix}");
            assert!(matches!(Take::parse(std::io::Cursor::new(text)), Err(ReadError::Parse(2, _))));
        }
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
    fn canonical_order_uses_stable_time_order_but_payload_errors_keep_file_precedence() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::{NoteEvent, SourceId};
        let header = ron::to_string(&Record::Header(Header::default())).unwrap();
        let serialize =
            |event| ron::to_string(&Record::Canonical(CanonicalRecord::from_event(event))).unwrap();
        let baseline = SourceBaseline::new(SourceId::DIRECT, 1, 2.0, 5, true, &[]).unwrap();
        let frame = serialize(CanonicalEvent::Baseline(&baseline));
        let mut delta: NoteDelta = NoteEvent::on(2.0, SourceId::DIRECT, 0, 60, 0.8).into();
        delta.sequence = 4;
        let note = serialize(CanonicalEvent::Note(delta));
        let mut later = delta;
        later.sequence = 6;
        later.event.time = 3.0;
        let later = serialize(CanonicalEvent::Note(later));
        let take = Take::parse(std::io::Cursor::new(format!(
            "{header}\n{later}\n{note}\n{frame}\n{note}\n"
        )))
        .unwrap();
        assert_eq!(take.events.len(), 4, "validation does not remove duplicate records");
        assert!(matches!(take.events[0], CanonicalRecord::Delta(_)));
        assert!(matches!(take.events[1], CanonicalRecord::Baseline(_)));
        assert_eq!(take.events.last().unwrap().time(), 3.0);
        assert!(matches!(
            Take::parse(std::io::Cursor::new(format!("{header}\n{frame}\n{note}\n"))),
            Err(ReadError::InvalidCanonical(3))
        ));

        delta.event.time = 3.0;
        let late_history = serialize(CanonicalEvent::Note(delta));
        let invalid_order = format!("{header}\n{late_history}\n{frame}\n");
        assert!(matches!(
            Take::parse(std::io::Cursor::new(&invalid_order)),
            Err(ReadError::InvalidCanonical(2))
        ));
        delta.event.kind = harmonigraph_core::NoteEventKind::On { velocity: 2.0 };
        let malformed_duplicate = serialize(CanonicalEvent::Note(delta));
        assert!(matches!(
            Take::parse(std::io::Cursor::new(format!(
                "{header}\n{note}\n{frame}\n{malformed_duplicate}\n"
            ))),
            Err(ReadError::InvalidCanonical(4))
        ));
        assert!(matches!(
            Take::parse(std::io::Cursor::new(format!("{invalid_order}{malformed_duplicate}\n"))),
            Err(ReadError::InvalidCanonical(4))
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
        let baseline = SourceBaseline::new(SourceId::DIRECT, 1, 2.0, 5, true, &[row]).unwrap();
        let full = ron::to_string(&Record::Canonical(CanonicalRecord::from_event(
            CanonicalEvent::Baseline(&baseline),
        )))
        .unwrap();
        let header = ron::to_string(&Record::Header(Header::default())).unwrap();
        for invalid in [
            full.replace("note:60", "note:256"),
            full.replace("ObservedDirect", "UnknownProvenance"),
        ] {
            assert_ne!(invalid, full);
            let parsed = Take::parse(std::io::Cursor::new(format!("{header}\n{invalid}")));
            assert!(
                matches!(parsed, Err(ReadError::Parse(2, _))),
                "complete malformed record was accepted: {parsed:?}"
            );
        }
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

    /// A version-4 take written before a field was pruned still reads, and
    /// reads the same.
    ///
    /// Every record carries a container-level `#[serde(default)]` and none of
    /// them denies unknown fields, so a key the reader no longer has costs that
    /// key alone — the same rule the UI blob lives by. That is the whole of
    /// what makes dropping a field from the take format safe to do without
    /// touching `FORMAT_VERSION`, and it is worth pinning rather than
    /// remembering: `deny_unknown_fields` on any of these would turn every
    /// pruned field into a take nobody can open.
    ///
    /// The keys below are the real ones and in the real shape the pre-prune
    /// writer emitted them in — `coverage_start` after `t`, sixteen `channels`
    /// of 128 controllers after `voices`, `configuration_revision` first inside
    /// an assignment, `start_samples` after `sample_rate` and `window_points`
    /// after `appearance` (#895 dropped those two). A hand-shortened stand-in
    /// would prove nothing about the nested vector RON actually has to skip.
    ///
    /// The HEADER's two matter for a second reason the others do not have: the
    /// version check reads out of the header, so a header that failed to parse
    /// would be refused as a bad line rather than defaulted, and every take
    /// already on disk carries both keys.
    #[test]
    fn a_take_carrying_a_since_pruned_field_still_reads() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::{NoteEvent, SourceId};
        let row = VoiceBaseline {
            note: 60,
            velocity: 0.8,
            pitch_microcents: 6_000_000_000,
            ..Default::default()
        };
        let baseline = SourceBaseline::new(SourceId::DIRECT, 1, 2.0, 5, true, &[row]).unwrap();
        let frame = ron::to_string(&Record::Canonical(CanonicalRecord::from_event(
            CanonicalEvent::Baseline(&baseline),
        )))
        .unwrap();
        let mut delta: NoteDelta = NoteEvent::on(3.0, SourceId::DIRECT, 0, 60, 0.8).into();
        delta.assignment = Some(AssignmentMetadata {
            decision: 7,
            node: None,
            correction_microcents: 0,
            player_tuning: 0.0,
        });
        let note = ron::to_string(&Record::Canonical(CanonicalRecord::from_event(
            CanonicalEvent::Note(delta),
        )))
        .unwrap();
        let header = ron::to_string(&Record::Header(Header::default())).unwrap();
        let current = Take::parse(std::io::Cursor::new(format!("{header}\n{frame}\n{note}")))
            .expect("the fixture has to parse before the aged one means anything");

        // What the pre-prune writer put in the same two records.
        let channel = format!(
            "(controllers:[{}],controller_valid:(0,0),pitch_bend:None,pressure:None,program:None)",
            ["0"; 128].join(",")
        );
        let channels = format!(",channels:[{}]", vec![channel; 16].join(","));
        let aged_frame = frame
            .replace("t:2.0,", "t:2.0,coverage_start:2.0,")
            .replace(")))", &format!("{channels})))"));
        let aged_note =
            note.replace("assignment:Some((", "assignment:Some((configuration_revision:9,");
        let aged_header = header
            .replace("sample_rate:48000.0,", "sample_rate:48000.0,start_samples:Some(1024),")
            .replace("appearance:None,", "appearance:None,window_points:Some((1000.0,700.0)),");
        // Each key named separately, because one `assert_ne!` over the whole
        // line passes when only one of the replacements landed — and each was
        // checked alone against `deny_unknown_fields`.
        assert!(aged_frame.contains("coverage_start:2.0"), "{aged_frame}");
        assert!(aged_frame.contains(",channels:[(controllers:[0,"), "{aged_frame}");
        assert!(aged_note.contains("configuration_revision:9"), "{aged_note}");
        assert!(aged_header.contains("start_samples:Some(1024)"), "{aged_header}");
        assert!(aged_header.contains("window_points:Some((1000.0,700.0))"), "{aged_header}");
        let aged =
            Take::parse(std::io::Cursor::new(format!("{aged_header}\n{aged_frame}\n{aged_note}")))
                .expect("a take carrying pruned keys still opens");
        assert_eq!(aged.events, current.events, "and draws exactly what it drew");
        assert_eq!(aged.header.version, current.header.version, "and its header still reads");
    }

    #[test]
    fn an_old_header_is_refused_before_a_final_record_can_look_truncated() {
        for version in 0..FORMAT_VERSION {
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
    frames: u64,
    max_frames: u64,
    failed: bool,
    bytes: Vec<u8>,
    #[cfg(feature = "test-support")]
    fail_finish: bool,
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
        Self::with_file(|| std::fs::File::create(path), sample_rate, channels)
    }

    /// Initialize a caller-owned file, allowing exclusive creation at the recording boundary.
    pub fn from_file(
        file: std::fs::File,
        sample_rate: f32,
        channels: u16,
    ) -> std::io::Result<WavWriter> {
        Self::with_file(|| Ok(file), sample_rate, channels)
    }

    fn with_file(
        open: impl FnOnce() -> std::io::Result<std::fs::File>,
        sample_rate: f32,
        channels: u16,
    ) -> std::io::Result<WavWriter> {
        let channels = channels.max(1);
        let rate = sample_rate.max(1.0) as u32;
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "WAV format exceeds RIFF header limits",
            )
        };
        let block_align = channels.checked_mul(Self::BITS / 8).ok_or_else(invalid)?;
        let byte_rate = rate.checked_mul(u32::from(block_align)).ok_or_else(invalid)?;
        let max_frames = u64::from(u32::MAX - (Self::HEADER_BYTES - 8)) / u64::from(block_align);
        let mut file = open()?;
        let mut header = Vec::with_capacity(Self::HEADER_BYTES as usize);
        header.extend(b"RIFF");
        header.extend(0u32.to_le_bytes()); // patched by finish()
        header.extend(b"WAVE");
        header.extend(b"fmt ");
        header.extend(16u32.to_le_bytes());
        header.extend(Self::FORMAT_FLOAT.to_le_bytes());
        header.extend(channels.to_le_bytes());
        header.extend(rate.to_le_bytes());
        header.extend(byte_rate.to_le_bytes());
        header.extend(block_align.to_le_bytes());
        header.extend(Self::BITS.to_le_bytes());
        header.extend(b"data");
        header.extend(0u32.to_le_bytes()); // patched by finish()
        std::io::Write::write_all(&mut file, &header)?;

        Ok(WavWriter {
            file,
            channels,
            frames: 0,
            max_frames,
            failed: false,
            bytes: Vec::new(),
            #[cfg(feature = "test-support")]
            fail_finish: false,
        })
    }

    /// Append whole interleaved frames. Refuse an oversized chunk before
    /// writing any of it, keeping the existing prefix within standard RIFF.
    /// A failed writer accepts no further samples; `finish` repairs its prefix.
    pub fn write(&mut self, interleaved: &[f32]) -> std::io::Result<()> {
        self.append(interleaved, std::io::Write::write_all)
    }

    fn append(
        &mut self,
        interleaved: &[f32],
        write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        if self.failed {
            return Err(std::io::Error::other("WAV recording already failed"));
        }
        if interleaved.is_empty() {
            return Ok(());
        }
        let frames =
            self.frames.checked_add((interleaved.len() / usize::from(self.channels)) as u64);
        if !interleaved.len().is_multiple_of(usize::from(self.channels))
            || frames.is_none_or(|frames| frames > self.max_frames)
        {
            self.failed = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "WAV requires whole frames within the 4 GiB RIFF size limit",
            ));
        }
        self.bytes.clear();
        self.bytes.reserve(interleaved.len() * 4);
        for sample in interleaved {
            self.bytes.extend(sample.to_le_bytes());
        }
        if let Err(error) = write(&mut self.file, &self.bytes) {
            self.failed = true;
            return Err(error);
        }
        self.frames = frames.unwrap();
        Ok(())
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn limit_frames_for_test(&mut self, frames: u64) {
        self.max_frames = self.max_frames.min(frames);
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn fail_finish_for_test(&mut self) {
        self.fail_finish = true;
    }

    /// Patch the two size fields and close.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.patch()
    }

    fn patch(&mut self) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        #[cfg(feature = "test-support")]
        if self.fail_finish {
            return Err(std::io::Error::other("injected WAV finalization failure"));
        }
        let (data_bytes, riff_bytes) = Self::sizes(self.frames, self.channels)?;
        // A failed write_all may have appended a partial chunk. Only frames
        // whose entire write succeeded belong to the file's declared prefix.
        self.file.set_len(u64::from(Self::HEADER_BYTES) + u64::from(data_bytes))?;
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&riff_bytes.to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&data_bytes.to_le_bytes())?;
        self.file.flush()
    }

    fn sizes(frames: u64, channels: u16) -> std::io::Result<(u32, u32)> {
        let data = frames.checked_mul(u64::from(channels)).and_then(|n| n.checked_mul(4));
        let sizes = data.and_then(|data| {
            let riff = data.checked_add(u64::from(Self::HEADER_BYTES - 8))?;
            Some((u32::try_from(data).ok()?, u32::try_from(riff).ok()?))
        });
        sizes.ok_or_else(|| std::io::Error::other("WAV exceeds the 4 GiB RIFF size limit"))
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

    #[test]
    fn wav_limit_includes_the_riff_header_and_whole_frames() {
        let max = (u64::from(u32::MAX) - 36) / 8;
        assert_eq!(WavWriter::sizes(max, 2).unwrap(), (4_294_967_256, 4_294_967_292));
        assert!(WavWriter::sizes(max - 1, 2).is_ok());
        assert!((max + 1) * 8 <= u64::from(u32::MAX), "data still fits, RIFF does not");
        assert!(WavWriter::sizes(max + 1, 2).is_err());
        assert!(WavWriter::sizes(u64::MAX, 2).is_err());
    }

    #[test]
    fn wav_limit_refuses_a_chunk_and_preserves_its_playable_prefix() {
        let path = temp("limited.wav");
        let mut writer = WavWriter::create(&path, 48_000.0, 2).unwrap();
        writer.limit_frames_for_test(3);
        writer.write(&[0.5, -0.5, 0.25, -0.25]).unwrap();
        assert!(writer.write(&[1.0; 4]).is_err());
        assert!(writer.write(&[1.0; 2]).is_err(), "a failed recording cannot resume");
        assert_eq!(writer.frames(), 2);
        writer.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 16);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 16);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
        assert_eq!(f32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0.5);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn wav_accepts_its_exact_frame_limit_and_refuses_partial_frames() {
        let path = temp("whole-frames.wav");
        let mut writer = WavWriter::create(&path, 48_000.0, 2).unwrap();
        writer.limit_frames_for_test(1);
        writer.write(&[0.5, -0.5]).unwrap();
        assert_eq!(writer.frames(), 1);
        assert!(writer.write(&[1.0]).is_err());
        writer.finish().unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 8);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unsupported_wav_header_values_do_not_truncate_an_existing_file() {
        let path = temp("invalid-format.wav");
        std::fs::write(&path, b"existing recording").unwrap();
        for (rate, channels) in [(48_000.0, u16::MAX), (u32::MAX as f32, 2)] {
            assert!(WavWriter::create(&path, rate, channels).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"existing recording");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_failed_partial_wav_append_is_removed_when_finalizing() {
        use std::io::Write;
        let path = temp("partial-write.wav");
        let mut writer = WavWriter::create(&path, 48_000.0, 2).unwrap();
        writer.write(&[0.5, -0.5]).unwrap();
        let result = writer.append(&[1.0; 4], |file, bytes| {
            file.write_all(&bytes[..11])?;
            Err(std::io::Error::other("disk stopped during a frame"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 8 + 11);
        assert!(writer.write(&[1.0; 2]).is_err());
        writer.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        std::fs::remove_file(path).unwrap();
    }
}
