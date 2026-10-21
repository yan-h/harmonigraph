//! The gate a golden frame passes: byte-exact against a blessed PNG, or a
//! contact sheet and a demand that the PR say what moved.
//!
//! Only the MECHANISM lives here. What a frame is OF, and what a diff on it
//! means, belongs with the scenes — `harmonigraph-render`'s
//! `lattice_tests::golden` and `harmonigraph-offline`'s `golden` each carry
//! their own set and their own reading of one.
//!
//! The frames on record are the ones the CI runner draws. A Mac on another
//! macOS draws them apart: on 2026-10-18 the owner's drew every frame of both
//! sets 1-3/255 off a `main` CI passed. So a failed comparison writes the frame
//! it drew under `target/golden-diff/actual/`, laid out as `crates/` is, CI
//! uploads that directory beside the amplified sheets when a group fails, and
//! a picture change is re-baselined from the run's `golden-diff-<group>`
//! artifacts: read the sheets, copy `actual/*` over `crates/`, and push. One
//! run writes every frame that moved, because each golden test holds one frame
//! (a second [`Gate::check`] in the same test would go unreached once the first
//! fails) and `ci.sh` runs every test binary rather than stopping at the first
//! that fails.
//! `HARMONIGRAPH_BLESS=1 cargo test --workspace golden` re-baselines to the
//! machine it runs on, which is the record only where that machine draws what
//! CI does.
//!
//! There is one gate rather than one per crate because that protocol is a
//! CONTRACT with whoever is holding the branch. A second implementation of it
//! would be free to drift — a tolerance here, a different environment variable
//! there — and the drift would show up as a session re-baselining one crate's
//! frames and silently not the other's.
//!
//! The one thing besides the mechanism that both sets share is [`LOOK`], the
//! settings their frames are drawn at, for the same reason: two copies of it
//! would drift.

use std::path::{Path, PathBuf};

/// The look every golden frame starts from: an `AppearanceDocument` in RON,
/// pinned rather than read off the fresh defaults.
///
/// A golden is worth having for the PR that must not move the picture — a
/// refactor, a frame saving, a cache — and is a formality on the PR that means
/// to. Drawn at the defaults, a set is the second kind on every capture of a
/// DAW look: #1475 re-blessed eighteen frames that way, and nobody learned
/// anything from them. Pinned, a capture moves nothing here, and whether the
/// fresh look is right is judged where it is seen, in the DAW.
///
/// Each crate parses what it draws from: the offline set the whole document,
/// the lattice set only its `view`. Missing keys take the field's `Default`,
/// as in any saved blob (the `persistence-contract` skill), so how it ages is
/// worth knowing before a frame moves:
///
/// - A NEW field enters at its `Default`. A feature that ships on moves the
///   frames that draw it, which is a picture change and stated as one.
/// - A RENAMED or retyped field drops its pinned value for the new `Default`.
///   If that moves a frame, carry the value over to the new key here rather
///   than blessing.
/// - An appearance version bump fails the offline parse; edit the number.
///
/// Re-pinning the whole look, say to a newer DAW capture through
/// `./read-plugin-state.py --appearance`, re-baselines every frame. That goes
/// in a PR of its own.
pub const LOOK: &str = include_str!("../look.ron");

/// How far a channel may drift before the frame counts as changed.
///
/// Zero: the record is one machine, CI's runner, so there a difference is the
/// shader's or the scene's and not the platform's. Another machine's 1-3/255
/// is not a reason for one: a tolerance wide enough for it would pass a uniform
/// two-level shift, which is #453's shape (a mean of 3.3/255) and the signal
/// rather than the noise.
const TOLERANCE: u8 = 0;

/// How many distinct pixel values a frame has to carry to count as a picture.
///
/// The vacuity guard every set shares. A golden cannot pass for the wrong
/// reason the way a claim test can — it asserts every pixel — but it CAN be a
/// frame with nothing in it: a camera looking past the lattice, a name
/// projected off the pane, a window with no audio inside it. Such a frame is
/// blessed once and then agrees with itself forever, which is #450's rule in
/// the shape a golden takes it in. Twenty is far below what any of these
/// scenes draws and far above a flat fill plus its edge.
const LEVELS: usize = 20;

/// The environment variable that turns a comparison into a re-baseline.
const BLESS: &str = "HARMONIGRAPH_BLESS";

/// The environment variable that draws a frame and skips its comparison.
///
/// For `tools/shader-assets.py` alone, which runs the golden tests to make
/// every pipeline they build load from a candidate Metal corpus, and whose
/// verdict is about the corpus rather than the picture. Compared, a PR that both
/// moves a frame and owes a corpus could not regenerate the corpus until the
/// frame was re-baselined, a CI round later: four of the eight regenerations
/// dispatched on 2026-10-19..21 failed on exactly that, a moved frame and no
/// corpus fault. The vacuity guard still runs, and [`BLESS`] still wins.
/// `ci.sh` unsets it, so it cannot turn the gate itself into a pass.
const DRAW_ONLY: &str = "HARMONIGRAPH_GOLDEN_DRAW_ONLY";

/// One crate's set of blessed frames.
pub struct Gate {
    /// Where the blessed PNGs live — `<crate>/golden`, committed.
    dir: PathBuf,
    /// Where a failing comparison writes its contact sheet — under `target/`,
    /// so the working tree stays clean while the gate is passing.
    diffs: PathBuf,
    /// Where it writes the frame it actually drew, laid out as the repository
    /// is (`actual/<crate>/golden/<name>.png`), so a frame CI drew is a copy
    /// into `crates/` away from being the one on record.
    actual: PathBuf,
}

impl Gate {
    /// The gate for the crate whose `CARGO_MANIFEST_DIR` is `manifest`.
    ///
    /// Takes the caller's `env!` rather than reading its own: this crate's
    /// manifest directory is this crate, and every set of frames belongs to
    /// the crate that draws them.
    pub fn new(manifest: &str) -> Gate {
        let manifest = PathBuf::from(manifest);
        // Beside the workspace's one `target/`, not inside the crate: a sheet
        // is written on failure and read once, and `cargo clean` is the right
        // way to be rid of it.
        let diffs = manifest.join("../../target/golden-diff");
        let krate = manifest.file_name().expect("a crate directory").to_owned();
        let actual = diffs.join("actual").join(krate).join("golden");
        Gate { dir: manifest.join("golden"), diffs, actual }
    }

    /// `frame` is byte-identical to the PNG on record under `name`, or the
    /// test fails with a sheet to look at.
    ///
    /// Under [`BLESS`] it writes the frame instead and reports what that moved,
    /// so a re-baseline states its own diff rather than leaving it to be found
    /// in the PR.
    ///
    /// `frame` is tightly packed RGBA8, `size[0] * size[1] * 4` bytes.
    pub fn check(&self, name: &str, size: [u32; 2], frame: &[u8]) {
        let bytes = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .expect("RGBA8 dimensions exceed addressable memory");
        assert_eq!(frame.len(), bytes, "{name}: wrong RGBA8 byte length");
        let mut levels = std::collections::BTreeSet::new();
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
        let path = self.dir.join(format!("{name}.png"));

        if std::env::var_os(BLESS).is_some() {
            let before = read_png(&path)
                .filter(|(_, previous_size)| *previous_size == size)
                .map(|(px, _)| drift(&px, frame));
            write_png(&path, size, frame);
            match before {
                Some((mean, max)) if max > 0 => {
                    eprintln!(
                        "blessed {name}: mean {mean:.3}, max {max} — say what moved in the PR"
                    )
                }
                Some(_) => eprintln!("blessed {name}: unchanged"),
                None => eprintln!("blessed {name}: new frame"),
            }
            return;
        }
        if std::env::var_os(DRAW_ONLY).is_some() {
            return;
        }

        // A new frame and a resized one have nothing to compare against, and
        // are written out for the same reason a moved one is: the record
        // comes from what CI drew.
        let actual = self.actual.join(format!("{name}.png"));
        let Some((expected, on_record)) = read_png(&path) else {
            write_png(&actual, size, frame);
            panic!(
                "no golden frame at {}\nthe frame drawn: {}\nre-baseline it from CI's \
                 golden-diff-<group> artifact (see this crate's docs)",
                path.display(),
                actual.display(),
            );
        };
        if on_record != size {
            write_png(&actual, size, frame);
            panic!(
                "{name}: golden was written at a different size; the frame drawn: {}",
                actual.display()
            );
        }

        let (mean, max) = drift(&expected, frame);
        if max > TOLERANCE {
            let sheet = self.write_contact_sheet(name, size, &expected, frame);
            write_png(&actual, size, frame);
            panic!(
                "{name} moved: mean {mean:.3}/255, max {max}/255\n\
                 expected | actual | difference at 8x: {}\n\
                 the frame drawn: {}\n\
                 If the change is intended, say what moved in the PR body and re-baseline \
                 from the frames CI drew: download the failed run's golden-diff-<group> \
                 artifacts and copy actual/* over crates/ (see this crate's docs). \
                 {BLESS}=1 cargo test --workspace golden re-baselines to THIS machine, \
                 which is the record only if it draws what CI does.",
                sheet.display(),
                actual.display(),
            );
        }
    }

    /// Expected, actual, and the difference at 8x, side by side in one image.
    ///
    /// The amplification is the point: the drift this gate exists to catch is a
    /// handful of levels, which is invisible in a raw subtraction and obvious
    /// at 8x.
    fn write_contact_sheet(
        &self,
        name: &str,
        size: [u32; 2],
        expected: &[u8],
        actual: &[u8],
    ) -> PathBuf {
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
                            (u32::from(expected[src + c].abs_diff(actual[src + c])) * 8).min(255)
                                as u8
                        };
                        [amp(0), amp(1), amp(2), 255]
                    }),
                ] {
                    let dst = (y * w * 3 + panel * w + x) * 4;
                    sheet[dst..dst + 4].copy_from_slice(&px);
                }
            }
        }
        let path = self.diffs.join(format!("{name}.png"));
        write_png(&path, [size[0] * 3, size[1]], &sheet);
        path
    }
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
    // Written beside the target and renamed over it, so a reader in another
    // process sees the old frame or the new one, never a truncated file (#1361).
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

    #[test]
    fn golden_requires_a_complete_frame_even_when_the_prefix_matches() {
        let dir = std::env::temp_dir().join(format!("harmonigraph-golden-{}", std::process::id()));
        let gate = Gate { dir: dir.clone(), diffs: dir.join("diffs"), actual: dir.join("actual") };
        let frame: Vec<u8> = (0..64).flat_map(|x| [x, 0, 0, 255]).collect();
        write_png(&dir.join("complete.png"), [8, 8], &frame);
        gate.check("complete", [8, 8], &frame);
        for len in [128, 255, 257, 260] {
            let mut incomplete = frame.clone();
            incomplete.resize(len, 0);
            assert!(
                std::panic::catch_unwind(|| gate.check("complete", [8, 8], &incomplete)).is_err(),
                "accepted {len} bytes for a 256-byte frame",
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A frame that moved is left on disk exactly as drawn, alpha included,
    /// which is what re-baselining from CI's uploaded frames rests on.
    #[test]
    fn a_moved_frame_is_written_out_as_drawn() {
        let dir = std::env::temp_dir().join(format!("harmonigraph-moved-{}", std::process::id()));
        let gate = Gate { dir: dir.clone(), diffs: dir.join("diffs"), actual: dir.join("actual") };
        let on_record: Vec<u8> = (0..64).flat_map(|x| [x, 0, 0, 255]).collect();
        write_png(&dir.join("moved.png"), [8, 8], &on_record);
        let drawn: Vec<u8> = (0..64).flat_map(|x| [x, 1, 0, 128 + x]).collect();
        assert!(std::panic::catch_unwind(|| gate.check("moved", [8, 8], &drawn)).is_err());
        assert_eq!(read_png(&dir.join("actual/moved.png")), Some((drawn, [8, 8])));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
