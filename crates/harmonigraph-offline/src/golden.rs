//! Byte-exact spectral frames: what the heatmap draws, held against a build.
//!
//! The pane's own suite is claim tests — a run of buckets reads alike two
//! ways, the floor settles as the axis zooms out, cells run dark to bright —
//! and each passes for any picture holding its property. That is the right
//! shape while the look is moving and the wrong one for #503, which rewrites
//! the compose as a fragment shader and has to say the picture did not move.
//! Nothing else in the tree can say that: the heatmap is built on the CPU and
//! uploaded, so a claim test can read the arithmetic and never the pixels.
//!
//! Here rather than in `harmonigraph-ui`, because the spectrogram is not a
//! function anything can call for a frame — it is a pane, an egui texture and
//! a mesh, and this crate is where those are drawn without a window. The set
//! is the offline renderer's, so it also holds the exports honest, which is
//! the half of the picture the DAW cannot show.
//!
//! **Each frame reaches one thing #503 changes**, and that is what makes the
//! set worth its runtime rather than one frame worth four:
//!
//! - a TALL and a SHORT pane, zoomed out. Both take a power mean over a run of
//!   buckets, and their difference is the per-pixel footprint — the
//!   8.7 dB pane-height dependence of #491, which the port (PR B) must leave
//!   exactly where it is and the bucket-space filter (PR C) exists to remove.
//!   Neither frame carries that alone; the pair does.
//! - a ZOOMED-IN pane, where a row is narrower than a bucket and reads a lerp
//!   between two of them instead. Its half-bucket centre offset is one of the
//!   things #503's own trap list calls easy to lose in the port, and no
//!   zoomed-out frame executes it at all.
//! - the WHOLE-SONG layout, which folds its own grid and owns its texture
//!   outright rather than scrolling a ring. It is a second build path, ported
//!   separately, and the live frames say nothing about it.
//!
//! Each frame was held against the read it claims, by breaking that read and
//! measuring what moved. Flattening the power mean to a plain max moves the
//! short pane by a mean of 7.9/255, the tall by 4.3 and the whole-song frame
//! by 3.8, and the zoomed-in one by 0.4 — the ordering the footprint argument
//! predicts, widest run first. Dropping the lerp's half-bucket centre offset
//! moves the zoomed-in frame by 1.1 and the other three by nothing at all.
//! Before that last number the zoomed-in shot was six semitones over 384
//! rows, which the two-octave floor widened under it into a mean on every
//! row: it drew a plausible frame, blessed, and measured nothing.
//!
//! One edge in the read has no frame here: the lerp's clamp at the topmost
//! bucket, which only a row zoomed in AND at the very top of the analyzer's
//! axis reaches, where this fixture is silent. `the_curve_and_the_heatmap_
//! read_a_run_of_buckets_alike` is what holds that, and PR B keeps it.
//!
//! **A changed golden is a stated picture change**: re-baseline with
//! `HARMONIGRAPH_BLESS=1 cargo test --workspace golden`, read the
//! contact sheet it names, and say in the PR body what moved and why. The
//! comparison and the sheet are [`harmonigraph_golden::Gate`].
//!
//! The frames are Metal-on-this-Mac specific, like the lattice set's. A driver
//! or OS update re-baselines all of them at once, and its signature is every
//! frame moving a little rather than one moving a lot.

use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS};
use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_take::{Header, Take};
use harmonigraph_ui::{Layout, SharedState};

use crate::render::{render, Settings};
use crate::replay::Replay;
use crate::wav::Audio;

/// Seconds of audio the shot plays before the frame is taken.
///
/// The frame is the LAST one, so this is also how much history the window has
/// to draw from — see [`WINDOW`].
const SECONDS: f64 = 2.0;

/// Seconds of history the pane's depth axis spans.
///
/// Under [`SECONDS`], so the last frame's window is covered end to end. At the
/// dial's own 180 s the whole take would be one column against the now-line and
/// the rest of the frame the bed, which is a picture of the fixture being too
/// short rather than of the heatmap.
const WINDOW: f32 = 1.5;

/// Frames per second the shot renders at.
///
/// It does not decide what the heatmap holds — the analyzer is fed one frame's
/// worth of samples per frame and stamps its columns off its own hop, so the
/// same audio makes the same columns at any rate — so this is purely how many
/// times the pane is drawn to get to the last one.
const FPS: f64 = 10.0;

const SAMPLE_RATE: f32 = 48_000.0;

/// A tone stack: when it enters, its fundamental in Hz, and its level.
///
/// Partials at every multiple, at `1/k`, which is what puts energy across the
/// whole analyzer axis from three fundamentals — a zoomed-out frame spans ten
/// octaves, and a picture of three lines in it says nothing about the run
/// between them.
struct Tone {
    at: f64,
    hz: f32,
    gain: f32,
}

/// Three stacks entering in turn, so the time axis carries edges rather than
/// stripes: a column of the heatmap differs from its neighbour, and a build
/// that lost the time axis draws something a diff can see.
const TONES: [Tone; 3] = [
    Tone { at: 0.0, hz: 55.0, gain: 0.25 },
    Tone { at: 0.55, hz: 220.0, gain: 0.2 },
    Tone { at: 1.1, hz: 880.0, gain: 0.15 },
];

/// Partials per stack. The top one of the 880 Hz stack lands at 21 kHz, inside
/// Nyquist at [`SAMPLE_RATE`] — an aliased partial would fold down onto a
/// bucket that has nothing to do with the note and pin the golden to the
/// arithmetic of the fold.
const PARTIALS: usize = 24;

/// Amplitude of the broadband bed under the tones.
///
/// The bed is not decoration. A row zoomed out averages a run of buckets, and
/// over digital silence every bucket in the run holds the same value — where
/// the power mean, a plain max and an average all agree, so the read the port
/// has to preserve is invisible. It is also what #491's 8.7 dB is measured on:
/// the FLOOR is what moves with the pane's height, not the partials.
const BED: f32 = 0.02;

/// A deterministic bed: a plain LCG, so the same shot draws the same noise on
/// every machine and every run.
///
/// `rand` would be a dependency for one line, and a seeded generator from it
/// would still be a stream this crate does not own — a version bump could
/// re-baseline every frame here for no picture reason at all.
fn bed(step: &mut u32) -> f32 {
    *step = step.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*step >> 8) as f32 / (1 << 23) as f32 * 2.0 - 1.0
}

/// [`SECONDS`] of mono audio: [`TONES`] over a [`BED`].
fn probe_audio() -> Audio {
    let frames = (SECONDS * f64::from(SAMPLE_RATE)) as usize;
    let mut rng = 0x5eed_1234;
    let samples = (0..frames)
        .map(|f| {
            let t = f as f64 / f64::from(SAMPLE_RATE);
            let tones: f32 = TONES
                .iter()
                .filter(|tone| t >= tone.at)
                .flat_map(|tone| {
                    (1..=PARTIALS).map(move |k| {
                        let hz = f64::from(tone.hz) * k as f64;
                        tone.gain / k as f32
                            * (std::f64::consts::TAU * hz * (t - tone.at)).sin() as f32
                    })
                })
                .sum();
            tones + BED * bed(&mut rng)
        })
        .collect();
    Audio { sample_rate: SAMPLE_RATE, samples, channels: 1 }
}

/// One golden frame: the pane's size and what it is dialled to.
struct Shot {
    /// Output pixels. The `spectral` preset is full-bleed, so the height IS the
    /// pitch axis and decides the image's rows.
    size: [u32; 2],
    /// The displayed pitch range, in MIDI. Which arm of the row read runs is
    /// this against the height above: wider than a bucket per row is a mean,
    /// narrower is a lerp.
    range: (f32, f32),
    /// Lay the whole take out at once under a playhead, rather than scrolling
    /// a window.
    whole: bool,
}

/// The whole analyzer axis — where a fresh pane opens.
fn whole_axis() -> (f32, f32) {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    (SPECTRUM_MIN_MIDI, SPECTRUM_MAX_MIDI)
}

/// The taller of the two panes, in output pixels.
///
/// A multiple of the 64-pixel quantum the live build rounds its height up to,
/// so the image has exactly this many rows and the picture is not a shorter
/// one resampled.
const TALL: [u32; 2] = [256, 384];

/// The shorter pane: two quanta against the tall one's six.
const SHORT: [u32; 2] = [256, 128];

/// The pane the zoomed-in shot is drawn on: narrow, and taller than either of
/// the panes above.
///
/// The height is what the shot is FOR, and it is not a choice — a lerp needs a
/// row narrower than a bucket, `PITCH_RANGE_MIN_SPAN` floors the pane at two
/// octaves however far the range is dragged, and two octaves is 768 buckets.
/// So no pane under 768 rows reads a lerp anywhere at any zoom, and one at
/// twice that reads one on half its rows. The width pays for it: the time axis
/// is not what this frame is about, and a run of these at the pair's 256 would
/// be a megabyte of committed PNG for the same claim.
const ZOOMED: [u32; 2] = [128, 1536];

/// The pitch range the zoomed-in shot shows, in whole MIDI notes: two octaves
/// from ~370 Hz, where the 55 Hz stack lays down twenty partials with a couple
/// of semitones between the closest pair.
///
/// Two octaves EXACTLY, because that is the floor — a narrower pair of numbers
/// is widened back to it before anything reads them, so writing one here would
/// describe a picture the pane cannot draw.
const ZOOMED_IN: (u32, u32) = (66, 90);

/// The zoomed-in shot really is zoomed past one bucket per row.
///
/// Checked where the constants are rather than where the frame is drawn: a
/// range that quietly stopped reaching the lerp arm would still render, still
/// bless, and still read as coverage — which is the failure #450 named and the
/// one a golden is least able to report on itself. It has already happened
/// once here: at six semitones over 384 rows the arithmetic said every row was
/// a lerp, the floor above widened the range to twenty-four, and the frame
/// took the mean on all of them.
const _: () = assert!(
    (ZOOMED[1] as usize) > (ZOOMED_IN.1 - ZOOMED_IN.0) as usize * BINS_PER_SEMITONE,
    "the zoomed-in shot's rows are wider than a bucket, so it reads a mean like the pair \
     above and the lerp arm has no frame at all — raise the pane",
);

/// ...and its range is one the pane will actually show, rather than one the
/// two-octave floor widens under it.
const _: () = assert!(
    ZOOMED_IN.1 - ZOOMED_IN.0 >= 24,
    "a range under the analyzer's two-octave floor is widened before it is drawn, so the \
     span the assertion above is written against is not the span the frame has",
);

/// ...and the zoomed-out pair really is not.
///
/// Both panes have fewer rows than the axis has buckets, so every row covers a
/// run of them and takes the mean. The tall one covers ten, the short thirty.
const _: () = assert!(
    (TALL[1] as usize) < SPECTRUM_BINS && (SHORT[1] as usize) < SPECTRUM_BINS,
    "a zoomed-out pane with a row per bucket reads lerps, and the pair stops being about \
     the footprint a mean takes",
);

impl Shot {
    /// The take the shot replays: no notes, and the pane dialled so the
    /// heatmap is the frame.
    ///
    /// No notes deliberately. A roll ribbon is painted OVER the heatmap on the
    /// same axes, so every note in the fixture is heatmap the gate cannot see —
    /// and the roll is not what #503 moves.
    fn take(&self) -> Take {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let cfg = &mut state.spectrum_config;
        cfg.show_roll = false;
        // The whole depth to the spectrogram's region, which also drops the
        // live curve and its axis rulings through their own `split > 0`
        // guards. What is left is the heatmap and the frequency labels.
        cfg.roll_fraction = 1.0;
        cfg.roll_seconds = WINDOW;
        (cfg.low_midi, cfg.high_midi) = self.range;
        Take {
            header: Header { ui_state: Some(state.save_persist()), ..Default::default() },
            notes: Vec::new(),
            params: Vec::new(),
            truncated: false,
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            layout: Layout::preset("spectral").expect("the spectral preset exists"),
            size: self.size,
            pixels_per_point: 1.0,
            fps: FPS,
            start: 0.0,
            end: SECONDS,
            audio_start: 0.0,
            whole_song_spectrogram: self.whole,
        }
    }
}

/// Render `shot` and hold its LAST frame against the one on record.
///
/// The last, because that is the frame whose window is full: an earlier one is
/// a picture of history still arriving, which moves with the analyzer's hop
/// rather than with anything the heatmap does.
///
/// A machine with no usable GPU adapter draws nothing and asserts nothing —
/// the same skip `render::tests` takes.
fn check(name: &str, shot: Shot) {
    let settings = shot.settings();
    let audio = probe_audio();
    let mut replay = Replay::new(shot.take());
    let mut last = Vec::new();
    match render(&mut replay, Some(&audio), &settings, |bytes| {
        last.clear();
        last.extend_from_slice(bytes);
        Ok(true)
    }) {
        Ok(_) => {}
        Err(e) if e.contains("no usable GPU adapter") => {
            eprintln!("skipping {name}: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    harmonigraph_golden::Gate::new(env!("CARGO_MANIFEST_DIR")).check(name, shot.size, &last);
}

/// A tall pane zoomed out draws the frame on record.
#[test]
fn a_tall_pane_zoomed_out_draws_the_frame_on_record() {
    check("spectrogram-tall-pane", Shot { size: TALL, range: whole_axis(), whole: false });
}

/// A short pane zoomed out draws the frame on record.
///
/// Its partner above holds the same audio at the same zoom, so the pair is the
/// pane's height and nothing else — which is the whole of what #491 measured
/// and what a per-pixel-footprint mean makes unavoidable.
#[test]
fn a_short_pane_zoomed_out_draws_the_frame_on_record() {
    check("spectrogram-short-pane", Shot { size: SHORT, range: whole_axis(), whole: false });
}

/// A pane zoomed in past one bucket per row draws the frame on record.
///
/// Two rows to a bucket, so half of them lie wholly inside one and read
/// between its neighbours; the rest straddle a boundary and take a two-bucket
/// mean. That mix is the picture at the tightest zoom the pane has, not a
/// weakness of the fixture — the arms are chosen per row, and no range makes
/// one of them unanimous.
#[test]
fn a_zoomed_in_pane_draws_the_frame_on_record() {
    let range = (ZOOMED_IN.0 as f32, ZOOMED_IN.1 as f32);
    check("spectrogram-zoomed-in", Shot { size: ZOOMED, range, whole: false });
}

/// The whole-song layout draws the frame on record.
#[test]
fn the_whole_song_layout_draws_the_frame_on_record() {
    check("spectrogram-whole-song", Shot { size: TALL, range: whole_axis(), whole: true });
}
