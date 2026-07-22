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
//! four sample decoders, against a workspace that documents every
//! dependency it takes on.

/// A decoded mono mixdown.
pub struct Audio {
    pub sample_rate: f32,
    /// Mono samples, channels averaged — what the analyzer wants.
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn seconds(&self) -> f64 {
        if self.sample_rate <= 0.0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }

    /// The mono samples covering `[from, to)` seconds, clamped to what
    /// exists. Past the end this is empty, which reads to the analyzer as
    /// "the source stopped" — exactly right for a bounce that ends before
    /// the visual tail does.
    pub fn slice_seconds(&self, from: f64, to: f64) -> &[f32] {
        let index = |t: f64| {
            (t * f64::from(self.sample_rate)).round().clamp(0.0, self.samples.len() as f64)
                as usize
        };
        let (start, end) = (index(from), index(to));
        &self.samples[start..end.max(start)]
    }
}

pub fn read(path: impl AsRef<std::path::Path>) -> Result<Audio, String> {
    let bytes = std::fs::read(path.as_ref())
        .map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    decode(&bytes).map_err(|e| format!("{}: {e}", path.as_ref().display()))
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub fn decode(bytes: &[u8]) -> Result<Audio, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }

    let mut format: Option<(u16, u16, u32, u16)> = None; // tag, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4) as usize;
        let body_at = at + 8;
        // A declared size past the end means a truncated file; take what
        // is there rather than refusing, so a killed export still renders.
        let body = &bytes[body_at..(body_at + size).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let tag = u16_at(body, 0);
                // WAVE_FORMAT_EXTENSIBLE stores the real tag in its GUID's
                // first two bytes; DAWs write it for anything above stereo.
                let tag = if tag == 0xFFFE && body.len() >= 26 { u16_at(body, 24) } else { tag };
                format = Some((tag, u16_at(body, 2), u32_at(body, 4), u16_at(body, 14)));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are word-aligned; an odd size carries a pad byte.
        at = body_at + size + (size & 1);
    }

    let (tag, channels, rate, bits) = format.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 {
        return Err("fmt chunk claims zero channels".into());
    }

    // 1 = PCM integer, 3 = IEEE float. Anything else is compressed.
    let decode_one: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 16) => |s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0,
        (1, 24) => |s| {
            // Sign-extend 24-bit little-endian into an i32.
            let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
            v as f32 / 8_388_608.0
        },
        (1, 32) => |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        (1, 8) => |s| (f32::from(s[0]) - 128.0) / 128.0,
        _ => {
            return Err(format!(
                "unsupported WAV encoding (format tag {tag}, {bits}-bit); \
                 export as PCM or 32-bit float"
            ))
        }
    };

    let width = usize::from(bits / 8).max(1);
    let frame = width * usize::from(channels);
    let frames = data.len() / frame;
    let gain = 1.0 / f32::from(channels);
    let mut samples = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * frame;
        let mut sum = 0.0;
        for c in 0..usize::from(channels) {
            let at = base + c * width;
            sum += decode_one(&data[at..at + width]);
        }
        samples.push(sum * gain);
    }

    Ok(Audio { sample_rate: rate as f32, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn float32_stereo_decodes_to_a_mono_average() {
        let frames = vec![vec![1.0, 0.0], vec![-1.0, 1.0], vec![0.5, 0.5]];
        let audio = decode(&build(3, 32, 2, 48_000, &frames)).unwrap();
        assert_eq!(audio.sample_rate, 48_000.0);
        assert_eq!(audio.samples, vec![0.5, 0.0, 0.5]);
    }

    #[test]
    fn pcm16_and_pcm24_decode_to_the_same_signal() {
        let frames: Vec<Vec<f32>> = (0..16)
            .map(|i| vec![(i as f32 / 8.0 - 1.0).clamp(-1.0, 1.0)])
            .collect();
        let a = decode(&build(1, 16, 1, 44_100, &frames)).unwrap();
        let b = decode(&build(1, 24, 1, 44_100, &frames)).unwrap();
        assert_eq!(a.samples.len(), b.samples.len());
        for (x, y) in a.samples.iter().zip(&b.samples) {
            assert!((x - y).abs() < 1e-4, "{x} vs {y}");
        }
    }

    #[test]
    fn unknown_chunks_between_fmt_and_data_are_skipped() {
        // The builder always writes a LIST chunk; if the walk mishandled
        // it, `data` would be missed or misaligned.
        let audio = decode(&build(3, 32, 1, 48_000, &[vec![0.25], vec![-0.25]])).unwrap();
        assert_eq!(audio.samples, vec![0.25, -0.25]);
    }

    #[test]
    fn seconds_and_slicing_line_up_with_the_sample_rate() {
        let frames: Vec<Vec<f32>> = (0..100).map(|i| vec![i as f32 / 100.0]).collect();
        let audio = decode(&build(3, 32, 1, 100, &frames)).unwrap();
        assert_eq!(audio.seconds(), 1.0);
        assert_eq!(audio.slice_seconds(0.1, 0.2).len(), 10);
        // Past the end is empty, not a panic: the visual tail outlives
        // the bounce whenever a note fades out at the end.
        assert!(audio.slice_seconds(5.0, 6.0).is_empty());
        // A backwards range is empty too, rather than panicking on the
        // reversed slice bounds.
        assert!(audio.slice_seconds(0.5, 0.2).is_empty());
    }

    #[test]
    fn a_non_wav_file_is_rejected_clearly() {
        assert!(decode(b"not a wav at all").is_err());
    }

    #[test]
    fn a_compressed_wav_says_what_to_do_about_it() {
        // Format tag 2 = MS ADPCM.
        let mut bytes = build(1, 16, 1, 48_000, &[vec![0.0]]);
        bytes[20] = 2; // the format tag, first field of the fmt chunk body
        let Err(err) = decode(&bytes) else {
            panic!("a compressed WAV should be rejected, not silently misread");
        };
        assert!(err.contains("unsupported"), "{err}");
    }
}
