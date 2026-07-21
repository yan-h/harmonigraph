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
    /// File name (not path) of the audio recorded with this take, if
    /// any. A sibling of the take file, so the pair can be moved
    /// together. The renderer uses it when no audio is given explicitly.
    #[serde(default)]
    pub audio_file: Option<String>,
    /// Take time corresponding to the audio's first sample.
    ///
    /// Recording usually starts at song position 0, making this 0 — but
    /// arming mid-song does not, and without it the spectrum and the
    /// muxed track would both sit at the wrong place by exactly however
    /// far in you started.
    #[serde(default)]
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
        // Sort by time. Records are *written* in the order the audio
        // thread saw them, which is time order only while the transport
        // runs forward — a loop rollover or a jump makes the file
        // non-monotonic, and a replay that walks it in file order would
        // then stall on the first out-of-order record and deliver
        // everything after it late. Stable, so simultaneous events keep
        // the order they were played in (note-off before the note-on that
        // replaces it, and so on).
        take.notes.sort_by(|a, b| a.t.total_cmp(&b.t));
        take.params.sort_by(|a, b| a.t.total_cmp(&b.t));
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
            let note = NoteRecord { t, channel: 0, note: 60, kind: NoteKind::Off };
            text.push_str(&ron::to_string(&Record::Note(note)).unwrap());
            text.push('\n');
            let param = ParamRecord { t, id: "pitch-class-fade".into(), value: t as f32 };
            text.push_str(&ron::to_string(&Record::Param(param)).unwrap());
            text.push('\n');
        }
        let take = Take::parse(std::io::Cursor::new(text.as_bytes())).unwrap();
        let note_times: Vec<f64> = take.notes.iter().map(|n| n.t).collect();
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
            let note = NoteRecord { t: 2.0, channel: 0, note: 60, kind };
            text.push_str(&ron::to_string(&Record::Note(note)).unwrap());
            text.push('\n');
        }
        let take = Take::parse(std::io::Cursor::new(text.as_bytes())).unwrap();
        assert_eq!(take.notes[0].kind, NoteKind::Off);
        assert!(matches!(take.notes[1].kind, NoteKind::On { .. }));
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
/// tolerates a short RIFF size — and `lattice-offline`'s reader
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
