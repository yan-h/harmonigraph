//! Byte-exact spectral frames: what the heatmap draws, held against a build.
//!
//! The pane's own suite is claim tests — a run of buckets reads alike two
//! ways, the floor settles as the axis zooms out, cells run dark to bright —
//! and each passes for any picture holding its property. That is the right
//! shape while the look is moving and the wrong one for #503, which rewrites
//! the compose as a fragment shader and has to say exactly what the picture
//! did.
//! Nothing else in the tree can say that: a claim test reads the arithmetic
//! and never the pixels the shader actually writes.
//!
//! Here rather than in `harmonigraph-ui` because the set is the offline
//! renderer's: it holds the EXPORTS honest, which is the half of the picture
//! the DAW cannot show, and a pane drawn without a window is what this crate
//! is for.
//!
//! **Each frame reaches one thing #503 changes**, and that is what makes the
//! set worth its runtime rather than one frame worth four:
//!
//! - a TALL and a SHORT pane, zoomed out. Both resample the same bucket-space
//!   image and differ only in how finely they sample it, so what the pair
//!   carries is the INVARIANCE #491 asked for — neither frame says anything
//!   about it alone. The bucket-space filter is what put them there: before it
//!   the short pane drew a mean of 75.5/255 against the tall one's 55.2, and
//!   after it 43.9 against 42.3.
//! - a ZOOMED-IN pane, where a row is narrower than a bucket and reads a lerp
//!   between two of them instead. Its half-bucket centre offset is one of the
//!   things #503's own trap list calls easy to lose in the port, and no
//!   zoomed-out frame executes it at all.
//! - the WHOLE-SONG layout, which folds a grid over the entire take rather
//!   than scrolling a window across it. It is a second build path, ported
//!   separately, and the live frames say nothing about it.
//!
//! Each frame was held against the read it claims, by breaking that read and
//! measuring what moved. Flattening the minifying arm to a plain max over its
//! run moves the short pane by a mean of 32.0/255, the tall by 14.8 and the
//! whole-song frame by 12.5, and the zoomed-in one by 1.1 — the ordering the
//! footprint argument predicts, widest run first. Dropping the magnifying
//! arm's half-bucket centre offset moves the zoomed-in frame by 1.1 and the
//! other three by nothing at all.
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
/// The bed is not decoration. A row zoomed out resamples a run of buckets, and
/// over digital silence every bucket in the run holds the same value — where a
/// mean, a plain max and a lerp all agree, so the read the set is held against
/// is invisible. It is also what makes the tall/short pair a measurement at
/// all: the floor between the partials is the part of the picture a change of
/// pane height moves most.
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
/// Its height is what the pair measures: the same bucket-space image is
/// resampled to exactly this many rows, so a frame that moves with the pane's
/// height rather than with its contents shows up against [`SHORT`].
const TALL: [u32; 2] = [256, 384];

/// The shorter pane, a third of [`TALL`]'s height on the same width — far
/// enough apart that a per-pixel footprint which does not tile the axis
/// separates the two means.
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

/// Frames per second the cost measurement renders at.
///
/// Higher than [`FPS`] to get more frames out of the same audio: [`SECONDS`]
/// at this rate is 240 of them, which puts the fixed cost of standing a
/// renderer up far enough below the per-frame figure to stop mattering.
///
/// It is not free of the picture, so the DIFFERENCE columns are what to read
/// and the absolute ones are not. A frame is fed exactly its own step of
/// audio, so at 120 fps each carries half the samples a 60 fps frame does —
/// half the analyzer columns to transform, and half as many to fold into the
/// grid. That share sits in all three configurations and subtracts out; what
/// it leaves behind is an absolute per-frame figure under what the shipping
/// rate pays.
const TIMING_FPS: f64 = 120.0;

/// Frames dropped off the front of a render before the clock starts.
///
/// Every render builds its own device, so every render compiles the shaders
/// and creates the pipelines it touches — and the heatmap's are built lazily,
/// on the first frame that has two analyzer columns to draw rather than on
/// frame 0. Timed from frame 1 that compile lands INSIDE the span, on the one
/// row it can bias: neither control ever creates that pipeline, so it does not
/// subtract out. Enough frames to cover it, and small beside the 240 a render
/// emits.
const WARMUP: u64 = 16;

/// Fraction of the frame's height the control's pane is squeezed into.
const SLIVER: f32 = 0.02;

/// What the pane is asked to draw for one row of the table.
#[derive(Clone, Copy, PartialEq)]
enum Drawn {
    /// The pane's whole depth given to the heatmap.
    Heatmap,
    /// None of it, which leaves the live curve and its rulings — egui shapes
    /// built on the CPU, and the thing the heatmap replaced the compose of.
    Curve,
    /// The same pane, in a sliver of the frame — [`SLIVER`] of its height,
    /// the rest background. A layout with NO panes is refused by the renderer,
    /// and this is the nearest thing to it: every stage still runs and the
    /// pane still draws, over about a fiftieth of the pixels. It is the
    /// control the other two are measured against, and it carries that
    /// fiftieth with it, so a difference reads a hair low.
    Sliver,
}

/// Milliseconds a frame takes end to end, and how many were rendered.
fn frame_ms(size: [u32; 2], drawn: Drawn) -> Option<(f64, u64)> {
    let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
    let cfg = &mut state.spectrum_config;
    cfg.show_roll = false;
    cfg.roll_fraction = if drawn == Drawn::Heatmap { 1.0 } else { 0.0 };
    cfg.roll_seconds = WINDOW;
    (cfg.low_midi, cfg.high_midi) = whole_axis();
    let take = Take {
        header: Header { ui_state: Some(state.save_persist()), ..Default::default() },
        notes: Vec::new(),
        params: Vec::new(),
        truncated: false,
    };
    let mut layout = Layout::preset("spectral").expect("the spectral preset exists");
    if drawn == Drawn::Sliver {
        // A placement's rect is `(x0, y0, x1, y1)`, so the sliver is taken from
        // the pane's OWN top edge rather than the frame's. Floored at two
        // pixels because `Layout::resolve` drops a pane under one pixel tall,
        // and a layout that resolves to no panes is an error out of the
        // renderer — a panic where a reading was asked for.
        let floor = 2.0 / size[1] as f32;
        for placement in &mut layout.panes {
            placement.rect.3 = placement.rect.1 + SLIVER.max(floor);
        }
    }
    let settings = Settings {
        layout,
        size,
        pixels_per_point: 1.0,
        fps: TIMING_FPS,
        start: 0.0,
        end: SECONDS,
        audio_start: 0.0,
        whole_song_spectrogram: false,
    };
    let audio = probe_audio();
    let mut replay = Replay::new(take);
    // The clock starts past [`WARMUP`] frames and not at the call: standing a
    // renderer up costs a few hundred milliseconds of adapter and pipeline
    // creation, which divided over the frames is the same order as the
    // per-frame figure being measured — and the last of it is paid a few
    // frames INTO the render rather than before it.
    let mut seen = 0u64;
    let mut first: Option<std::time::Instant> = None;
    let mut last = None;
    let mut frames = 0u64;
    match render(&mut replay, Some(&audio), &settings, |_| {
        seen += 1;
        if seen <= WARMUP {
            return Ok(true);
        }
        let now = std::time::Instant::now();
        if first.is_none() {
            first = Some(now);
        } else {
            frames += 1;
        }
        last = Some(now);
        Ok(true)
    }) {
        Ok(_) => {}
        Err(e) if e.contains("no usable GPU adapter") => return None,
        Err(e) => panic!("{e}"),
    }
    // A render that finished with nothing left to time is a broken fixture,
    // not a missing GPU — which is what the `None` above is read as.
    assert!(
        frames > 0,
        "{seen} frames rendered, under the two past the {WARMUP}-frame warm-up an interval needs"
    );
    let span = last.zip(first).map_or(0.0, |(l, f)| l.duration_since(f).as_secs_f64() * 1000.0);
    Some((span / frames as f64, frames))
}

/// One size's readings: the three configurations' own medians, and the two
/// costs, which are medians of PER-ROUND differences rather than differences
/// of the medians above.
struct Round {
    /// Sliver, heatmap, curve — the frame each configuration draws, in ms.
    columns: [f64; 3],
    /// Median and scatter, in ms.
    heatmap_cost: (f64, f64),
    curve_cost: (f64, f64),
    frames: u64,
}

/// What drawing the heatmap costs a rendered frame, end to end.
///
/// The same take rendered at one size three ways — the pane's whole depth to
/// the heatmap, the same depth to the live curve, and the pane squeezed into a
/// sliver — so the analyzer, the replay, egui, the encode and the readback sit
/// in all three and difference out. [`Drawn::Sliver`] is the control: what is
/// left over it is what drawing that pane costs through the path that ships.
///
/// It has to be measured in a frame rather than around one. A microbenchmark
/// of the draw alone cannot see it: the per-draw command overhead is larger
/// than the fragment work, and a submit-and-wait costs milliseconds of round
/// trip either way.
///
/// `#[ignore]`, and it asserts nothing: the numbers belong to the machine that
/// ran them.
#[test]
#[ignore]
fn what_the_heatmap_costs_a_frame() {
    // Sizes the pane is really dragged to, ending past a full Retina window:
    // the brief predicts the heatmap's share barely moves with the pitch axis,
    // because a taller pane reads shorter runs and the runs tile the buckets
    // either way.
    let sizes = [[256u32, 128], [256, 384], [512, 768], [1024, 1536], [2048, 1024]];
    eprintln!("\n== what one rendered frame costs, and the heatmap's share ==");
    eprintln!(
        "{:>12}  {:>7}  {:>7}  {:>7}   {:>12}  {:>12}  {:>6}",
        "size", "sliver", "heatmap", "curve", "heatmap-", "curve-", "px"
    );
    // Readings scatter by a few tenths of a millisecond, so each figure is the
    // median of REPS renders.
    // Interleaved, not blocked: this machine drifts by most of a millisecond
    // over a minute, and three runs of one config followed by three of another
    // turn that drift into a difference between them. One of each per round
    // puts the drift in all three equally, where the subtraction removes it.
    // ROTATED within the round for the same reason one scale down: a drift
    // ACROSS the three renders of a single round is a ramp that a fixed order
    // samples at a fixed offset per configuration, which is precisely the
    // difference being printed. REPS is a multiple of three, so every
    // configuration spends the same number of rounds in each position.
    const ORDER: [Drawn; 3] = [Drawn::Sliver, Drawn::Heatmap, Drawn::Curve];
    const REPS: usize = 9;
    let round = |size| -> Option<Round> {
        let mut runs = [const { Vec::new() }; 3];
        let mut frames = 0;
        // A throwaway round in front, for whatever the process caches once —
        // the driver's on-disk function cache, the first touch of the adapter.
        // Not for pipeline creation: a render builds its own device, so every
        // render pays that, and [`WARMUP`] is what keeps it off the clock.
        for &drawn in &ORDER {
            frame_ms(size, drawn)?;
        }
        for rep in 0..REPS {
            for step in 0..ORDER.len() {
                let slot = (rep + step) % ORDER.len();
                let (ms, n) = frame_ms(size, ORDER[slot])?;
                runs[slot].push(ms);
                frames = n;
            }
        }
        // PAIRED: each round's difference is taken first and the median is of
        // those, never of the three columns separately. A median per column
        // and a subtraction after it lets one polluted render move the answer
        // — a single 1.4 ms excursion in the control's column dragged a
        // difference from 1.19 ms to 0.31 ms with the other two columns
        // steady, because nothing in that arrangement knows the two readings
        // belong to the same round. Here such a round is one sample among
        // REPS and the median steps over it. The columns are still printed,
        // as medians of their own, for the shape of the frame they describe —
        // the DIFFERENCE columns are the measurement.
        let mut costs = [Vec::with_capacity(REPS), Vec::with_capacity(REPS)];
        let [sliver, heatmap, curve] = &runs;
        for ((base, heat), curv) in sliver.iter().zip(heatmap).zip(curve) {
            costs[0].push(heat - base);
            costs[1].push(curv - base);
        }
        let mid = |times: &mut Vec<f64>| {
            times.sort_by(f64::total_cmp);
            times[times.len() / 2]
        };
        // Half the span the middle REPS-2 samples cover, printed beside each
        // cost. It is what says whether a row may be quoted: this machine
        // moves a whole render by a millisecond under contention, and a row
        // whose spread is the size of its own figure is the noise, not the
        // heatmap. It is not an error bar on a mean — nothing here is
        // normally distributed — just the scatter the median stepped over.
        let spread = |times: &mut Vec<f64>| {
            times.sort_by(f64::total_cmp);
            (times[times.len() - 2] - times[1]) / 2.0
        };
        let mut columns = [0.0; 3];
        for (slot, times) in runs.iter_mut().enumerate() {
            columns[slot] = mid(times);
        }
        Some(Round {
            columns,
            heatmap_cost: (mid(&mut costs[0]), spread(&mut costs[0])),
            curve_cost: (mid(&mut costs[1]), spread(&mut costs[1])),
            frames,
        })
    };
    for size in sizes {
        let Some(r) = round(size) else {
            eprintln!("no usable GPU adapter; skipping");
            return;
        };
        let [sliver, heatmap, curve] = r.columns;
        let px = size[0] * size[1];
        eprintln!(
            "{:>5} x {:<4}  {sliver:>7.3}  {heatmap:>7.3}  {curve:>7.3}   {:>6.3} ±{:<5.3}  {:>6.3} ±{:<5.3}  {:>5.2}M  ({} frames)",
            size[0],
            size[1],
            r.heatmap_cost.0,
            r.heatmap_cost.1,
            r.curve_cost.0,
            r.curve_cost.1,
            px as f64 / 1.0e6,
            r.frames,
        );
    }
    eprintln!(
        "(heatmap- and curve- are each pane's cost over the sliver frame, as\n \
         median ± the scatter the median stepped over; a row whose scatter is\n \
         the size of its figure is this machine, not the pane)\n"
    );
}
