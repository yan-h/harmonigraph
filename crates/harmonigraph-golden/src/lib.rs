//! The gate a golden frame passes: byte-exact against a blessed PNG, or a
//! contact sheet and a demand that the PR say what moved.
//!
//! Only the MECHANISM lives here. What a frame is OF, and what a diff on it
//! means, belongs with the scenes — `harmonigraph-render`'s
//! `lattice_tests::golden` and `harmonigraph-offline`'s `golden` each carry
//! their own set and their own reading of one.
//!
//! There is one gate rather than one per crate because the bless protocol is a
//! CONTRACT with whoever is holding the branch: `HARMONIGRAPH_BLESS=1 cargo
//! test --workspace golden` re-baselines every set in one run, and the
//! amplified sheet it names is what a re-baseline is read from. A second
//! implementation of that would be free to drift — a tolerance here, a
//! different environment variable there — and the drift would show up as a
//! session blessing one crate's frames and silently not the other's.

use std::path::{Path, PathBuf};

/// How far a channel may drift before the frame counts as changed.
///
/// Zero: one machine, one driver, one backend, so a difference is the shader's
/// or the scene's and not the platform's. A tolerance would have to be wider
/// than #453's mean of 3.3/255 to be worth having, and that is the signal
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

/// One crate's set of blessed frames.
pub struct Gate {
    /// Where the blessed PNGs live — `<crate>/golden`, committed.
    dir: PathBuf,
    /// Where a failing comparison writes its contact sheet — under `target/`,
    /// so the working tree stays clean while the gate is passing.
    diffs: PathBuf,
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
        Gate { dir: manifest.join("golden"), diffs }
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
        let levels: std::collections::BTreeSet<[u8; 4]> =
            frame.chunks_exact(4).map(|px| [px[0], px[1], px[2], px[3]]).collect();
        assert!(
            levels.len() >= LEVELS,
            "{name} drew {} distinct pixel values — the fixture reaches nothing",
            levels.len(),
        );
        let path = self.dir.join(format!("{name}.png"));

        if std::env::var_os(BLESS).is_some() {
            let before = read_png(&path).map(|(px, _)| drift(&px, frame));
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

        let Some((expected, on_record)) = read_png(&path) else {
            panic!(
                "no golden frame at {}\nrun: {BLESS}=1 cargo test --workspace golden",
                path.display(),
            );
        };
        assert_eq!(on_record, size, "{name}: golden was written at a different size");

        let (mean, max) = drift(&expected, frame);
        if max > TOLERANCE {
            let sheet = self.write_contact_sheet(name, size, &expected, frame);
            panic!(
                "{name} moved: mean {mean:.3}/255, max {max}/255\n\
                 expected | actual | difference at 8x: {}\n\
                 If the change is intended, re-baseline and say what moved in the PR body:\n\
                 {BLESS}=1 cargo test --workspace golden",
                sheet.display()
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
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("png header").write_image_data(rgba).expect("png data");
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
