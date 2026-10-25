//! Golden frames, compared build against build: what this branch draws held
//! byte-exactly against what its base draws. A reported move is always between
//! two draws on the same machine in the same run; in CI, an answer of no move
//! may instead come from the base's frames cached by main's own run on the same
//! macOS build (`tools/golden-diff.py --base-frames`).
//!
//! Only the MECHANISM lives here. What a frame is OF, and what a diff on it
//! means, belongs with the scenes — `harmonigraph-render`'s
//! `lattice_tests::golden` and `spectrogram/tests.rs` and
//! `harmonigraph-offline`'s `golden` each carry their own set and their own
//! reading of one.
//!
//! A golden test draws its frame and hands it to [`Gate::check`], which checks
//! it is a picture at all and, under [`WRITE`], writes it out. That is all a
//! test does: an ordinary `cargo test` only proves every scene still draws.
//! `tools/golden-diff.py` draws the set twice, once from the base (an archive
//! of `git merge-base origin/main HEAD`, or `HEAD^1` in CI) and once from the
//! working tree, and [`compare`] says which frames moved, by how much, with a
//! contact sheet of each.
//!
//! Until 2026-10 the record was a committed PNG per frame, drawn by the CI
//! runner, and a picture change cost a CI round to re-baseline: no other
//! machine draws the runner's bytes (the owner's Mac is 1-3/255 off), so
//! nothing could be checked or blessed locally. 132 commits re-baselined it in
//! its eight weeks, and the three unintended moves it is credited with (#1158,
//! #1459, #991) were each a few levels on a few pixels, blessed rather than
//! fixed. What it was for, and what this keeps, is the proof a refactor or a
//! frame saving moved no pixel — which needs a base drawn on the same machine,
//! not a record.
//!
//! There is one mechanism rather than one per crate so that the sets cannot
//! drift apart in what they compare or where they write.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The look every golden frame starts from: an `AppearanceDocument` in RON,
/// pinned rather than read off the fresh defaults.
///
/// A golden is worth having for the PR that must not move the picture — a
/// refactor, a frame saving, a cache — and is a formality on the PR that means
/// to. Drawn at the defaults, a set is the second kind on every capture of a
/// DAW look: #1475 moved eighteen frames that way, and nobody learned anything
/// from them. Pinned, a capture moves nothing here, and whether the fresh look
/// is right is judged where it is seen, in the DAW.
///
/// Each set parses what it draws from: the offline set the whole document,
/// the lattice set only its `view`, the spectrogram styles only the spectrum's
/// `atmosphere`. Missing keys take the field's `Default`,
/// as in any saved blob (the `persistence-contract` skill), so how it ages is
/// worth knowing before a frame moves:
///
/// - A NEW field enters at its `Default`. A feature that ships on moves the
///   frames that draw it, which is a picture change and stated as one.
/// - A RENAMED or retyped field drops its pinned value for the new `Default`.
///   If that moves a frame, carry the value over to the new key here rather
///   than stating it as a picture change.
/// - An appearance version bump fails the offline parse; edit the number.
///
/// Re-pinning the whole look, say to a newer DAW capture through
/// `./read-plugin-state.py --appearance`, moves every frame. That goes in a PR
/// of its own.
pub const LOOK: &str = include_str!("../look.ron");

/// How many distinct pixel values a frame has to carry to count as a picture.
///
/// The vacuity guard every set shares. A golden cannot pass for the wrong
/// reason the way a claim test can — it compares every pixel — but it CAN be a
/// frame with nothing in it: a camera looking past the lattice, a name
/// projected off the pane, a window with no audio inside it. Such a frame
/// agrees with its base forever, which is #450's rule in the shape a golden
/// takes it in. Twenty is far below what any of these scenes draws and far
/// above a flat fill plus its edge.
const LEVELS: usize = 20;

/// The environment variable naming the directory frames are written to, as
/// `<dir>/<crate>/<name>.png`. Unset, a golden test draws and checks and
/// writes nothing.
pub const WRITE: &str = "HARMONIGRAPH_GOLDEN_WRITE";

/// One crate's set of golden frames.
pub struct Gate {
    krate: String,
}

impl Gate {
    /// The gate for the crate whose `CARGO_MANIFEST_DIR` is `manifest`.
    ///
    /// Takes the caller's `env!` rather than reading its own: every set of
    /// frames belongs to the crate that draws them.
    pub fn new(manifest: &str) -> Gate {
        let krate = Path::new(manifest).file_name().expect("a crate directory");
        Gate { krate: krate.to_string_lossy().into_owned() }
    }

    /// `frame` is a complete picture, written under [`WRITE`] when it is set.
    ///
    /// `frame` is tightly packed RGBA8, `size[0] * size[1] * 4` bytes.
    pub fn check(&self, name: &str, size: [u32; 2], frame: &[u8]) {
        let out = std::env::var_os(WRITE).map(PathBuf::from);
        self.check_into(out.as_deref(), name, size, frame);
    }

    fn check_into(&self, out: Option<&Path>, name: &str, size: [u32; 2], frame: &[u8]) {
        let bytes = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .expect("RGBA8 dimensions exceed addressable memory");
        assert_eq!(frame.len(), bytes, "{name}: wrong RGBA8 byte length");
        let mut levels = BTreeSet::new();
        for px in frame.as_chunks::<4>().0 {
            levels.insert([px[0], px[1], px[2], px[3]]);
            if levels.len() == LEVELS {
                break;
            }
        }
        assert!(
            levels.len() >= LEVELS,
            "{name} drew {} distinct pixel values — the fixture reaches nothing",
            levels.len(),
        );
        if let Some(out) = out {
            write_png(&out.join(&self.krate).join(format!("{name}.png")), size, frame);
        }
    }
}

/// What one set of frames did against another.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// `(frame, mean, max)` for each frame whose bytes differ from the base's,
    /// with its contact sheet written under `sheets/`.
    pub moved: Vec<(String, f64, u8)>,
    /// Frames the head draws and the base does not.
    pub new: Vec<String>,
    /// Frames the base draws and the head does not.
    pub removed: Vec<String>,
    /// `(frame, base size, head size)` for each frame drawn at a new size.
    pub resized: Vec<(String, [u32; 2], [u32; 2])>,
    /// Frames the head drew differently on its second draw: nondeterminism,
    /// reported apart so it never reads as a picture change, and never compared
    /// with the base, so it is stated like one.
    pub unstable: Vec<String>,
}

/// Compare the frames under `head` against `base` (each `<crate>/<name>.png`),
/// writing a contact sheet of each moved frame under `sheets`.
///
/// Byte-exact: both sides are drawn on one machine, so any difference is the
/// branch's and not the platform's, and a tolerance wide enough for noise
/// would pass a uniform two-level shift, which is #453's shape (a mean of
/// 3.3/255) and the signal rather than the noise. `again`, when given, is the
/// head drawn a second time; a frame the two draw differently, or that only
/// one of them draws, is unstable, and is still compared with the base.
pub fn compare(base: &Path, head: &Path, again: Option<&Path>, sheets: &Path) -> Report {
    let (base_frames, head_frames) = (frames(base), frames(head));
    let mut report = Report::default();
    for name in base_frames.difference(&head_frames) {
        report.removed.push(name.clone());
    }
    if let Some(again) = again {
        for name in frames(again).difference(&head_frames) {
            report.unstable.push(name.clone());
        }
    }
    for name in &head_frames {
        let drawn = read_png(&head.join(name)).expect("a frame just listed");
        if again.is_some_and(|again| read_png(&again.join(name)).as_ref() != Some(&drawn)) {
            report.unstable.push(name.clone());
        }
        let Some(expected) = read_png(&base.join(name)) else {
            report.new.push(name.clone());
            continue;
        };
        if expected == drawn {
            continue;
        }
        let ((expected, before), (actual, size)) = (expected, drawn);
        if before != size {
            report.resized.push((name.clone(), before, size));
            continue;
        }
        let (mean, max) = drift(&expected, &actual);
        write_png(
            &sheets.join(name),
            [size[0] * 3, size[1]],
            &contact_sheet(size, &expected, &actual),
        );
        report.moved.push((name.clone(), mean, max));
    }
    report
}

/// Every `<crate>/<name>.png` under `dir`.
fn frames(dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for krate in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for frame in std::fs::read_dir(krate.path()).into_iter().flatten().flatten() {
            let path = frame.path();
            if path.extension().is_some_and(|e| e == "png") {
                let relative = path.strip_prefix(dir).expect("listed under dir");
                found.insert(relative.to_string_lossy().into_owned());
            }
        }
    }
    found
}

/// Expected, actual, and the difference at 8x, side by side in one image.
///
/// The amplification is the point: the drift this exists to catch is a
/// handful of levels, which is invisible in a raw subtraction and obvious at
/// 8x.
fn contact_sheet(size: [u32; 2], expected: &[u8], actual: &[u8]) -> Vec<u8> {
    let [w, h] = [size[0] as usize, size[1] as usize];
    let mut sheet = vec![0u8; w * 3 * h * 4];
    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * 4;
            for (panel, px) in [
                (0usize, [expected[src], expected[src + 1], expected[src + 2], 255]),
                (1, [actual[src], actual[src + 1], actual[src + 2], 255]),
                (2, {
                    let amp = |c: usize| {
                        (u32::from(expected[src + c].abs_diff(actual[src + c])) * 8).min(255) as u8
                    };
                    [amp(0), amp(1), amp(2), 255]
                }),
            ] {
                let dst = (y * w * 3 + panel * w + x) * 4;
                sheet[dst..dst + 4].copy_from_slice(&px);
            }
        }
    }
    sheet
}

/// Mean and largest per-channel drift between two RGBA8 frames.
///
/// Both are reported because they separate the two ways a picture moves: a mean
/// near zero with a large max is a few pixels relocating (an edge, a glyph),
/// and a small max spread over a nonzero mean is a level shifting everywhere —
/// the #453 shape, and the one an eye does not catch.
pub fn drift(expected: &[u8], actual: &[u8]) -> (f64, u8) {
    assert_eq!(expected.len(), actual.len(), "cannot compare incomplete frames");
    let mut sum = 0u64;
    let mut max = 0u8;
    for (e, a) in expected.iter().zip(actual) {
        let d = e.abs_diff(*a);
        sum += u64::from(d);
        max = max.max(d);
    }
    (sum as f64 / expected.len() as f64, max)
}

fn write_png(path: &Path, size: [u32; 2], rgba: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("golden directory");
    }
    // Written beside the target and renamed over it, so an interrupted run
    // leaves the old frame or the new one, never a truncated file to compare.
    let tmp = path.with_extension(format!("png.{}.tmp", std::process::id()));
    let file = std::fs::File::create(&tmp).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
    writer.finish().expect("png finish");
    std::fs::rename(&tmp, path).expect("rename png into place");
}

fn read_png(path: &Path) -> Option<(Vec<u8>, [u32; 2])> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader =
        png::Decoder::new(std::io::BufReader::new(file)).read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("png data");
    buf.truncate(info.buffer_size());
    Some((buf, [info.width, info.height]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("harmonigraph-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A frame of 64 distinct levels, `shift` added to its green channel.
    fn frame(shift: u8) -> Vec<u8> {
        (0..64).flat_map(|x| [x, shift, 0, 255]).collect()
    }

    #[test]
    fn a_frame_must_be_complete_and_a_picture() {
        let gate = Gate { krate: "k".into() };
        gate.check_into(None, "complete", [8, 8], &frame(0));
        for len in [128, 255, 257, 260] {
            let mut incomplete = frame(0);
            incomplete.resize(len, 0);
            assert!(
                std::panic::catch_unwind(|| gate.check_into(None, "c", [8, 8], &incomplete))
                    .is_err(),
                "accepted {len} bytes for a 256-byte frame",
            );
        }
        let flat = [7u8, 7, 7, 255].repeat(64);
        assert!(std::panic::catch_unwind(|| gate.check_into(None, "flat", [8, 8], &flat)).is_err());
    }

    /// One level on one channel is a move, a frame on one side only is new or
    /// removed, a new size is a resize, and a head that draws a frame
    /// differently twice, or only once, is unstable as well.
    #[test]
    fn compare_names_what_moved_appeared_vanished_and_wavered() {
        let dir = scratch("compare");
        let gate = Gate { krate: "k".into() };
        let [base, head, again] = ["base", "head", "again"].map(|side| dir.join(side));
        for (name, b, h, a) in [
            ("still", Some(0), Some(0), Some(0)),
            ("moved", Some(0), Some(1), Some(1)),
            ("gone", Some(0), None, None),
            ("added", None, Some(0), Some(0)),
            ("wavers", Some(0), Some(1), Some(2)),
            ("flickers", None, None, Some(0)),
        ] {
            for (side, shift) in [(&base, b), (&head, h), (&again, a)] {
                if let Some(shift) = shift {
                    gate.check_into(Some(side), name, [8, 8], &frame(shift));
                }
            }
        }
        gate.check_into(Some(&base), "reshaped", [8, 8], &frame(0));
        for side in [&head, &again] {
            gate.check_into(Some(side), "reshaped", [4, 16], &frame(0));
        }
        let sheets = dir.join("sheets");
        let report = compare(&base, &head, Some(&again), &sheets);
        let mean = 64.0 / 256.0;
        assert_eq!(
            report,
            Report {
                moved: vec![("k/moved.png".into(), mean, 1), ("k/wavers.png".into(), mean, 1)],
                new: vec!["k/added.png".into()],
                removed: vec!["k/gone.png".into()],
                resized: vec![("k/reshaped.png".into(), [8, 8], [4, 16])],
                unstable: vec!["k/flickers.png".into(), "k/wavers.png".into()],
            }
        );
        assert!(sheets.join("k/moved.png").is_file(), "no contact sheet for the moved frame");
        assert_eq!(compare(&base, &base, None, &sheets), Report::default());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
