//! A minimal WAV reader: enough to get the DAW's bounce into the
//! spectrum analyzer, and no more.
//!
//! The spectrum can't come from the plugin. Its live audio ring drops
//! samples under backpressure by design (the right failure mode for a
//! meter, the wrong one for a recording), and during an offline export
//! there is no GUI draining it at all. So the analyzer is fed from the
//! bounced file instead — which is also the file the video's audio comes
//! from, so the curve and the sound can't drift apart.
//!
//! Hand-rolled rather than pulled from a crate: it is one chunk walk and
//! five sample decoders, against a workspace that documents every
//! dependency it takes on.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::ops::Range;

// Bound storage by samples/bytes, not by a requested time range or FFT hop.
// One unusually wide channel frame may exceed this target, but duration cannot.
const BUFFER_BYTES: usize = 64 * 1024;
const DECODE_SAMPLES: usize = 16 * 1024;

/// Seekable uncompressed WAV input, with channels intact. Each consumer scans
/// absolute source-frame ranges through the same reusable decoding buffers.
/// The file must remain unchanged while open; I/O failures abort the export.
pub struct Audio {
    pub sample_rate: f32,
    pub channels: usize,
    reader: BufReader<File>,
    data_start: u64,
    frames: usize,
    position: usize,
    width: usize,
    decode_one: fn(&[u8]) -> f32,
    encoded: Vec<u8>,
    decoded: Vec<f32>,
}

impl Audio {
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
                .seek(SeekFrom::Start(
                    self.data_start + (start * self.channels * self.width) as u64,
                ))
                .map_err(|e| format!("WAV seek at frame {start}: {e}"))?;
            self.position = start;
        }
        let chunk_frames = (DECODE_SAMPLES / self.channels).max(1);
        while self.position < end {
            let count = chunk_frames.min(end - self.position);
            self.encoded.resize(count * self.channels * self.width, 0);
            self.reader
                .read_exact(&mut self.encoded)
                .map_err(|e| format!("WAV read at frame {}: {e}", self.position))?;
            self.decoded.clear();
            self.decoded.extend(self.encoded.chunks_exact(self.width).map(self.decode_one));
            self.position += count;
            consume(&self.decoded);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn from_samples(sample_rate: f32, samples: Vec<f32>, channels: usize) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + samples.len() as u32 * 4).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(3u16.to_le_bytes());
        bytes.extend((channels as u16).to_le_bytes());
        bytes.extend((sample_rate as u32).to_le_bytes());
        bytes.extend((sample_rate as u32 * channels as u32 * 4).to_le_bytes());
        bytes.extend((channels as u16 * 4).to_le_bytes());
        bytes.extend(32u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples.len() as u32 * 4).to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        Self::from_bytes(&bytes).unwrap()
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

pub fn read(path: impl AsRef<std::path::Path>) -> Result<Audio, String> {
    let path = path.as_ref();
    let open = || -> Result<Audio, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mut reader = BufReader::with_capacity(BUFFER_BYTES, file);
        let mut header = [0u8; 12];
        reader.read_exact(&mut header).map_err(|_| "not a RIFF/WAVE file")?;
        if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
            return Err("not a RIFF/WAVE file".into());
        }
        let mut format = None;
        let mut data = None;
        let mut at = 12;
        let mut cursor = 12;
        while at + 8 <= len {
            reader.seek_relative(at as i64 - cursor as i64).map_err(|e| e.to_string())?;
            let mut chunk = [0; 8];
            reader.read_exact(&mut chunk).map_err(|e| e.to_string())?;
            let size = u64::from(u32_at(&chunk, 4));
            let body_at = at + 8;
            cursor = body_at;
            let available = size.min(len - body_at);
            match &chunk[..4] {
                b"fmt " if available >= 16 => {
                    let mut body = [0u8; 26];
                    let count = available.min(body.len() as u64) as usize;
                    reader.read_exact(&mut body[..count]).map_err(|e| e.to_string())?;
                    cursor += count as u64;
                    let tag = u16_at(&body, 0);
                    let tag = if tag == 0xFFFE && count >= 26 { u16_at(&body, 24) } else { tag };
                    format = Some((tag, u16_at(&body, 2), u32_at(&body, 4), u16_at(&body, 14)));
                }
                b"data" => data = Some((body_at, available)),
                _ => {}
            }
            at = body_at + size + (size & 1);
        }
        let (tag, channels, rate, bits) = format.ok_or("no fmt chunk")?;
        let (data_start, data_bytes) = data.ok_or("no data chunk")?;
        if channels == 0 {
            return Err("fmt chunk claims zero channels".into());
        }
        let decode_one: fn(&[u8]) -> f32 = match (tag, bits) {
            (1, 8) => |s| (f32::from(s[0]) - 128.0) / 128.0,
            (1, 16) => |s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0,
            (1, 24) => |s| (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            _ => return Err(format!("unsupported WAV encoding (format tag {tag}, {bits}-bit); export as PCM or 32-bit float")),
        };
        let channels = usize::from(channels);
        let width = usize::from(bits / 8);
        // Declared data can exceed the physical file after an interrupted bounce.
        // Preserve the old reader's complete-frame prefix, never a partial channel.
        let frames = (data_bytes / (channels * width) as u64) as usize;
        reader.seek_relative(data_start as i64 - cursor as i64).map_err(|e| e.to_string())?;
        Ok(Audio {
            sample_rate: rate as f32,
            channels,
            reader,
            data_start,
            frames,
            position: 0,
            width,
            decode_one,
            encoded: Vec::with_capacity(DECODE_SAMPLES.max(channels) * width),
            decoded: Vec::with_capacity(DECODE_SAMPLES.max(channels)),
        })
    };
    open().map_err(|e| format!("{}: {e}", path.display()))
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
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

    /// Build a WAV in memory: `tag`/`bits` pick the encoding, `frames`
    /// are per-channel sample values in -1..1.
    fn build(tag: u16, bits: u16, channels: u16, rate: u32, frames: &[Vec<f32>]) -> Vec<u8> {
        let width = usize::from(bits / 8);
        let mut data = Vec::new();
        for frame in frames {
            for &v in frame {
                match (tag, bits) {
                    (1, 16) => data.extend(((v * 32_767.0) as i16).to_le_bytes()),
                    (1, 24) => {
                        let x = (v * 8_388_607.0) as i32;
                        data.extend(&x.to_le_bytes()[0..3]);
                    }
                    (3, 32) => data.extend(v.to_le_bytes()),
                    _ => unreachable!("test builder covers what the tests use"),
                }
            }
        }
        let mut fmt = Vec::new();
        fmt.extend(tag.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * u32::from(channels) * width as u32).to_le_bytes()); // byte rate
        fmt.extend((channels * bits / 8).to_le_bytes()); // block align
        fmt.extend(bits.to_le_bytes());

        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend((36u32 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVE");
        out.extend(b"fmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(&fmt);
        // An unknown chunk between fmt and data, which real DAW exports
        // are full of (LIST/INFO, bext, cue) and a walker must skip.
        out.extend(b"LIST");
        out.extend(4u32.to_le_bytes());
        out.extend(b"INFO");
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(&data);
        out
    }

    /// Channels come through INTACT — the analyzer combines them in the power
    /// domain and cannot do that with an average it never saw
    /// ([`ChannelBank`](harmonigraph_core::spectrum::ChannelBank)). Alignment also
    /// measures their energy before combining channels.
    #[test]
    fn float32_stereo_keeps_its_channels() {
        let frames = vec![vec![1.0, 0.0], vec![-1.0, 1.0], vec![0.5, 0.5]];
        let mut audio = Audio::from_bytes(&build(3, 32, 2, 48_000, &frames)).unwrap();
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
                slice.chunks_exact(2).all(|f| f[0] > 0.0 && f[1] < 0.0),
                "[{from}, {to}) swapped the channels: {slice:?}",
            );
        }
    }

    #[test]
    fn pcm16_and_pcm24_decode_to_the_same_signal() {
        let frames: Vec<Vec<f32>> =
            (0..16).map(|i| vec![(i as f32 / 8.0 - 1.0).clamp(-1.0, 1.0)]).collect();
        let mut a = Audio::from_bytes(&build(1, 16, 1, 44_100, &frames)).unwrap();
        let mut b = Audio::from_bytes(&build(1, 24, 1, 44_100, &frames)).unwrap();
        assert_eq!(a.all_samples().len(), b.all_samples().len());
        for (x, y) in a.all_samples().iter().zip(&b.all_samples()) {
            assert!((x - y).abs() < 1e-4, "{x} vs {y}");
        }
    }

    #[test]
    fn unknown_chunks_between_fmt_and_data_are_skipped() {
        // The builder always writes a LIST chunk; if the walk mishandled
        // it, `data` would be missed or misaligned.
        let mut audio =
            Audio::from_bytes(&build(3, 32, 1, 48_000, &[vec![0.25], vec![-0.25]])).unwrap();
        assert_eq!(audio.all_samples(), vec![0.25, -0.25]);
    }

    #[test]
    fn seconds_and_slicing_line_up_with_the_sample_rate() {
        let frames: Vec<Vec<f32>> = (0..100).map(|i| vec![i as f32 / 100.0]).collect();
        let mut audio = Audio::from_bytes(&build(3, 32, 1, 100, &frames)).unwrap();
        assert_eq!(audio.seconds(), 1.0);
        assert_eq!(slice_seconds(&mut audio, 0.1, 0.2).0.len(), 10);
        // Past the end is empty, not a panic: the visual tail outlives
        // the bounce whenever a note fades out at the end.
        assert!(slice_seconds(&mut audio, 5.0, 6.0).0.is_empty());
        // A backwards range is empty too, rather than panicking on the
        // reversed slice bounds.
        assert!(slice_seconds(&mut audio, 0.5, 0.2).0.is_empty());
    }

    #[test]
    fn a_non_wav_file_is_rejected_clearly() {
        assert!(Audio::from_bytes(b"not a wav at all").is_err());
    }

    #[test]
    fn a_compressed_wav_says_what_to_do_about_it() {
        // Format tag 2 = MS ADPCM.
        let mut bytes = build(1, 16, 1, 48_000, &[vec![0.0]]);
        bytes[20] = 2; // the format tag, first field of the fmt chunk body
        let Err(err) = Audio::from_bytes(&bytes) else {
            panic!("a compressed WAV should be rejected, not silently misread");
        };
        assert!(err.contains("unsupported"), "{err}");
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

    #[test]
    fn all_encodings_extensible_padding_and_truncated_channel_frames() {
        for (tag, bits, raw, expected) in [
            (1u16, 8u16, vec![0, 128, 192, 255], vec![-1.0, 0.0, 0.5, 127.0 / 128.0]),
            (
                1,
                16,
                [-32768i16, 0, 16384, 32767].into_iter().flat_map(i16::to_le_bytes).collect(),
                vec![-1.0, 0.0, 0.5, 32767.0 / 32768.0],
            ),
            (
                1,
                24,
                vec![0, 0, 128, 0, 0, 0, 0, 0, 64, 255, 255, 127],
                vec![-1.0, 0.0, 0.5, 8388607.0 / 8388608.0],
            ),
            (
                1,
                32,
                [i32::MIN, 0, 1073741824, i32::MAX]
                    .into_iter()
                    .flat_map(i32::to_le_bytes)
                    .collect(),
                vec![-1.0, 0.0, 0.5, 1.0],
            ),
            (
                3,
                32,
                [-1.0f32, 0.0, 0.5, 1.0].into_iter().flat_map(f32::to_le_bytes).collect(),
                vec![-1.0, 0.0, 0.5, 1.0],
            ),
        ] {
            for extensible in [false, true] {
                let mut bytes = b"RIFF\0\0\0\0WAVEJUNK\x01\0\0\0x\0fmt ".to_vec();
                let fmt_len = if extensible { 26u32 } else { 16 };
                bytes.extend(fmt_len.to_le_bytes());
                bytes.extend(if extensible { 0xfffeu16 } else { tag }.to_le_bytes());
                bytes.extend(2u16.to_le_bytes());
                bytes.extend(44100u32.to_le_bytes());
                bytes.extend([0; 6]);
                bytes.extend(bits.to_le_bytes());
                if extensible {
                    bytes.extend([0; 8]);
                    bytes.extend(tag.to_le_bytes());
                }
                bytes.extend(b"data");
                bytes.extend((raw.len() as u32 + 99).to_le_bytes());
                bytes.extend(&raw);
                // A trailing sample cannot become half a stereo frame.
                bytes.extend(&raw[..usize::from(bits / 8)]);
                let mut audio = Audio::from_bytes(&bytes).unwrap();
                assert_eq!(audio.frames(), 2);
                assert_eq!(audio.all_samples(), expected, "{tag}/{bits}, extensible={extensible}");
            }
        }
    }

    #[test]
    fn io_failure_is_returned_instead_of_silently_ending_a_range() {
        let path = std::env::temp_dir()
            .join(format!("harmonigraph-wav-truncate-{}.wav", std::process::id()));
        let bytes = build(3, 32, 1, 48_000, &vec![vec![0.5]; 50_000]);
        std::fs::write(&path, bytes).unwrap();
        let mut audio = read(&path).unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(44).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(audio.for_frames(40_000..50_000, |_| {}).unwrap_err().contains("WAV read"));
    }
}
