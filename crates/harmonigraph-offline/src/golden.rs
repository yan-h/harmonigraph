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
//! set worth its runtime rather than one frame worth three:
//!
//! - a TALL and a SHORT pane, zoomed out. Both resample the same bucket-space
//!   image and differ only in how finely they sample it, so what the pair
//!   carries is the INVARIANCE #491 asked for — neither frame says anything
//!   about it alone. The bucket-space filter is what put them there: before it
//!   the short pane drew a mean of 75.5/255 against the tall one's 55.2, and
//!   after it 43.9 against 42.3 (#514's palette; under today's they read 91.9
//!   against 92.9).
//! - a ZOOMED-IN pane, where a row is narrower than a bucket and reads a lerp
//!   between two of them instead. Its half-bucket centre offset is one of the
//!   things #503's own trap list calls easy to lose in the port, and no
//!   zoomed-out frame executes it at all.
//!
//! All three draw the plain heatmap, every effect at zero, which is the
//! picture these claims were first measured on (see [`Shot::take`]).
//!
//! Each frame was held against the read it claims, by breaking that read and
//! measuring what moved. Flattening the minifying arm to a plain max over its
//! run moves the short pane by a mean of 38.1/255, the tall by 18.1,
//! and the zoomed-in one by 1.2 — the ordering the
//! footprint argument predicts, widest run first. Dropping the magnifying
//! arm's half-bucket centre offset moves the zoomed-in frame by 1.1 and the
//! other two by nothing at all. Re-measured on the plain frames in #1306; #514
//! measured 32.0, 14.8, 1.1 and 1.1 under its own palette.
//! Before that last number the zoomed-in shot was six semitones over 384
//! rows, which the two-octave floor widened under it into a mean on every
//! row: it drew a plausible frame, blessed, and measured nothing.
//!
//! One edge in the read has no frame here: the lerp's clamp at the topmost
//! bucket, which only a row zoomed in AND at the very top of the analyzer's
//! axis reaches, where this fixture is silent. `the_curve_and_the_heatmap_
//! read_a_run_of_buckets_alike` is what holds that, and PR B keeps it.
//!
//! **Every shot starts from [`harmonigraph_golden::LOOK`]**, never from the
//! fresh defaults, so a capture of a new DAW look moves none of these frames
//! and a diff here is something a PR's own code did.
//!
//! **A moved golden is a stated picture change**: `tools/golden-diff.py`
//! draws these frames from the base and from the branch and names each that
//! moved with a contact sheet, and the PR body says what moved and why. The
//! mechanism is [`harmonigraph_golden`]'s; what is here is the scenes, and
//! "the frame on record" in their names is the one the base draws.

use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS};
use harmonigraph_take::{Header, NoteKind, NoteRecord, Take};
use harmonigraph_ui::SpectrogramRender;

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
///
/// Every fixture that dials it in also pins the render's spectrogram to
/// Scrolling. The pinned look's choice is "Fit video", which replaces the span with
/// the render's own length — [`SECONDS`] — so this would be dead and the
/// window's oldest part the bed.
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
    Audio::from_samples(SAMPLE_RATE, samples, 1)
}

/// One golden frame: the pane's size and what it is dialled to.
pub(crate) struct Shot {
    /// Output pixels. The spectral fixture is full-bleed, so the height IS the
    /// pitch axis and decides the image's rows.
    pub(crate) size: [u32; 2],
    /// The displayed pitch range, in MIDI. Which arm of the row read runs is
    /// this against the height above: wider than a bucket per row is a mean,
    /// narrower is a lerp.
    pub(crate) range: (f32, f32),
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
/// agree with its base, and still read as coverage — which is the failure #450
/// named and the one a golden is least able to report on itself. It has already happened
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
    /// The take the #503 frames replay: no notes, and the pane dialled so the
    /// plain heatmap — every effect at zero — is the frame.
    ///
    /// No notes deliberately. A roll ribbon is painted OVER the heatmap on the
    /// same axes, so every note in the fixture is heatmap the gate cannot see —
    /// and the roll is not what #503 moves.
    ///
    /// The plain heatmap because that is what the bucket filter's claims were
    /// measured on: #514 predates every effect (#872 added the first). A blur
    /// or a texture over the frame would put a resample of their own
    /// between the filter and the gate, and the pinned look carries both.
    fn take(&self) -> Take {
        self.dialled(|a| {
            let atmosphere = &mut a.spectrum.atmosphere;
            atmosphere.pitch_softness = 0.0;
            atmosphere.time_softness = 0.0;
            atmosphere.cloud_depth = 0.0;
        })
    }

    /// The same take with one further turn of the appearance, for a shot that
    /// is about a setting rather than about the pane.
    pub(crate) fn dialled(
        &self,
        tune: impl FnOnce(&mut harmonigraph_ui::AppearanceDocument),
    ) -> Take {
        let mut appearance = pinned_look();
        let cfg = &mut appearance.spectrum;
        // These established goldens pin the immediate texture response.
        cfg.atmosphere.color_pickup = 0.0;
        cfg.atmosphere.color_release = 0.0;
        cfg.show_roll = false;
        // The whole depth to the spectrogram's region, which also drops the
        // live curve and its axis rulings through their own `split > 0`
        // guards. What is left is the heatmap and the frequency labels.
        cfg.roll_fraction = 1.0;
        cfg.roll_seconds = WINDOW;
        (cfg.low_midi, cfg.high_midi) = self.range;
        appearance.render.spectrogram = SpectrogramRender::Scrolling;
        tune(&mut appearance);
        Take {
            header: Header { appearance: Some(appearance.serialize()), ..Default::default() },
            events: Vec::new(),
            params: Vec::new(),
            configurations: Vec::new(),
            truncated: false,
            incomplete: None,
            ..Default::default()
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            layout: crate::frames::single_pane(harmonigraph_ui::Pane::Spectral),
            size: self.size,
            pixels_per_point: 1.0,
            fps: FPS,
            start: 0.0,
            end: SECONDS,
            audio_start: 0.0,
        }
    }
}

/// [`harmonigraph_golden::LOOK`], which every shot here is dialled from.
fn pinned_look() -> harmonigraph_ui::AppearanceDocument {
    harmonigraph_ui::AppearanceDocument::parse(harmonigraph_golden::LOOK)
        .expect("the golden look parses")
}

/// The pinned look names every field the document saves that a frame reads.
/// One it leaves out draws at its `Default`, so the next DAW capture that moves
/// that default moves the frames drawing it, which the pin exists to prevent.
///
/// Except `render`, the Video pane's export settings: no golden reads them
/// from the pin. Each takes its size and layout from its own `Settings`, and
/// the one field the renderer reads, `render.spectrogram`, every shot sets
/// itself (#1853).
#[test]
fn the_pinned_look_pins_every_drawn_field() {
    fn paths(value: &ron::Value, at: &str, out: &mut Vec<String>) {
        match value {
            ron::Value::Map(map) => {
                for (key, value) in map.iter() {
                    let ron::Value::String(key) = key else { continue };
                    let path = format!("{at}.{key}");
                    paths(value, &path, out);
                    out.push(path);
                }
            }
            ron::Value::Option(Some(value)) => paths(value, at, out),
            ron::Value::Seq(items) => items.iter().for_each(|item| paths(item, at, out)),
            _ => {}
        }
    }
    let keys = |text: &str| {
        let mut out = Vec::new();
        paths(&ron::from_str(text).expect("RON"), "", &mut out);
        out
    };
    let pinned = keys(harmonigraph_golden::LOOK);
    let missing: Vec<_> = keys(&pinned_look().serialize())
        .into_iter()
        .filter(|path| !path.starts_with(".render") && !pinned.contains(path))
        .collect();
    assert!(missing.is_empty(), "look.ron draws these at their Default: {missing:#?}");
}

/// Render `shot` and hold its LAST frame against the one on record.
///
/// The last, because that is the frame whose window is full: an earlier one is
/// a picture of history still arriving, which moves with the analyzer's hop
/// rather than with anything the heatmap does.
///
/// Local runs may skip without a GPU; CI requires one through the shared
/// device setup before this function can return without comparing pixels.
fn check(name: &str, shot: Shot) {
    let take = shot.take();
    check_take(name, shot, take);
}

fn check_take(name: &str, shot: Shot, take: Take) {
    let Some(last) = last_frame(&shot, take) else {
        eprintln!("skipping {name}: no usable GPU adapter");
        return;
    };
    harmonigraph_golden::Gate::new(env!("CARGO_MANIFEST_DIR")).check(name, shot.size, &last);
}

/// The LAST frame `take` renders at `shot`, or `None` without a GPU.
pub(crate) fn last_frame(shot: &Shot, take: Take) -> Option<Vec<u8>> {
    let settings = shot.settings();
    let mut audio = probe_audio();
    let mut replay = Replay::new(take);
    let mut last = Vec::new();
    let appearance = crate::render::appearance_for(replay.take(), None).unwrap();
    match render(&mut replay, Some(&mut audio), &settings, appearance, |bytes| {
        // Keep this frame and hand the previous one back to be drawn into
        // again.
        Ok(std::mem::replace(&mut last, bytes))
    }) {
        Ok(_) => Some(last),
        Err(e) if e.contains("no usable GPU adapter") => None,
        Err(e) => panic!("{e}"),
    }
}

/// A tall pane zoomed out draws the frame on record.
#[test]
fn a_tall_pane_zoomed_out_draws_the_frame_on_record() {
    check("spectrogram-tall-pane", Shot { size: TALL, range: whole_axis() });
}

/// A short pane zoomed out draws the frame on record.
///
/// Its partner above holds the same audio at the same zoom, so the pair is the
/// pane's height and nothing else — which is the whole of what #491 measured
/// and what a per-pixel-footprint mean makes unavoidable.
#[test]
fn a_short_pane_zoomed_out_draws_the_frame_on_record() {
    check("spectrogram-short-pane", Shot { size: SHORT, range: whole_axis() });
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
    check("spectrogram-zoomed-in", Shot { size: ZOOMED, range });
}

/// The watercolour wash draws the frame on record.
///
/// The three frames above draw the plain heatmap, so none of them reaches a
/// texture. This is the one that has the wash in it, and it earns its place
/// because nothing else
/// committed here executes `wash_cloud_tone` end to end — the claim tests
/// beside the shader each turn one dial and compare two frames, where this pins
/// the whole construction, every constant in it, at the pinned look's settings
/// for the texture.
#[test]
fn the_watercolour_wash_draws_the_frame_on_record() {
    let shot = Shot { size: TALL, range: whole_axis() };
    let take = shot.dialled(|a| {
        a.spectrum.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::Watercolor;
    });
    check_take("spectrogram-watercolour-wash", shot, take);
}

/// Scales draw the frame on record, at the pinned look's settings for them.
///
/// The one texture with neither a tile nor a starfield: `fs_velvet_tone` into
/// the tone target, then the composite's one tap. No other frame here reaches
/// that pass.
#[test]
fn scales_draw_the_frame_on_record() {
    let shot = Shot { size: TALL, range: whole_axis() };
    let take = shot.dialled(|a| {
        a.spectrum.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::VelvetScales;
    });
    check_take("spectrogram-scales", shot, take);
}

/// Stars at full resolution: every slice, palette position and far-to-near
/// compositing at the pane's own pixels.
#[test]
fn full_resolution_stars_draw_the_frame_on_record() {
    let shot = Shot { size: TALL, range: whole_axis() };
    let take = shot.dialled(|a| {
        a.spectrum.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::Stars;
        a.spectrum.atmosphere.stars.star_resolution = 1.0;
    });
    check_take("spectrogram-starfield", shot, take);
}

/// The pinned look's Stars settings draw into a reduced image, filtered up
/// into the pane.
#[test]
fn pinned_stars_draw_the_reduced_frame_on_record() {
    let shot = Shot { size: TALL, range: whole_axis() };
    let take = shot.dialled(|a| {
        a.spectrum.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::Stars;
    });
    check_take("spectrogram-starfield-reduced", shot, take);
}

/// A non-vacuous spectral-shadow frame: held roll notes cast the geometry
/// group's shadow while their labels and the axis cast the text group's. The
/// pair lands through separate pane compositors, so this is also the frame
/// that catches either old under-body black or the skin knockout being lost.
#[test]
fn spectral_shadows_draw_the_frame_on_record() {
    let shot = Shot { size: [320, 200], range: (48.0, 84.0) };
    let mut appearance = pinned_look();
    appearance.spectrum.atmosphere.color_pickup = 0.0;
    appearance.spectrum.atmosphere.color_release = 0.0;
    appearance.spectrum.show_roll = true;
    appearance.spectrum.roll_fraction = 0.65;
    appearance.spectrum.roll_seconds = WINDOW;
    appearance.render.spectrogram = SpectrogramRender::Scrolling;
    (appearance.spectrum.low_midi, appearance.spectrum.high_midi) = shot.range;
    let style = harmonigraph_scene::ShadowStyle { width: 0.75, depth: 0.85, ..Default::default() };
    appearance.view.shadow.spectral_text = style;
    let notes: Vec<_> = [60u8, 64, 67, 72]
        .into_iter()
        .map(|note| NoteRecord {
            source: 0,
            t: 0.0,
            channel: 0,
            note,
            kind: NoteKind::On { velocity: 0.85 },
        })
        .collect();
    let take = Take {
        header: Header { appearance: Some(appearance.serialize()), ..Default::default() },
        events: notes.into_iter().map(harmonigraph_take::CanonicalRecord::Note).collect(),
        params: Vec::new(),
        configurations: Vec::new(),
        truncated: false,
        incomplete: None,
        ..Default::default()
    };
    check_take("spectrogram-spectral-shadows", shot, take);
}
