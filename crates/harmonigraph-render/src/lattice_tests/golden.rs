//! Byte-exact frames for the parts of the picture a feature PR is not
//! supposed to reach.
//!
//! Every other test in this suite is a CLAIM test: it names a property and
//! measures it, so it passes for any picture holding that property. That is
//! the right shape for a dial whose look is still moving, and it is why five
//! shader terms survive the whole suite (#450) and why #453 could move the
//! frame by a mean of 3.3/255 with local swings of −90 while 146 tests stayed
//! green. Nothing here compares a build against another build, so
//! "behaviour-preserving" is otherwise an unverifiable claim.
//!
//! These two do compare, and their subject is chosen so that a re-baseline is
//! rare rather than weekly: a node over the clearing it cuts in what stands
//! behind it, and the resting marker field standing in one node's light. Both are the parts a Shimmer or a Wash change
//! leaves alone, so a diff here on such a PR is the blast radius being wider
//! than its author believed — which is the one thing the claim tests cannot
//! say. Scenes that move by design stay out on purpose; a gate that fires on
//! every PR is one that gets blessed without being read, and a blind bless is
//! exactly the failure this exists to catch.
//!
//! **A changed golden is a stated picture change.** Re-baseline with
//! `HARMONIGRAPH_BLESS=1 cargo test -p harmonigraph-render golden`, look at
//! the contact sheet it names, and say in the PR body what moved and why.
//!
//! The frames are Metal-on-this-Mac specific. GitHub Actions is off and
//! `ci.sh` is the only gate, so that costs nothing today; a driver or OS
//! update re-baselines all of them at once, and its signature is every scene
//! moving by a small amount rather than one scene moving by a large one.

use super::fixtures::*;
use crate::*;
use std::path::PathBuf;

/// Wide enough that a marker's arms and the halo bridges between nodes are
/// several pixels across, and a multiple of 64 so `readback`'s 256-byte row
/// alignment holds.
const GOLDEN_SIZE: [u32; 2] = [256, 256];

/// How far a channel may drift before the frame counts as changed.
///
/// Zero: one machine, one driver, one backend, so a difference here is the
/// shader's or the scene's and not the platform's. A tolerance would have to
/// be wider than #453's mean of 3.3/255 to be worth having, and that is the
/// signal rather than the noise.
const TOLERANCE: u8 = 0;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden")
}

/// A node standing in front of what is drawn behind it, over a white ground.
///
/// The subject is the CLEARING — the hole a node cuts so that it reads as
/// being in front of the sheets and markers behind — which is depth ordering
/// and geometry rather than any dial's look. The ground is white because the
/// clearing is measured as what changed where the hole is, and over black
/// that is a few levels rather than a channel's whole range.
///
/// A mark is held, so the frame carries the bulge: the hole is the node's own
/// shape one reach out, so it swells over the wedge a mark extends and hugs
/// the rings everywhere else.
fn node_over_its_clearing() -> Scene {
    let mut scene = clearing_node(2, 0.6, true, 0.85);
    // The fixture's ground is white because its own tests read the clearing as
    // a difference against it. A golden is read as an absolute frame instead,
    // and a saturated channel records nothing: over white, 95% of these pixels
    // are pure black or pure white and no shader change can move them.
    scene.background = glam::Vec4::new(0.30, 0.31, 0.36, 1.0);
    // A halo over the whole frame, so the ground outside the node carries a
    // gradient rather than one flat value.
    scene.glow_reach = 4.0;
    scene.glow_strength = 2.0;
    scene.glow_feather = 1.0;
    scene
}

/// The resting marker field standing in one node's light.
///
/// This is where a marker's cross meets a node's halo: the ink, the standoff
/// it holds the light off by, and the shadow the cross casts. The shape of
/// that shadow is what #450's disc-for-a-cross mutation changes and what no
/// claim test in the suite can see, so this frame is the one carrying the
/// acceptance criterion.
fn resting_markers_in_one_light() -> Scene {
    // The dials the marker suite measures shadows at, so the frame shows the
    // moat rather than a marker sitting in undimmed light.
    shadowed_markers(0.85, 0.5, 1.0)
}

/// Mean and largest per-channel drift between two RGBA8 frames.
///
/// Both are reported because they separate the two ways a picture moves: a
/// mean near zero with a large max is a few pixels relocating (an edge, a
/// glyph), and a small max spread over a nonzero mean is a level shifting
/// everywhere — the #453 shape, and the one an eye does not catch.
fn drift(expected: &[u8], actual: &[u8]) -> (f64, u8) {
    let mut sum = 0u64;
    let mut max = 0u8;
    for (e, a) in expected.iter().zip(actual) {
        let d = e.abs_diff(*a);
        sum += u64::from(d);
        max = max.max(d);
    }
    (sum as f64 / expected.len() as f64, max)
}

fn write_png(path: &std::path::Path, size: [u32; 2], rgba: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("golden directory");
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("png header").write_image_data(rgba).expect("png data");
}

fn read_png(path: &std::path::Path) -> Option<(Vec<u8>, [u32; 2])> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader =
        png::Decoder::new(std::io::BufReader::new(file)).read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("png data");
    buf.truncate(info.buffer_size());
    Some((buf, [info.width, info.height]))
}

/// Expected, actual, and the difference at 8x, side by side in one image.
///
/// The amplification is the point: the drift this gate exists to catch is a
/// handful of levels, which is invisible in a raw subtraction and obvious at
/// 8x. Written on failure only, so the working tree stays clean while the
/// gate is passing.
fn write_contact_sheet(name: &str, expected: &[u8], actual: &[u8]) -> PathBuf {
    let [w, h] = GOLDEN_SIZE;
    let (w, h) = (w as usize, h as usize);
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
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/golden-diff")
        .join(format!("{name}.png"));
    write_png(&path, [GOLDEN_SIZE[0] * 3, GOLDEN_SIZE[1]], &sheet);
    path
}

fn check(name: &str, scene: &Scene) {
    let Some(mut shooter) = Shooter::new(GOLDEN_SIZE) else {
        return;
    };
    let actual = shooter.shot(scene);
    let path = golden_dir().join(format!("{name}.png"));

    if std::env::var_os("HARMONIGRAPH_BLESS").is_some() {
        let before = read_png(&path).map(|(px, _)| drift(&px, &actual));
        write_png(&path, GOLDEN_SIZE, &actual);
        match before {
            Some((mean, max)) if max > 0 => {
                eprintln!("blessed {name}: mean {mean:.3}, max {max} — say what moved in the PR")
            }
            Some(_) => eprintln!("blessed {name}: unchanged"),
            None => eprintln!("blessed {name}: new frame"),
        }
        return;
    }

    let Some((expected, size)) = read_png(&path) else {
        panic!(
            "no golden frame at {}\nrun: HARMONIGRAPH_BLESS=1 cargo test -p harmonigraph-render golden",
            path.display()
        );
    };
    assert_eq!(size, GOLDEN_SIZE, "{name}: golden was written at a different size");

    let (mean, max) = drift(&expected, &actual);
    if max > TOLERANCE {
        let sheet = write_contact_sheet(name, &expected, &actual);
        panic!(
            "{name} moved: mean {mean:.3}/255, max {max}/255\n\
             expected | actual | difference at 8x: {}\n\
             If the change is intended, re-baseline and say what moved in the PR body:\n\
             HARMONIGRAPH_BLESS=1 cargo test -p harmonigraph-render golden",
            sheet.display()
        );
    }
}

/// A node over its clearing is byte-identical to the frame on record.
///
/// Depth ordering and the clearing's shape are what this frame holds, so a
/// diff here on a PR about how any one element LOOKS is reach its author did
/// not intend.
#[test]
fn a_node_over_its_clearing_draws_the_frame_on_record() {
    check("node-clearing", &node_over_its_clearing());
}

/// The resting marker field in one node's light is byte-identical to the
/// frame on record.
///
/// Carries the shadow's SHAPE, which the four marker-shadow tests each miss
/// for their own reason (#450): a disc substituted for the cross passes all
/// of them and fails this.
#[test]
fn the_resting_marker_field_draws_the_frame_on_record() {
    check("resting-markers-in-one-light", &resting_markers_in_one_light());
}
