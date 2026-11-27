//! A minimal WAV reader: enough to get the audio a take recorded into the
//! spectrum analyzer, and no more. It reads the take's own WAV in the layout
//! `harmonigraph_take::WavWriter` writes and reads back, and refuses any
//! other.
//!
//! The spectrum can't come from the plugin. Its live audio ring drops
//! samples under backpressure by design (the right failure mode for a
//! meter, the wrong one for a recording), and during an offline export
//! there is no GUI draining it at all. So the analyzer is fed from the
//! WAV the recorder wrote beside the take instead — which is also the file
//! the video's audio comes from, so the curve and the sound can't drift
//! apart.
//!
//! Hand-rolled rather than pulled from a crate: it is one fixed header and
//! one sample decoder, against a workspace that documents every dependency
//! it takes on.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::ops::Range;

use harmonigraph_take::WavWriter;

// Bound storage by samples/bytes, not by a requested time range or FFT hop.
// One unusually wide channel frame may exceed this target, but duration cannot.
const BUFFER_BYTES: usize = 64 * 1024;
const DECODE_SAMPLES: usize = 16 * 1024;
/// A sample's bytes: the take's WAV is 32-bit float.
const WIDTH: usize = 4;
/// Where the samples start, right after the writer's header.
const DATA_START: u64 = WavWriter::HEADER_BYTES as u64;

/// Seekable uncompressed WAV input, with channels intact. Each consumer scans
/// absolute source-frame ranges through the same reusable decoding buffers.
/// The file must remain unchanged while open; I/O failures abort the export.
pub struct Audio {
    pub sample_rate: f32,
    pub channels: usize,
    /// The writer never finished the header, as when the plugin or the host
    /// quit mid-take or the writer failed to: the export says so.
    pub unfinished: bool,
    reader: BufReader<File>,
    frames: usize,
    position: usize,
    encoded: Vec<u8>,
    decoded: Vec<f32>,
}

impl Audio {
    #[cfg(test)]
    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn seconds(&self) -> f64 {
        if self.sample_rate <= 0.0 {
            return 0.0;
        }
        self.frames as f64 / f64::from(self.sample_rate)
    }

    /// Round and clamp on FRAME boundaries, including backwards/empty ranges.
    pub fn range_seconds(&self, from: f64, to: f64) -> Range<usize> {
        let frame = |t: f64| {
            (t * f64::from(self.sample_rate)).round().clamp(0.0, self.frames as f64) as usize
        };
        let start = frame(from);
        start..frame(to).max(start)
    }

    /// Feed every complete interleaved frame in a clamped absolute range.
    /// Physical chunk boundaries do not define analyzer or envelope boundaries.
    /// Sequential requests reuse the buffered cursor; only discontinuities seek.
    pub fn for_frames(
        &mut self,
        range: Range<usize>,
        mut consume: impl FnMut(&[f32]),
    ) -> Result<(), String> {
        let start = range.start.min(self.frames);
        let end = range.end.min(self.frames).max(start);
        if start == end {
            return Ok(());
        }
        if self.position != start {
            self.reader
                .seek(SeekFrom::Start(DATA_START + (start * self.channels * WIDTH) as u64))
                .map_err(|e| format!("WAV seek at frame {start}: {e}"))?;
            self.position = start;
        }
        let chunk_frames = (DECODE_SAMPLES / self.channels).max(1);
        while self.position < end {
            let count = chunk_frames.min(end - self.position);
            self.encoded.resize(count * self.channels * WIDTH, 0);
            self.reader
                .read_exact(&mut self.encoded)
                .map_err(|e| format!("WAV read at frame {}: {e}", self.position))?;
            self.decoded.clear();
            self.decoded
                .extend(self.encoded.as_chunks::<WIDTH>().0.iter().map(|s| f32::from_le_bytes(*s)));
            self.position += count;
            consume(&self.decoded);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn from_samples(sample_rate: f32, samples: Vec<f32>, channels: usize) -> Self {
        Self::from_bytes(&float32_wav(sample_rate as u32, &samples, channels as u16)).unwrap()
    }

    #[cfg(test)]
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "harmonigraph-wav-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        let result = read(&path);
        // The open file owns the fixture from here on (also when a test panics).
        std::fs::remove_file(path).unwrap();
        result
    }

    #[cfg(test)]
    pub(crate) fn all_samples(&mut self) -> Vec<f32> {
        let mut samples = Vec::new();
        self.for_frames(0..self.frames, |chunk| samples.extend_from_slice(chunk)).unwrap();
        samples
    }
}

/// A take's WAV in memory: interleaved 32-bit float `samples`, sizes stated.
#[cfg(test)]
fn float32_wav(rate: u32, samples: &[f32], channels: u16) -> Vec<u8> {
    let data = samples.len() as u32 * 4;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * u32::from(channels) * 4).to_le_bytes());
    bytes.extend((channels * 4).to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data.to_le_bytes());
    for sample in samples {
        bytes.extend(sample.to_le_bytes());
    }
    bytes
}

/// The take's WAV at `path`: every whole frame after its header, to the end
/// of the file, so a take whose sizes were never patched reads whole too.
pub fn read(path: impl AsRef<std::path::Path>) -> Result<Audio, String> {
    let path = path.as_ref();
    let open = || -> Result<Audio, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mut reader = BufReader::with_capacity(BUFFER_BYTES, file);
        let mut bytes = [0u8; WavWriter::HEADER_BYTES as usize];
        reader.read_exact(&mut bytes).map_err(|_| "not a take's WAV: shorter than its header")?;
        let header = WavWriter::read_header(&bytes, len)?;
        let channels = usize::from(header.channels);
        Ok(Audio {
            sample_rate: header.rate as f32,
            channels,
            unfinished: !header.finished,
            reader,
            frames: header.frames as usize,
            position: 0,
            encoded: Vec::with_capacity(DECODE_SAMPLES.max(channels) * WIDTH),
            decoded: Vec::with_capacity(DECODE_SAMPLES.max(channels)),
        })
    };
    open().map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice_seconds(audio: &mut Audio, from: f64, to: f64) -> (Vec<f32>, usize) {
        let range = audio.range_seconds(from, to);
        let end = range.end;
        let mut samples = Vec::new();
        audio.for_frames(range, |chunk| samples.extend_from_slice(chunk)).unwrap();
        (samples, end)
    }

    /// Channels come through INTACT — the analyzer combines them in the power
    /// domain and cannot do that with an average it never saw
    /// (`ChannelBank` in `harmonigraph-analysis`). Alignment also
    /// measures their energy before combining channels.
    #[test]
    fn float32_stereo_keeps_its_channels() {
        let frames = [[1.0, 0.0], [-1.0, 1.0], [0.5, 0.5]];
        let mut audio = Audio::from_samples(48_000.0, frames.concat(), 2);
        assert_eq!(audio.sample_rate, 48_000.0);
        assert_eq!(audio.channels, 2);
        assert_eq!(
            audio.all_samples(),
            vec![1.0, 0.0, -1.0, 1.0, 0.5, 0.5],
            "interleaved, as decoded"
        );
        assert_eq!(audio.frames(), 3, "frames, not samples");
    }

    /// A slice has to start on channel 0 however its bounds are clamped: one
    /// sample out and every channel reads the next one's data for the rest of
    /// the slice — silent, and wrong in a way that looks like a phase problem.
    #[test]
    fn a_stereo_slice_is_cut_on_frame_boundaries() {
        // L = 1, 2, 3, 4; R = -1, -2, -3, -4, at 4 Hz so a frame is 0.25 s.
        let frames: Vec<Vec<f32>> = (1..=4).map(|i| vec![i as f32, -(i as f32)]).collect();
        let mut audio = Audio::from_samples(4.0, frames.iter().flatten().copied().collect(), 2);
        assert_eq!(audio.seconds(), 1.0);
        assert_eq!(slice_seconds(&mut audio, 0.25, 0.75).0, vec![2.0, -2.0, 3.0, -3.0]);
        // Every slice starts on a left sample, whatever the bounds do.
        for (from, to, expected_end) in
            [(0.0, 1.0, 4), (0.1, 0.6, 2), (-1.0, 0.3, 1), (0.4, 9.0, 4), (0.9, 0.1, 4)]
        {
            let (slice, end) = slice_seconds(&mut audio, from, to);
            assert_eq!(end, expected_end, "the endpoint belongs to the clamped slice");
            assert_eq!(slice.len() % 2, 0, "[{from}, {to}) cut a frame in half");
            assert!(
                slice.as_chunks::<2>().0.iter().all(|f| f[0] > 0.0 && f[1] < 0.0),
                "[{from}, {to}) swapped the channels: {slice:?}",
            );
        }
    }

    #[test]
    fn seconds_and_slicing_line_up_with_the_sample_rate() {
        // A second and a half, so a length that is not one second is read.
        let frames: Vec<Vec<f32>> = (0..150).map(|i| vec![i as f32 / 150.0]).collect();
        let mut audio = Audio::from_samples(100.0, frames.concat(), 1);
        assert_eq!(audio.seconds(), 1.5);
        assert_eq!(slice_seconds(&mut audio, 0.1, 0.2).0.len(), 10);
        // Past the end is empty, not a panic: the visual tail outlives
        // the audio whenever a note fades out at the end.
        assert!(slice_seconds(&mut audio, 5.0, 6.0).0.is_empty());
        // A backwards range is empty too, rather than panicking on the
        // reversed slice bounds.
        assert!(slice_seconds(&mut audio, 0.5, 0.2).0.is_empty());
    }

    #[test]
    fn a_non_wav_file_is_rejected_clearly() {
        assert!(Audio::from_bytes(b"not a wav at all").is_err());
    }

    /// The reader decodes the take's own 32-bit float and nothing else; PCM,
    /// say, is refused rather than misread as float.
    #[test]
    fn a_wav_in_another_encoding_is_refused() {
        let mut bytes = float32_wav(48_000, &[0.0], 1);
        bytes[20] = 1; // the format tag, first field of the fmt chunk body: PCM
        let Err(err) = Audio::from_bytes(&bytes) else {
            panic!("a PCM WAV should be refused, not misread");
        };
        assert!(err.contains("not a take's WAV"), "{err}");
    }

    #[test]
    fn large_ranges_seeks_and_final_chunks_keep_frames_and_bound_storage() {
        let channels = 3;
        let frames = 70_003;
        let samples: Vec<f32> = (0..frames * channels).map(|i| i as f32 / 1000.0).collect();
        let mut audio = Audio::from_samples(192_000.0, samples.clone(), channels);
        let capacity = (audio.encoded.capacity(), audio.decoded.capacity());
        for range in [
            0..1,
            1..5462,
            5462..30_123,
            40_017..70_003,
            23..49_917,
            0..usize::MAX,
            frames..frames + 10,
        ] {
            let mut actual = Vec::new();
            let mut chunks = 0;
            audio
                .for_frames(range.clone(), |chunk| {
                    assert_eq!(chunk.len() % channels, 0);
                    assert!(chunk.len() <= DECODE_SAMPLES);
                    actual.extend_from_slice(chunk);
                    chunks += 1;
                })
                .unwrap();
            assert_eq!(
                actual,
                samples[range.start.min(frames) * channels..range.end.min(frames) * channels]
            );
            if range.end == usize::MAX {
                assert!(chunks > 10, "must reach repeated internal segmentation");
            }
            assert_eq!(capacity, (audio.encoded.capacity(), audio.decoded.capacity()));
        }
    }

    /// A take's WAV whose process died before `finish`: its header still says
    /// it holds no data. It reads to the end of the file in whole frames at
    /// the rate its first write stated, and says it was unfinished; so does
    /// one that died before its first sample, while a finished empty one
    /// stays quiet.
    #[test]
    fn a_wav_whose_writer_never_finished_reads_to_the_end_of_the_file() {
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-wav-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let crashed = dir.join("crashed.wav");
        let mut writer = WavWriter::create(&crashed, 2).unwrap();
        // Not the rate the header states before its first write.
        writer.write(44_100.0, &[0.5, -0.5, 0.25, -0.25]).unwrap();
        // As a crash leaves it: neither `finish` nor the drop patches the sizes.
        std::mem::forget(writer);
        // And a write cut off mid-frame.
        let mut file = std::fs::OpenOptions::new().append(true).open(&crashed).unwrap();
        std::io::Write::write_all(&mut file, &0.125f32.to_le_bytes()).unwrap();
        let mut audio = read(&crashed).unwrap();
        assert!(audio.unfinished);
        assert_eq!(audio.sample_rate, 44_100.0);
        assert_eq!(audio.all_samples(), [0.5, -0.5, 0.25, -0.25], "whole frames only");

        let silent = dir.join("silent.wav");
        std::mem::forget(WavWriter::create(&silent, 2).unwrap());
        let audio = read(&silent).unwrap();
        assert!(audio.unfinished, "a take that crashed before its first sample");
        assert_eq!(audio.frames(), 0);

        let empty = dir.join("empty.wav");
        WavWriter::create(&empty, 2).unwrap().finish().unwrap();
        let audio = read(&empty).unwrap();
        assert!(!audio.unfinished, "a finished take with no audio is whole");
        assert_eq!(audio.frames(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn io_failure_is_returned_instead_of_silently_ending_a_range() {
        let path = std::env::temp_dir()
            .join(format!("harmonigraph-wav-truncate-{}.wav", std::process::id()));
        let bytes = float32_wav(48_000, &[0.5; 50_000], 1);
        std::fs::write(&path, bytes).unwrap();
        let mut audio = read(&path).unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(44).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(audio.for_frames(40_000..50_000, |_| {}).unwrap_err().contains("WAV read"));
    }
}
