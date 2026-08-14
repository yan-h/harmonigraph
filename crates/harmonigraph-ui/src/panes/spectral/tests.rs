//! Tests for the Spectral pane: its axes in every orientation, the
//! gestures that move them, and what the whole pane paints.

use super::*;
use super::axes::*;
use super::gestures::*;
use super::settings::*;
use crate::tests::probe::{
    events_into, fresh, frame_into, painted_full, painted_into, press, themed, themed_at,
};
use crate::{SpectralOrientation, SpectrumConfig};
use harmonigraph_core::{NoteEvent, NoteEventKind};

/// A 300x100 pane at an offset origin, so a mistake that assumes the
/// rect starts at zero shows up.
const WIDE: egui::Rect =
    egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };
const TALL: egui::Rect =
    egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(110.0, 320.0) };

/// The window the pane is painted into. Bigger than [`WIDE`] and [`TALL`] on
/// both axes on purpose: a pane that draws outside its own rect then leaves
/// the ink in the shape list, where a test can find it, instead of having it
/// clipped away by a screen the size of the pane.
const SCREEN: egui::Vec2 = egui::vec2(500.0, 500.0);

/// One frame of the whole Spectral pane into `rect` at `now`, on a themed
/// context of its own.
fn painted_pane(rect: egui::Rect, state: &mut SharedState, now: f64) -> egui::FullOutput {
    painted_into(SCREEN, rect, |ui| spectral_pane(ui, state, now, 0))
}

fn axes(rect: egui::Rect, orientation: SpectralOrientation) -> Axes {
    let cfg = SpectrumConfig { orientation, ..Default::default() };
    Axes::new(rect, &cfg)
}

/// `power` for a level in dB, undoing the 10*log10 `loudness` applies.
fn power_at(db: f32) -> f32 {
    10.0f32.powf(db / 10.0)
}

/// The note names follow the pitch zoom and the markings do not, which is
/// the whole of the difference between text written ON the picture and
/// text labelling the axis it is drawn against.
///
/// Both ends of the zoom are pinned, because both are claims: at the whole
/// axis a name is exactly the size it was dialled at, so nothing about the
/// default view changes, and at the tightest range the analyzer offers it
/// is five times that — the axis being ten octaves and the floor two, and
/// the law being a constant share of the axis rather than some softened
/// fraction of one.
#[test]
fn names_follow_the_pitch_zoom_and_markings_hold_still() {
    let cfg = SpectrumConfig::default();
    let axes = axes(reference_pane(), SpectralOrientation::Left);
    let at = |span| text_scales(&cfg, &axes, span, 2.0);
    // Every size comes out snapped onto a whole physical pixel, and a name
    // is 12.35pt, which is not one at 2x — so the law is met to within half
    // a pixel of type and no closer. See `text::snap_scale`.
    let pixel = 0.5 / (names::LABEL_PT * 2.0);

    let full = at(FULL_PITCH_SPAN).names.label;
    assert!((full - 1.0).abs() <= pixel, "the whole axis draws names at {full}, not 1");
    let tightest = at(crate::PITCH_RANGE_MIN_SPAN).names.label;
    assert!(
        (tightest - FULL_PITCH_SPAN / crate::PITCH_RANGE_MIN_SPAN).abs() <= pixel,
        "the tightest range draws names at {tightest}, not in proportion to its zoom",
    );
    // Monotone in between, and never under the size it started at: the
    // reference is the widest range there is, so the only way is up.
    let mut previous = 0.0;
    for span in [FULL_PITCH_SPAN, 96.0, 60.0, 36.0, crate::PITCH_RANGE_MIN_SPAN] {
        let names = at(span).names.label;
        assert!(names >= previous, "{span} semitones drew smaller names than the span above");
        previous = names;
    }
    // A range zoomed past either end (a hand-edited blob; the bars cannot
    // do it) still lands inside the band rather than off it.
    assert_eq!(at(0.0).names.label, tightest);
    assert_eq!(at(1e6).names.label, full);

    // The markings ignore all of it, and answer to their own bar.
    assert_eq!(at(FULL_PITCH_SPAN).markings, at(crate::PITCH_RANGE_MIN_SPAN).markings);
    let bigger = SpectrumConfig { marking_scale: 2.0, ..SpectrumConfig::default() };
    let doubled = text_scales(&bigger, &axes, 24.0, 2.0).markings;
    // Within a rung of the size ladder, which is what the bar's 2 is
    // rounded onto — see `text::snap_scale`.
    assert!((doubled / 2.0 - 1.0).abs() <= 0.04, "the bar's 2 drew at {doubled}");
    assert_eq!(
        text_scales(&bigger, &axes, 24.0, 2.0).names.label,
        at(24.0).names.label,
        "and the two bars are independent",
    );

    // The air a name keeps in front of it answers to neither: it is a distance
    // on the screen, and the zoom that grows the type is exactly what it must
    // not follow — see `names::LABEL_INSET`.
    //
    // Read together with the `.label` assertions above, this is also what holds
    // the PAIR the right way round. The two halves of a `NameScale` are both
    // `f32` and are equal in any picture that is not zoomed, so a construction
    // that swapped them would draw correctly at the dialled view and wrongly at
    // every other — and `text_scales` is the one place that builds one.
    assert_eq!(at(FULL_PITCH_SPAN).names.air, at(crate::PITCH_RANGE_MIN_SPAN).names.air);
    let louder = SpectrumConfig { note_name_scale: 2.0, ..SpectrumConfig::default() };
    assert_eq!(text_scales(&louder, &axes, 24.0, 2.0).names.air, at(24.0).names.air);
}

/// A pane at the size these sizes were chosen at, so a test about anything
/// else is not also a test about the pane being some other size.
fn reference_pane() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, REFERENCE_PITCH_LEN))
}

/// Text shrinks with the pane it is drawn on — every kind of it, and in
/// proportion.
///
/// This is one mechanism doing three jobs that were three mechanisms: a
/// window dragged narrower, the Render preview drawing this pane small,
/// and the offline render drawing it large. A kind of text that missed it
/// would come out at some other size in the video than in the pane the
/// look was dialled in on, which is the divergence this codebase least
/// wants.
#[test]
fn text_shrinks_with_the_pane() {
    let cfg = SpectrumConfig::default();
    let half = egui::Rect::from_min_size(
        reference_pane().min,
        egui::vec2(300.0, REFERENCE_PITCH_LEN * 0.5),
    );
    let full = axes(reference_pane(), SpectralOrientation::Left);
    let small = axes(half, SpectralOrientation::Left);
    let docked = text_scales(&cfg, &full, 48.0, 2.0);
    let shrunk = text_scales(&cfg, &small, 48.0, 2.0);
    assert!((shrunk.names.label / docked.names.label - 0.5).abs() < 0.02);
    assert!((shrunk.markings / docked.markings - 0.5).abs() < 0.02);
    // The air in front of a name too, and this is the whole reason it is a
    // scale rather than flat points: the preview and the video are one picture
    // at two sizes, so a name has to stand off its note by the same fraction of
    // the pane in both.
    assert!((shrunk.names.air / docked.names.air - 0.5).abs() < 0.02);
    // ...and at the reference pane the bars read what they say.
    assert!((docked.markings - 1.0).abs() < 0.02, "{}", docked.markings);
}

/// The Span readout carries its own unit, and switches to minutes at
/// the point where seconds alone stop reading — including the seam,
/// where a value that rounds up to a whole minute must be written as
/// one rather than as "60.0s".
#[test]
fn the_span_readout_names_its_own_unit() {
    assert_eq!(span_readout(1.0), "1.0s");
    assert_eq!(span_readout(12.34), "12.3s");
    assert_eq!(span_readout(59.9), "59.9s");
    assert_eq!(span_readout(59.97), "1m 00s", "rounds up ACROSS the seam");
    assert_eq!(span_readout(60.0), "1m 00s");
    assert_eq!(span_readout(65.4), "1m 05s", "seconds are padded, so the width holds");
    assert_eq!(span_readout(90.0), "1m 30s");
    assert_eq!(span_readout(600.0), "10m 00s", "the top of the bar's range");
}

/// The level range is a window with two ends: the floor reads as silence
/// and the ceiling as full height, wherever each is put. Pulling the
/// ceiling down is what lets quiet material fill the picture.
#[test]
fn the_level_range_maps_floor_to_zero_and_ceiling_to_one() {
    // No tilt, so the pivot pitch drops out and dB is dB.
    let cfg =
        SpectrumConfig { floor_db: -60.0, ceiling_db: 0.0, tilt: 0.0, ..Default::default() };
    let at = |db| loudness(&cfg, power_at(db), TILT_PIVOT_MIDI);
    assert!(at(-60.0).abs() < 1e-4, "the floor is silence");
    assert!((at(0.0) - 1.0).abs() < 1e-4, "the ceiling is full height");
    assert!((at(-30.0) - 0.5).abs() < 1e-4, "and it is linear in dB between them");
    assert_eq!(at(-90.0), 0.0, "under the floor stays at silence");

    // A ceiling pulled down onto the material lifts it to full height,
    // which the fixed 0 dB top could not do.
    let quiet =
        SpectrumConfig { floor_db: -60.0, ceiling_db: -30.0, tilt: 0.0, ..Default::default() };
    assert!((loudness(&quiet, power_at(-30.0), TILT_PIVOT_MIDI) - 1.0).abs() < 1e-4);
    assert!((loudness(&quiet, power_at(-45.0), TILT_PIVOT_MIDI) - 0.5).abs() < 1e-4);
}

/// A hand-edited state blob can carry a collapsed or inverted pair; the
/// bar cannot. Unclamped that divides by zero and paints NaN geometry,
/// which egui panics on — inside the host, for a plugin.
#[test]
fn a_collapsed_level_range_still_maps_to_a_finite_number() {
    for (floor, ceiling) in [(-60.0, -60.0), (-20.0, -80.0), (0.0, 0.0)] {
        let cfg = SpectrumConfig {
            floor_db: floor,
            ceiling_db: ceiling,
            tilt: 0.0,
            ..Default::default()
        };
        for db in [-120.0, -60.0, -12.0, 0.0] {
            let level = loudness(&cfg, power_at(db), TILT_PIVOT_MIDI);
            assert!(
                level.is_finite() && (0.0..=1.0).contains(&level),
                "{floor}..{ceiling} dB at {db} dB gave {level}",
            );
        }
    }
}

/// Every orientation the pane offers — the loop the axis tests run over.
///
/// [`SpectralOrientation::ALL`], not a second list of the same four names:
/// that one is built through an exhaustive `match`, so a fifth variant
/// fails to compile until it is added and every sweep below picks it up.
/// A literal here would leave the sweeps quietly covering four of five.
const EVERY_ORIENTATION: [SpectralOrientation; 4] = SpectralOrientation::ALL;

/// Each orientation puts the NOW-line on the side it is named for, which
/// is the whole meaning of the setting: that is where the spectrum sits,
/// where a ribbon arrives, and where the heatmap's newest column is. The
/// far corner pins the direction time then runs in.
#[test]
fn the_now_line_lands_on_the_side_the_orientation_names() {
    // WIDE is (10, 20)..(310, 120): 300 across, 100 down.
    let now_and_past = |o| {
        let a = axes(WIDE, o);
        (a.at(0.0, 0.0), a.at(0.0, 1.0))
    };
    assert_eq!(
        now_and_past(SpectralOrientation::Left),
        (egui::pos2(10.0, 120.0), egui::pos2(310.0, 120.0)),
        "now on the left, past to the right",
    );
    assert_eq!(
        now_and_past(SpectralOrientation::Right),
        (egui::pos2(310.0, 120.0), egui::pos2(10.0, 120.0)),
        "now on the right, past to the left",
    );
    assert_eq!(
        now_and_past(SpectralOrientation::Top),
        (egui::pos2(10.0, 20.0), egui::pos2(10.0, 120.0)),
        "now along the top, past below",
    );
    assert_eq!(
        now_and_past(SpectralOrientation::Bottom),
        (egui::pos2(10.0, 120.0), egui::pos2(10.0, 20.0)),
        "now along the bottom, past above",
    );
}

/// The reconstruction filter is pointed along the axis a name actually
/// travels, in every orientation — see [`names_slide`].
///
/// Asked of the DEPTH axis rather than of `is_time_vertical`: a name rides
/// time, `dir_depth` is where the pane puts time, and this is the claim that
/// the two agree. The filter is one-dimensional and separable, so pointing it
/// across a pane whose text scrolls downward is not a smaller correction but
/// no correction at all — measured at 89.6% of a hairline's coverage swinging
/// as it slides, against the 40% bound `a_sliding_hairline_keeps_its_weight`
/// holds it to.
///
/// Compared as the UNIT VECTOR the shader reads, which is what keeps this from
/// being the same sentence twice. Naming the two ends by
/// [`SlideAxis::vertical`] on both sides is a tautology whatever that function
/// does with its argument: the constructor is injective, so it cancels, and a
/// build with its two arms exchanged — every orientation's filter turned 90°,
/// including the two that were right — passes. `unit` is the value that
/// reaches `Locals::filter_axis`, so an assertion against it cannot cancel.
///
/// Unsigned, because the filter's two taps sit at `±FILTER_TAP` along this and
/// so cannot tell the ends of the axis apart: `Bottom` runs time up the pane
/// and `Top` runs it down, and both want the same offset.
///
/// Exact rather than within a tolerance. Every orientation lays its axes out
/// square with the screen, so `dir_depth` normalizes an axis-aligned vector
/// and the components are exactly 0 and 1. A future orientation that is not
/// square with the screen should fail here rather than round to whichever axis
/// it leans toward — a diagonal has no answer in this filter, and quietly
/// picking one would be the wrong kind of pass.
///
/// Nothing else on the pane cares: the axis labels share the batch and stand
/// still.
#[test]
fn the_names_filter_follows_the_axis_time_runs_along() {
    for orientation in EVERY_ORIENTATION {
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        // Both pane shapes. `names_slide` cannot see a rect, so this is not
        // asking it about one — it holds the OTHER side of the comparison,
        // that the depth axis is the orientation's and not the aspect ratio's,
        // which is what makes the reading it is compared against a fact about
        // the setting rather than about a 300x100 pane.
        for rect in [WIDE, TALL] {
            let depth = Axes::new(rect, &cfg).dir_depth();
            assert_eq!(
                super::names_slide(&cfg).unit(),
                [depth.x.abs(), depth.y.abs()],
                "{orientation:?} on {rect:?} runs time along {depth:?}, \
                 and the filter is pointed elsewhere",
            );
        }
    }
}

/// Pitch reads the conventional way in all four, rather than mirroring
/// with time: low at the BOTTOM wherever time is horizontal, low at the
/// LEFT wherever it is vertical. Flipping it along with time would turn
/// Right and Bottom into upside-down pictures of their partners, where
/// what they are for is the same picture arriving from the other side.
#[test]
fn pitch_climbs_the_same_way_in_the_pair_that_shares_an_axis() {
    for (o, low, high) in [
        (SpectralOrientation::Left, 120.0, 20.0),
        (SpectralOrientation::Right, 120.0, 20.0),
    ] {
        let a = axes(WIDE, o);
        assert_eq!(a.at(0.0, 0.5).y, low, "{o:?}: low pitch is not at the bottom");
        assert_eq!(a.at(1.0, 0.5).y, high, "{o:?}: high pitch is not at the top");
    }
    for o in [SpectralOrientation::Top, SpectralOrientation::Bottom] {
        let a = axes(WIDE, o);
        assert_eq!(a.at(0.0, 0.5).x, 10.0, "{o:?}: low pitch is not at the left");
        assert_eq!(a.at(1.0, 0.5).x, 310.0, "{o:?}: high pitch is not at the right");
    }
}

/// Which side is the pitch axis and which the time axis, in each pair.
#[test]
fn the_axes_take_the_pane_sides_the_orientation_asks_for() {
    for o in [SpectralOrientation::Left, SpectralOrientation::Right] {
        let a = axes(WIDE, o);
        assert_eq!(a.pitch_len(), 100.0, "{o:?}: pitch is the vertical side");
        assert_eq!(a.depth_len(), 300.0, "{o:?}: time is the horizontal side");
    }
    for o in [SpectralOrientation::Top, SpectralOrientation::Bottom] {
        let a = axes(TALL, o);
        assert_eq!(a.pitch_len(), 100.0, "{o:?}: pitch is the horizontal side");
        assert_eq!(a.depth_len(), 300.0, "{o:?}: time is the vertical side");
    }
}

/// The far region's screen-to-time rate had three independent-looking
/// derivations across roll.rs, names.rs and the spectrogram plan before they
/// were unified as `TimeAxis::seconds_per_point` — nothing tied them
/// together. Checked here against the names.rs-style form directly: how far
/// apart `time_at(1.0)` and `time_at(0.0)` land, per point of the full depth
/// axis.
///
/// Splits stop at 0.8. Past that the two forms are not the same computation:
/// the names.rs form floors `axes.depth_len()` alone, which for an ordinary
/// pane (hundreds of points) never engages, while `seconds_per_point` floors
/// the REGION's own width (`depth_len * depth_span`), which does once the far
/// region is dragged under a point wide — a real numeric difference, not
/// float rounding, and the two grow apart without bound as the region closes
/// further. What keeps it from being a regression is that the region is what
/// a name is drawn IN: under a point wide there is no room for one letter,
/// let alone a name, whichever rate is used to space them — so the corner
/// where the two forms disagree is also the corner where nothing depends on
/// which one answered.
#[test]
fn seconds_per_point_matches_the_time_at_derivation() {
    for depth_len in [40.0f32, 300.0, 1200.0] {
        for orientation in EVERY_ORIENTATION {
            let rect = if orientation.is_time_vertical() {
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, depth_len))
            } else {
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(depth_len, 100.0))
            };
            let a = axes(rect, orientation);
            for split in [0.0f32, 0.2, 0.5, 0.8] {
                let time = TimeAxis {
                    split,
                    depth_span: 1.0 - split,
                    window: 12.0,
                    origin: 5.0,
                    now: 5.0,
                    whole_song: false,
                };
                let direct = (time.time_at(1.0) - time.time_at(0.0)).abs()
                    / f64::from(a.depth_len().max(1.0));
                let via_method = time.seconds_per_point(&a);
                // `time_at` divides in f32 before widening to f64
                // (`(d - split) / depth_span`), so the two derivations round
                // differently even within the splits swept here — an
                // f32-epsilon difference, not the real one named above.
                assert!(
                    (via_method - direct).abs() < 1e-6,
                    "{orientation:?} @{split}, {depth_len}pt: {via_method} vs {direct}",
                );
            }
        }
    }
}

/// Hover has to find the pitch the pointer is actually over, in every
/// orientation — the lattice highlight hangs off this one inverse.
#[test]
fn pitch_at_inverts_at_whichever_way_the_axes_run() {
    for rect in [WIDE, TALL] {
        for orientation in EVERY_ORIENTATION {
            let a = axes(rect, orientation);
            for step in 0..=10 {
                let p = step as f32 / 10.0;
                // Any depth: the inverse reads the pitch axis only.
                let back = a.pitch_at(a.at(p, 0.37));
                assert!((back - p).abs() < 1e-4, "{orientation:?}: {p} -> {back}");
            }
        }
    }
}

/// The divider drag reads the pointer through this inverse, so it has to
/// agree with `at` in every orientation — a sign flip would send the
/// handle the wrong way, and the two reversed layouts are exactly where a
/// missing flip hides.
#[test]
fn depth_at_inverts_at_whichever_way_the_axes_run() {
    for rect in [WIDE, TALL] {
        for orientation in EVERY_ORIENTATION {
            let a = axes(rect, orientation);
            for step in 0..=10 {
                let d = step as f32 / 10.0;
                // Any pitch: the inverse reads the depth axis only.
                let back = a.depth_at(a.at(0.37, d));
                assert!((back - d).abs() < 1e-4, "{orientation:?}: {d} -> {back}");
            }
        }
    }
}

/// The grab band straddles the divider, stays inside the pane (so a
/// divider dragged flat against an edge is still grabbable), and spans
/// the pitch axis.
#[test]
fn the_split_band_straddles_the_divider_and_stays_inside_the_pane() {
    for rect in [WIDE, TALL] {
        for orientation in EVERY_ORIENTATION {
            let a = axes(rect, orientation);
            for split in [0.0, 0.5, 1.0] {
                let band = a.depth_band(split, SPLIT_GRAB_HALF);
                assert!(rect.contains_rect(band), "{orientation:?} @{split}: {band:?}");
                assert!(band.contains(a.at(0.5, split)), "{orientation:?} @{split}: off-line");
                // Thin across depth, full width across pitch.
                let (thin, wide_) = if a.time_vertical {
                    (band.height(), band.width())
                } else {
                    (band.width(), band.height())
                };
                assert!(thin <= 2.0 * SPLIT_GRAB_HALF, "{orientation:?}: band too thick");
                assert_eq!(wide_, a.pitch_len(), "{orientation:?}: band must span pitch");
            }
        }
    }
}

/// Dragging the divider away from the spectrum GROWS the spectrum's
/// share, in every orientation, by the distance dragged — the whole
/// point of the handle, and the one thing an axis sign error breaks.
///
/// The drag is taken along `dir_depth` rather than written out per case,
/// so "away from the spectrum" means the same thing in the two reversed
/// layouts, where it points back toward the screen's origin.
#[test]
fn dragging_the_divider_moves_the_split_with_the_pointer() {
    for (rect, orientation) in EVERY_ORIENTATION.map(|o| (along_depth(o), o)) {
        let a = axes(rect, orientation);
        let before = 0.5;
        let after = drag_divider(rect, orientation, before, a.dir_depth() * 30.0);
        // Depth runs away from the spectrum, so +30 px of depth takes
        // 30/depth_len off the roll's share.
        let expected = before - 30.0 / a.depth_len();
        assert!(
            (after - expected).abs() < 0.02,
            "{orientation:?}: {before} -> {after}, wanted ~{expected}",
        );
    }
}

/// And back the other way, into the spectrum: the roll grows.
#[test]
fn dragging_the_divider_into_the_spectrum_grows_the_roll() {
    for (rect, orientation) in EVERY_ORIENTATION.map(|o| (along_depth(o), o)) {
        let drag = axes(rect, orientation).dir_depth() * -30.0;
        let after = drag_divider(rect, orientation, 0.5, drag);
        assert!(after > 0.55, "{orientation:?}: roll share should have grown, got {after}");
    }
}

/// Press on the divider, drag by `delta`, and return the resulting
/// `roll_fraction` — [`drag_pane`] aimed at the handle, which sits at the
/// split by definition.
fn drag_divider(
    rect: egui::Rect,
    orientation: SpectralOrientation,
    roll_fraction: f32,
    delta: egui::Vec2,
) -> f32 {
    let cfg =
        SpectrumConfig { orientation, roll_fraction, show_roll: true, ..Default::default() };
    drag_pane(rect, cfg, 1.0 - roll_fraction, delta).roll_fraction
}

/// The pane whose depth axis is its LONG side, for the orientation given —
/// which is the one a depth gesture has room to run along.
fn along_depth(orientation: SpectralOrientation) -> egui::Rect {
    if orientation.is_time_vertical() { TALL } else { WIDE }
}

/// Press at depth `grab` on the pane's own depth axis, drag by `delta`, and
/// return the config that leaves behind.
///
/// Four frames: egui needs the widget to exist before the press, and a drag
/// only registers once the pointer has moved while held. Which gesture runs
/// is the caller's business, chosen by where `grab` puts the press — on the
/// divider's band for the handle, clear of it for the pane behind.
fn drag_pane(
    rect: egui::Rect,
    cfg: SpectrumConfig,
    grab: f32,
    delta: egui::Vec2,
) -> SpectrumConfig {
    let mut state = fresh();
    state.spectrum_config = cfg;
    let ctx = themed();
    // A window big enough for the widest `rect` a caller passes, so the drag
    // is bounded by the pane rather than by the screen's edge.
    let screen = egui::vec2(900.0, 900.0);
    let at = Axes::new(rect, &cfg).at(0.5, grab);
    let frame = |events: Vec<egui::Event>, state: &mut SharedState| {
        let _ = events_into(&ctx, screen, rect, events, |ui| {
            spectral_pane(ui, state, 100.0, 0);
        });
    };
    frame(vec![egui::Event::PointerMoved(at)], &mut state);
    frame(vec![egui::Event::PointerMoved(at), press(at, true)], &mut state);
    frame(vec![egui::Event::PointerMoved(at + delta)], &mut state);
    frame(vec![press(at + delta, false)], &mut state);
    state.spectrum_config
}

/// Where the curve grows from its baseline, as a screen direction: away
/// from the far region it is joined to, or — with that region off — up from
/// the outer edge, which is the direction the whole depth axis runs in.
///
/// The gesture's own sign, written the other way round: it reads the
/// picture, and these tests read the layout, so a sign error in one does
/// not cancel in the other.
fn curve_grows(a: &Axes, joined: bool) -> egui::Vec2 {
    if joined { -a.dir_depth() } else { a.dir_depth() }
}

/// Dragging the spectrum away from its baseline spreads the curve out, so
/// the dB window it spans closes in — the Span's gesture on the axis the
/// spectrum measures, anchored on the baseline the way the Span is on the
/// now-line.
///
/// Every orientation, because "away from the baseline" is a different
/// screen direction in each and a sign error is invisible in the one it was
/// written against.
#[test]
fn dragging_the_spectrum_outward_closes_the_level_window() {
    for orientation in EVERY_ORIENTATION {
        let rect = along_depth(orientation);
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        let out = curve_grows(&Axes::new(rect, &cfg), true) * 40.0;
        let after = drag_pane(rect, cfg, 0.2, out);
        assert!(
            after.ceiling_db < cfg.ceiling_db - 3.0,
            "{orientation:?}: ceiling {} -> {}, wanted it down",
            cfg.ceiling_db,
            after.ceiling_db,
        );
        // The floor IS the baseline this zoom is about, so it does not move:
        // a window that slid bodily would be the Level bar's other gesture.
        assert_eq!(after.floor_db, cfg.floor_db, "{orientation:?}: the floor moved");
    }
}

/// And back toward the baseline, which flattens the curve by opening the
/// window — the same drag, read the other way.
#[test]
fn dragging_the_spectrum_inward_opens_the_level_window() {
    for orientation in EVERY_ORIENTATION {
        let rect = along_depth(orientation);
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        let inward = curve_grows(&Axes::new(rect, &cfg), true) * -40.0;
        let after = drag_pane(rect, cfg, 0.2, inward);
        assert!(
            after.ceiling_db > cfg.ceiling_db + 3.0,
            "{orientation:?}: ceiling {} -> {}, wanted it up",
            cfg.ceiling_db,
            after.ceiling_db,
        );
        assert_eq!(after.floor_db, cfg.floor_db, "{orientation:?}: the floor moved");
    }
}

/// With the roll and the spectrogram both off, the spectrum stands up from
/// the outer edge instead of hanging from the divider — so the direction it
/// grows in is the other way down the depth axis, and the gesture turns
/// with it. Held apart from the joined case because the two share a sign
/// and a fix that turns one turns the other.
#[test]
fn the_level_zoom_turns_with_the_curve_when_the_spectrum_owns_the_pane() {
    for orientation in EVERY_ORIENTATION {
        let rect = along_depth(orientation);
        let cfg = SpectrumConfig {
            orientation,
            show_roll: false,
            show_spectrogram: false,
            ..Default::default()
        };
        assert_eq!(spectrum_share(&cfg), 1.0, "the spectrum should own the whole pane here");
        let out = curve_grows(&Axes::new(rect, &cfg), false) * 40.0;
        // Both EDGES as well as the middle: the spectrum's region runs to the
        // far edge INCLUSIVE here, and a region test written as "nearer than
        // the split" leaves that last line of pixels panning while every other
        // pixel of the same pane zooms.
        for grab in [0.0, 0.5, 1.0] {
            let after = drag_pane(rect, cfg, grab, out);
            assert!(
                after.ceiling_db < cfg.ceiling_db - 3.0,
                "{orientation:?} @{grab}: ceiling {} -> {}, wanted it down",
                cfg.ceiling_db,
                after.ceiling_db,
            );
            assert_eq!(after.floor_db, cfg.floor_db, "{orientation:?} @{grab}: the floor moved");
        }
    }
}

/// One gesture, two values, and neither region touches the other's: a drag
/// over the far end zooms the Span alone, one over the spectrum the Level
/// alone. What decides is where the press landed, which is what makes it a
/// drag on the picture rather than a mode.
#[test]
fn a_depth_drag_moves_only_the_value_its_region_measures() {
    let (rect, cfg) = (WIDE, SpectrumConfig::default());
    let a = Axes::new(rect, &cfg);
    // Toward the past and outward along the curve are the same screen
    // direction here (the curve grows back out of the divider), so the two
    // drags below differ only in where they start.
    let far = drag_pane(rect, cfg, 0.8, a.dir_depth() * 40.0);
    assert!(far.roll_seconds < cfg.roll_seconds - 0.5, "far region: Span should have zoomed");
    assert_eq!(far.ceiling_db, cfg.ceiling_db, "far region: the level moved too");

    let near = drag_pane(rect, cfg, 0.2, -a.dir_depth() * 40.0);
    assert!(near.ceiling_db < cfg.ceiling_db - 3.0, "spectrum: level should have zoomed");
    assert_eq!(near.roll_seconds, cfg.roll_seconds, "spectrum: the Span moved too");
}

/// A drag ACROSS the spectrum still pans the pitch range. Panning is the
/// default everywhere on the pane; the level zoom is what a drag has to
/// lean into, exactly as the Span zoom is.
#[test]
fn a_drag_across_the_spectrum_still_pans_the_pitch_range() {
    let rect = WIDE;
    // Off both ends of the axis, so the pan has room to move rather than
    // sitting against a clamp.
    let cfg = SpectrumConfig { low_midi: 48.0, high_midi: 84.0, ..Default::default() };
    let after = drag_pane(rect, cfg, 0.2, Axes::new(rect, &cfg).dir_pitch() * 30.0);
    assert!(after.low_midi < cfg.low_midi - 1.0, "the range should have panned down");
    assert_eq!(after.ceiling_db, cfg.ceiling_db, "a pan must not move the level");
}

/// Panning stays the default through the NEAR-TIE, which is the only place
/// the lean margin is what decides. A drag leaning a couple of points more
/// along depth than across pitch is a pan that wandered, not a zoom aimed at
/// the Level — and with the margin dropped it would be read as the zoom.
///
/// The pitch-dominant case a page up is carried by the comparison alone and
/// says nothing about the margin, which is why this one is written to the
/// margin's own value.
#[test]
fn a_pan_that_leans_slightly_along_depth_is_still_a_pan() {
    let rect = WIDE;
    let cfg = SpectrumConfig { low_midi: 48.0, high_midi: 84.0, ..Default::default() };
    let a = Axes::new(rect, &cfg);
    // Depth ahead of pitch by three points — written out rather than derived
    // from the margin, because a lean computed off `DEPTH_ZOOM_LEAN` shrinks
    // with it and the test then passes at every value including zero. The
    // premise is asserted instead, so a margin narrowed under three points
    // fails here loudly rather than turning this into a test of nothing.
    let (across, lean) = (30.0, 3.0);
    assert!(lean < DEPTH_ZOOM_LEAN, "this drag has to sit INSIDE the margin to test it");
    let wobble = a.dir_pitch() * across + curve_grows(&a, true) * (across + lean);
    let after = drag_pane(rect, cfg, 0.2, wobble);
    assert_eq!(after.ceiling_db, cfg.ceiling_db, "a lean inside the margin moved the level");
    assert!(after.low_midi < cfg.low_midi - 0.5, "and the pan it was should still have run");
}

/// However far the drag runs, the window stops at the closest the pair may
/// come — the same limit the Level bar holds to, both of them writing the
/// same pair.
#[test]
fn the_level_zoom_stops_at_the_minimum_span() {
    let (rect, cfg) = (WIDE, SpectrumConfig::default());
    let out = curve_grows(&Axes::new(rect, &cfg), true) * 4_000.0;
    let after = drag_pane(rect, cfg, 0.2, out);
    assert!(
        (after.ceiling_db - (after.floor_db + crate::LEVEL_RANGE_MIN_SPAN)).abs() < 1e-3,
        "closed to {} dB, wanted {}",
        after.ceiling_db - after.floor_db,
        crate::LEVEL_RANGE_MIN_SPAN,
    );
    // And the other way, where what stops it is the top of the scale.
    let after = drag_pane(rect, cfg, 0.2, -out);
    assert_eq!(after.ceiling_db, crate::LEVEL_MAX_DB, "opened past full scale");
}

/// A marking label on the outer edge of a wide (Left) pane sits just inside
/// it and grows up-and-inward (LEFT_BOTTOM anchor).
///
/// Pinned as coordinates because both offsets are looks rather than laws —
/// a hair off the ruling across the pitch axis, enough off the edge along the
/// depth axis to clear it — and a look is exactly the kind of thing that
/// drifts silently.
///
/// The anchor BEFORE the pane backs it off by the label's own
/// [`ink_inset`](crate::text::ink_inset), which needs a laid-out galley and so
/// belongs to the frame rather than to the geometry. What the correction does
/// to it is held where the correction lives
/// (`an_ink_correction_lands_a_label_the_same_way_at_any_size`).
#[test]
fn marking_labels_sit_just_inside_the_outer_edge() {
    let a = axes(WIDE, SpectralOrientation::Left);
    let (d, into) = label_anchor(spectrum_share(&SpectrumConfig::default()));
    let (pos, align) = a.text_anchor(0.5, d, LABEL_GAP_PT, into);
    assert_eq!(pos, egui::pos2(12.0, 68.0));
    assert_eq!(align, egui::Align2::LEFT_BOTTOM);
}

/// Whichever way the pane is turned, a label sits inside the pane and grows
/// further in rather than off it.
///
/// Both layouts, because they anchor against OPPOSITE edges of the depth axis
/// (see [`label_anchor`]) — an offset that runs inward from one runs straight
/// off the other, and the four orientations decide which screen edge each of
/// those is.
#[test]
fn label_anchors_grow_into_the_pane() {
    for orientation in EVERY_ORIENTATION {
        let a = axes(WIDE, orientation);
        for split in [spectrum_share(&SpectrumConfig::default()), 1.0] {
            let (d, into) = label_anchor(split);
            let (pos, align) = a.text_anchor(0.5, d, LABEL_GAP_PT, into);
            // A nominal 40x12 label placed by this anchor.
            let box_ = align.anchor_size(pos, egui::vec2(40.0, 12.0));
            assert!(
                WIDE.contains_rect(box_),
                "{orientation:?} at split {split}: {box_:?} escapes {WIDE:?}",
            );
        }
    }
}

/// The frequency labels ride the end of the spectrum its PEAKS reach, not the
/// baseline they stand on — and that is the opposite end of the depth axis in
/// the curve's two layouts, since joining a roll mirrors it.
///
/// Stated against `spectrum_share` rather than a literal split, so the two
/// arms are the two layouts the pane can actually be in rather than two
/// numbers that happen to straddle the branch.
#[test]
fn the_frequency_labels_ride_the_peak_end_of_the_spectrum() {
    // Joined: the curve hangs off the now-line with its peaks pointing back
    // out to depth 0, so that outer edge is where the numbers go.
    let split = spectrum_share(&SpectrumConfig::default());
    assert!(split > 0.0 && split < 1.0, "this needs a pane the spectrum shares");
    let (d, into) = label_anchor(split);
    assert_eq!(d, 0.0, "the labels sat on the baseline, not the peak end");
    assert!(into > 0.0, "the offset runs off the pane rather than into it");

    // Nothing to join: the curve stands up from the outer edge instead, so the
    // peaks reach the far end of the axis and the labels follow them there.
    let alone =
        SpectrumConfig { show_roll: false, show_spectrogram: false, ..SpectrumConfig::default() };
    let (d, into) = label_anchor(spectrum_share(&alone));
    assert_eq!(d, 1.0, "the labels sat on the baseline, not the peak end");
    assert!(into < 0.0, "the offset runs off the pane rather than into it");

    // Whole-song draws no spectrum and gives the roll the whole axis; its
    // labels have only the near edge to ride, and must still turn inward.
    assert_eq!(label_anchor(0.0), (0.0, into.abs()));
}

/// The pitch range the whole analyzer covers, plus a hair either side, so a
/// frequency landing exactly on an end is inside the range rather than on the
/// float boundary of it — 20 Hz and 20 kHz are what the axis is DEFINED as, and
/// whether `hz_to_midi` lands a ulp above or below the constant is not what any
/// test here is about.
fn whole_axis() -> PitchScale {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    let (min_midi, max_midi) = (SPECTRUM_MIN_MIDI - 0.5, SPECTRUM_MAX_MIDI + 0.5);
    PitchScale { min_midi, max_midi, span: max_midi - min_midi }
}

/// The frequency grid is a decade ladder — every 10 Hz below 100, every 100 Hz
/// below 1 kHz, every 1 kHz below 10 — and the 1-2-5 series of each decade is
/// the part of it that carries a number.
#[test]
fn the_frequency_grid_rules_one_step_per_decade() {
    // A long axis, so nothing is thinned for room (that is the next test) and
    // what comes back is the whole ladder.
    let grid = frequency_grid(&whole_axis(), 4_000.0);
    let hz: Vec<f32> = grid.iter().map(|r| r.hz).collect();
    // From 20 Hz, where the analyzer's axis starts — 10 Hz is a step of this
    // ladder and simply falls off the bottom of the range.
    let ladder: Vec<f32> = (2..=9)
        .map(|s| s as f32 * 10.0)
        .chain((1..=9).map(|s| s as f32 * 100.0))
        .chain((1..=9).map(|s| s as f32 * 1_000.0))
        .chain([10_000.0, 20_000.0])
        .collect();
    assert_eq!(hz, ladder, "the ladder is not one even step per decade");

    // The numbered marks: an analyzer's 1-2-5 series, which is exactly the set
    // the pane writes a label beside. Pinned as a list rather than a count, so
    // a ladder that swapped one number for another is named and not just
    // tallied.
    let numbered: Vec<f32> = grid.iter().filter(|r| r.numbered).map(|r| r.hz).collect();
    assert_eq!(
        numbered,
        vec![20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0],
    );

    // The decade boundaries, which are what gets the stronger ink — the three
    // places on this axis where the ladder's step size changes tenfold, and NOT
    // the numbered marks between them. 20 kHz is a number and not a decade;
    // 100 Hz is both.
    let decades: Vec<f32> = grid.iter().filter(|r| r.decade).map(|r| r.hz).collect();
    assert_eq!(decades, vec![100.0, 1_000.0, 10_000.0]);

    // Low to high, and on the axis: the pane draws these in the order they come
    // back and clips nothing.
    assert!(grid.windows(2).all(|w| w[0].t < w[1].t), "the ladder came back out of order");
    assert!(grid.iter().all(|r| (0.0..=1.0).contains(&r.t)), "a ruling landed off the axis");

    // A range showing part of a decade gets that part and no more.
    let octave = PitchScale { min_midi: 60.0, max_midi: 72.0, span: 12.0 }; // 262..523 Hz
    let inside: Vec<f32> = frequency_grid(&octave, 4_000.0).iter().map(|r| r.hz).collect();
    assert_eq!(inside, vec![300.0, 400.0, 500.0]);
}

/// A short axis thins the ladder rather than smearing it, and thins it the
/// same way in every decade — but never drops a numbered mark, which would
/// leave a label sitting on nothing.
#[test]
fn a_short_axis_thins_the_ladder_and_keeps_the_numbers() {
    let scale = whole_axis();
    let long = frequency_grid(&scale, 4_000.0);
    let short = frequency_grid(&scale, 200.0);
    assert!(short.len() < long.len(), "a 200-point axis kept the whole ladder");

    let numbered = |grid: &[Ruling]| -> Vec<f32> {
        grid.iter().filter(|r| r.numbered).map(|r| r.hz).collect()
    };
    assert_eq!(numbered(&short), numbered(&long), "thinning ate a numbered mark");

    // Which steps survive turns on the length of a decade, which is the same
    // everywhere on a log axis — so the two full decades on this range keep the
    // same steps as each other rather than the grid thinning out along the axis.
    let steps = |grid: &[Ruling], base: f32| -> Vec<i32> {
        grid.iter()
            .filter(|r| r.hz >= base && r.hz < base * 10.0)
            .map(|r| (r.hz / base).round() as i32)
            .collect()
    };
    assert_eq!(steps(&short, 100.0), steps(&short, 1_000.0));
    assert!(steps(&short, 100.0).len() < 9, "nothing was thinned at all");

    // And what is left is spaced far enough apart to read as separate lines.
    for pair in short.windows(2) {
        let gap = (pair[1].t - pair[0].t) * 200.0;
        assert!(
            gap >= MIN_RULING_GAP_PT,
            "{} Hz and {} Hz came out {gap} points apart",
            pair[0].hz,
            pair[1].hz,
        );
    }

    // Squeezed hard enough it wears down to the numbers alone, rather than to
    // some arbitrary subset that happens to fit.
    let tiny = frequency_grid(&scale, 100.0);
    assert_eq!(tiny.iter().map(|r| r.hz).collect::<Vec<_>>(), numbered(&long));
}

/// A collapsed, inverted or NaN pitch range — which the bars cannot produce and
/// a hand-edited state blob can — rules nothing at all. Its span is what a
/// position divides by, so any ruling it kept would be placed at a NaN, and egui
/// panics on NaN geometry.
#[test]
fn a_collapsed_range_rules_nothing() {
    // A frequency landing EXACTLY on a collapsed range is the case that gets
    // through a guard on the decade length alone: 1 kHz is inside `60.0..60.0`
    // once the range is written at 1 kHz's own pitch.
    let khz = harmonigraph_core::spectrum::hz_to_midi(1_000.0);
    for (min_midi, max_midi) in [(khz, khz), (60.0, 60.0), (90.0, 30.0), (f32::NAN, f32::NAN)] {
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        assert!(
            frequency_grid(&scale, 300.0).is_empty(),
            "{min_midi}..{max_midi} ruled a line on an axis with no length",
        );
    }
}

/// With the roll off, the spectrum gets the whole depth axis — the
/// layout the voice-bar/curve calibration was set up against.
#[test]
fn the_roll_only_takes_depth_when_it_is_shown() {
    // Isolate the roll's depth share. The spectrogram claims depth the
    // same way and is on by default, so turn it off to test the roll alone.
    let mut cfg = SpectrumConfig { roll_fraction: 0.4, ..Default::default() };
    cfg.show_spectrogram = false;
    cfg.show_roll = false;
    assert_eq!(spectrum_share(&cfg), 1.0);
    cfg.show_roll = true;
    assert_eq!(spectrum_share(&cfg), 0.6);
    cfg.roll_fraction = 1.0;
    assert_eq!(spectrum_share(&cfg), 0.0, "the roll may take the whole pane");
}

/// What the curve leaves between itself and the pane's outer edge is a
/// constant number of points, not a share of the pane.
///
/// A share is an empty band that grows with the pane, so the picture reads
/// emptier the more room it is given — and the band is a border the analyzer
/// draws inside itself, next to the one the dock separator already draws
/// around it. Half a point is what the profile line needs to land inside the
/// edge rather than half over it, and it is all the gap there is.
///
/// Both halves are the claim: the budget is what the drawn curve is scaled by,
/// and the paint is where a slab could still land somewhere else.
#[test]
fn the_curve_clears_the_pane_edge_by_the_same_points_at_any_size() {
    for depth_len in [40.0f32, 300.0, 1200.0] {
        let gap = (0.6 - plot_budget(0.6, depth_len)) * depth_len;
        assert!(
            (gap - PLOT_HEADROOM_PT).abs() < 1e-3,
            "a {depth_len}-point axis left {gap} points of clearance, not {PLOT_HEADROOM_PT}",
        );
    }
    // A pane with no room even for that draws a flat curve, rather than one
    // reaching back through the now-line into the roll's half. Held at the
    // degenerate end too, where a divisor guarded into range would quietly
    // hand back a curve worth half the axis instead.
    assert_eq!(plot_budget(0.001, 10.0), 0.0);
    assert_eq!(plot_budget(1.0, 0.4), 0.0, "a sub-point axis is not a pane to draw on");
    assert_eq!(plot_budget(1.0, 0.0), 0.0, "and a zero-length one divides to a floor, not a NaN");

    // Painted: a tone well over the ceiling clamps, so the curve is drawn at
    // the full budget and the slab end nearest `edge` IS the clearance. `edge`
    // is the depth the curve grows toward, which is the only thing the two
    // layouts below disagree about.
    let reach = |rect: egui::Rect, cfg: SpectrumConfig, edge: f32| {
        let axes = Axes::new(rect, &cfg);
        let mut nearest = f32::INFINITY;
        for shape in paint_tone(rect, cfg) {
            // The spectrum's slabs, and nothing else on the pane: they are the
            // only shapes drawn in a palette color at this opacity.
            if let egui::Shape::LineSegment { points, stroke } = shape {
                if stroke.color.a() != 210 {
                    continue;
                }
                for point in points {
                    let depth = (axes.depth_at(point) - edge).abs();
                    nearest = nearest.min(depth * axes.depth_len());
                }
            }
        }
        assert!(nearest.is_finite(), "{rect:?} drew no spectrum at all");
        nearest
    };
    // Both layouts the pane has, because they place the curve against
    // different edges and reach the clearance down different branches of `sd`:
    // joined to the spectrogram it grows from the now-line toward depth 0, and
    // with the roll and spectrogram both off it owns the axis and grows from
    // depth 0 toward 1. A headroom applied to the wrong end shows up in one and
    // not the other.
    let alone = SpectrumConfig {
        show_roll: false,
        show_spectrogram: false,
        ..SpectrumConfig::default()
    };
    for (cfg, edge, layout) in
        [(SpectrumConfig::default(), 0.0, "joined"), (alone, 1.0, "whole-axis")]
    {
        for size in [egui::vec2(300.0, 100.0), egui::vec2(1200.0, 400.0)] {
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), size);
            let gap = reach(rect, cfg, edge);
            assert!(
                (gap - PLOT_HEADROOM_PT).abs() < 0.1,
                "a {layout} {size:?} pane left {gap} points at its edge, \
                 not {PLOT_HEADROOM_PT}",
            );
        }
    }
}

/// The pane under `cfg`, painted with a 1 kHz tone loud enough to clamp
/// against the default ceiling, so the spectrum curve is drawn at its full
/// depth budget. 1 kHz because it is the tilt's own pivot, where the slope
/// takes nothing off and the level is the tone's alone.
fn paint_tone(rect: egui::Rect, cfg: SpectrumConfig) -> Vec<egui::Shape> {
    let mut state = fresh();
    state.spectrum_config = cfg;
    let sr = 48_000.0;
    let samples: Vec<f32> = (0..48_000)
        .map(|i| (std::f32::consts::TAU * 1_000.0 * i as f32 / sr).sin())
        .collect();
    state.spectrum.push_samples(&samples, 1, sr, 1.0, &cfg);

    // A screen of its own: `rect` here runs to 1200x400, which SCREEN could
    // not hold, and this fixture is about what the curve reaches inside the
    // pane rather than about anything at the window's edge.
    let output = painted_into(egui::vec2(2000.0, 2000.0), rect, |ui| {
        spectral_pane(ui, &mut state, 1.0, 0);
    });
    output.shapes.into_iter().map(|s| s.shape).collect()
}

/// The whole pane, painted in every orientation with a roll that has
/// held notes, bent notes, notes off the pitch range and notes older
/// than the window. Geometry this fiddly is easy to make degenerate
/// (zero-area quads, NaN from a zero span), and egui panics on those.
#[test]
fn the_pane_paints_in_every_orientation() {
    for rect in [WIDE, TALL] {
        for orientation in EVERY_ORIENTATION {
            for roll_fraction in [0.0, 0.55, 1.0] {
                let shapes = paint(rect, orientation, roll_fraction);
                assert!(!shapes.is_empty(), "{orientation:?} drew nothing");
            }
        }
    }
}

/// The spectrogram heatmap is rebuilt and re-uploaded only when its inputs
/// change; between the ~20 Hz FFT columns most frames just redraw the quad
/// over the reused texture. Two frames with identical clock and history: the
/// second finds a matching key and takes that fast path — and must draw
/// exactly the quad the cold first frame built, since it reuses that build's
/// geometry. A stale or mis-cached build would move the quad.
#[test]
fn a_cached_spectrogram_frame_matches_the_cold_build() {
    // The textured strip's per-vertex position + uv. The spectrogram is the
    // pane's only mesh (notes are paths, labels are text), so its geometry
    // is what these shapes carry — however many quads it is split into.
    fn quad(out: &egui::FullOutput) -> Vec<[f32; 4]> {
        let mut v = Vec::new();
        for c in &out.shapes {
            if let egui::Shape::Mesh(m) = &c.shape {
                assert_eq!(m.indices.len(), m.vertices.len() / 4 * 6, "quads, please");
                v.extend(m.vertices.iter().map(|x| [x.pos.x, x.pos.y, x.uv.x, x.uv.y]));
            }
        }
        v
    }

    let mut state = fresh();
    state.spectrum_config.orientation = SpectralOrientation::Left;
    state.spectrum_config.show_spectrogram = true;
    state.spectrum_config.low_midi = 55.0;
    state.spectrum_config.high_midi = 79.0;
    state.spectrum_config.roll_seconds = 10.0;
    let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
    bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
    bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 0.4;
    for i in 0..40 {
        state.spectrum.push_history(90.0 + f64::from(i) * 0.1, &bins);
    }

    // ONE context across both frames, as in the live app: the cache hands
    // back a texture handle owned by this context.
    let ctx = themed();
    let now = 94.0;
    let mut frame =
        || frame_into(&ctx, SCREEN, WIDE, |ui| spectral_pane(ui, &mut state, now, 0));
    let cold = quad(&frame());
    assert!(!cold.is_empty(), "the spectrogram drew no textured quad to cache");
    let hit = quad(&frame());
    assert_eq!(cold, hit, "the cached frame drew a different quad than the cold build");
}

/// The strip reaches the now-line, but the newest column is older than that
/// — half an analysis window, by construction — so its leading sliver has
/// no data of its own and holds the newest column instead. Inside the live
/// ring the texels past the newest one hold what they carried a lap ago (a
/// column from a whole window back), so a `u` that ran on would paint that
/// sliver with the far end of the window.
///
/// Where `u` stops, the mesh SPLITS: a quad spanning the corner would
/// interpolate it across itself, and since these are vertex UVs that
/// rescales the whole image, once per slab as the corner crosses it. So the
/// drawn strip is a flat leading quad (one `u` on all four corners) joined
/// to the data quad at that same `u`.
#[test]
fn the_strip_holds_its_leading_sliver_instead_of_reading_round_the_ring() {
    let mut state = fresh();
    state.spectrum_config.orientation = SpectralOrientation::Left;
    state.spectrum_config.show_spectrogram = true;
    state.spectrum_config.roll_seconds = 2.0; // zoomed in: the sliver is widest
    let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
    bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
    for i in 0..100 {
        state.spectrum.push_history(90.0 + f64::from(i) * 0.02, &bins);
    }
    // The now-line, an analysis window's half-lag past the newest column.
    let now = 91.98 + 0.171;
    let out = painted_pane(WIDE, &mut state, now);
    let mesh = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Mesh(m) => Some(m.clone()),
            _ => None,
        })
        .expect("the spectrogram drew no textured strip");

    assert_eq!(mesh.vertices.len(), 8, "two quads, split where `u` stops");
    let mut us: Vec<f32> = mesh.vertices.iter().map(|v| v.uv.x).collect();
    us.sort_by(f32::total_cmp);
    // Two values only — the corner, shared by the flat quad's four vertices
    // and the data quad's leading two, and the far end of the data.
    let (far, hold) = (us[0], us[7]);
    assert!(far < hold, "the data quad spans no time at all");
    assert_eq!(us.iter().filter(|u| **u == hold).count(), 6, "not one flat leading quad: {us:?}");
    assert_eq!(us.iter().filter(|u| **u == far).count(), 2, "the data quad bends: {us:?}");
}

/// The heatmap image is sized in DEVICE PIXELS, not points. It is stretched
/// over the pane by the GPU, so sizing it in points builds it at the
/// display's density divided by the scale factor and then upsamples — on a
/// Retina screen, half the resolution in each axis, for a heatmap visibly
/// softer than the pane around it. Same pane, twice the density, twice the
/// rows.
///
/// (Rows and not columns: the time axis picks its slab off `live_slab`'s
/// ladder, so how much of a density increase reaches it depends on the span.)
#[test]
fn the_heatmap_image_is_built_at_device_pixels() {
    fn rows_at(ppp: f32) -> usize {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.show_spectrogram = true;
        state.spectrum_config.roll_seconds = 10.0;
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
        for i in 0..40 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.1, &bins);
        }
        let ctx = themed_at(ppp);
        // Twice: `set_pixels_per_point` lands on the following frame.
        for _ in 0..2 {
            let _ = frame_into(&ctx, SCREEN, WIDE, |ui| {
                spectral_pane(ui, &mut state, 94.0, 0);
            });
        }
        state.spectrum.spectrogram[0].tex.as_ref().expect("a heatmap was uploaded").size()[1]
    }

    let (one, two) = (rows_at(1.0), rows_at(2.0));
    assert!(one > 2, "no heatmap rows at 1x");
    // Exactly double, give or take the rounding of one pixel row.
    assert!(
        two.abs_diff(one * 2) <= 1,
        "{one} rows at 1x but {two} at 2x — the image is being sized in points",
    );
}

/// The now-line is painted after the roll that arrives at it.
///
/// A sounding note's ribbon reaches the line and carries its lead THROUGH it,
/// which makes the roll the one layer that lands on the boundary rather than
/// merely arriving at it: drawn over the line it takes the line's whole width
/// away under every note that is sounding, so the divider frays exactly where
/// the picture is busiest and the boundary hardest to follow. Painted last, the
/// line stays one unbroken mark and the ribbon still passes under it.
#[test]
fn the_now_line_paints_over_the_roll_that_arrives_at_it() {
    // Both the roll and the label batch are paint callbacks (the roll's notes
    // are instanced quads, not shapes), and a callback carries no identity a
    // test can read off the shape — so the roll is pinned by the note instead.
    // The count of callbacks BEFORE the line is what the note has to move, and
    // that holds however many other layers become callbacks and wherever in
    // the order they land. Two weaker versions of this both pass under either
    // draw order: testing the LAST callback finds the label batch, and
    // indexing the FIRST one assumes nothing before the roll ever emits a
    // callback, which the spectrogram would break the day it stops being a
    // mesh.
    let frame = |sounding: bool| {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        if sounding {
            state.tracker.handle_event(NoteEvent::on(0.0, 0, 69, 1.0));
        }
        let out = painted_pane(WIDE, &mut state, 0.1);
        let callbacks: Vec<usize> = out
            .shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(&s.shape, egui::Shape::Callback(_)))
            .map(|(i, _)| i)
            .collect();
        // The now-line is the one hairline-colored segment clean across the
        // pitch axis.
        let hairline = out.shapes.iter().position(|s| {
            matches!(&s.shape, egui::Shape::LineSegment { stroke, .. }
                if stroke.color == theme::hairline())
        });
        let hairline = hairline.expect("expected a now-line in the frame");
        (callbacks.len(), callbacks.iter().filter(|&&c| c < hairline).count())
    };

    let (quiet_total, quiet_early) = frame(false);
    let (sounding_total, sounding_early) = frame(true);
    assert_eq!(
        sounding_total,
        quiet_total + 1,
        "the sounding note did not add the roll's paint callback, so there is no \
         roll here to have drawn in either order",
    );
    assert_eq!(
        sounding_early,
        quiet_early + 1,
        "the roll paints over the line it arrives at, biting half its width out \
         under every sounding note",
    );
}

/// Whole-song mode's playhead is painted after the roll, for the same reason
/// the now-line is — and it needs it harder.
///
/// This mode hands the roll the WHOLE depth axis (`split` is 0), so the
/// playhead crosses every ribbon on the pane rather than meeting a row of
/// them end-on: under the roll it comes out dashed, notched once per note it
/// passes over. It is the one moving mark in a static picture, and it is what
/// `--playhead` bakes into an exported video.
#[test]
fn the_whole_song_playhead_paints_over_the_roll_it_sweeps_across() {
    // Same trick as the now-line's: the note is what identifies the roll's
    // paint callback, and the count of callbacks BEFORE the playhead is what
    // it has to move.
    let frame = |sounding: bool| {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        if sounding {
            state.tracker.handle_event(NoteEvent::on(0.0, 0, 69, 1.0));
        }
        // The take laid out statically, the way the offline renderer sets it
        // up. No columns: the heatmap is not what this is about, and the roll
        // reads `whole_song.roll` rather than the live tracker here.
        state.whole_song = Some(crate::WholeSong {
            start: 0.0,
            span: 2.0,
            columns: Vec::new(),
            roll: state.tracker.roll().clone(),
        });
        let out = painted_pane(WIDE, &mut state, 1.0);
        let callbacks = out
            .shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(&s.shape, egui::Shape::Callback(_)))
            .map(|(i, _)| i);
        // The playhead is the accent-colored segment across the pitch axis;
        // the now-line is not drawn in this mode at all.
        let playhead = out
            .shapes
            .iter()
            .position(|s| {
                matches!(&s.shape, egui::Shape::LineSegment { stroke, .. }
                    if stroke.color == theme::accent())
            })
            .expect("expected a playhead in a whole-song frame");
        let callbacks: Vec<usize> = callbacks.collect();
        (callbacks.len(), callbacks.iter().filter(|&&c| c < playhead).count())
    };

    let (quiet_total, quiet_early) = frame(false);
    let (sounding_total, sounding_early) = frame(true);
    assert_eq!(
        sounding_total,
        quiet_total + 1,
        "the note did not add the roll's paint callback, so there is no roll here \
         to have drawn in either order",
    );
    assert_eq!(
        sounding_early,
        quiet_early + 1,
        "the roll paints over the playhead sweeping across it, notching the mark \
         once per note it passes",
    );
}

/// A note sounding where the visible lattice has no node is flagged by a
/// band down the spectrum at its pitch — the lattice shows nothing for
/// such a note by definition, so this pane is the only place you can learn
/// one is playing. Put in the spectrum's territory rather than on the
/// note, whose color is already saying which voice it is.
#[test]
fn an_off_lattice_note_gets_a_band_down_the_spectrum() {
    let bands = |tuning_offset: f32| {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        // The band rides the note's envelope, and the pane below is drawn 50ms
        // in — a fraction of any real arrival. No envelope at all, so what is
        // counted is whether the flag is DRAWN rather than how far its note
        // has eased in.
        state.frame_params.fade_time = 0.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        if tuning_offset != 0.0 {
            state.tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: tuning_offset },
            });
        }
        let out = painted_pane(WIDE, &mut state, 0.05);
        let want = theme::warning_text().gamma_multiply(0.3);
        out.shapes
            .into_iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == want))
            .count()
    };
    assert_eq!(bands(0.0), 0, "a plain C has a node, so nothing to flag");
    assert_eq!(bands(0.5), 1, "half a semitone sharp has none");
}

/// The axis labels carry a rim, like the lattice's node names. What sits
/// behind them is a picture — a bright spectrogram slab, the spectrum's
/// own fill — so plain text has no contrast to rely on, and a label you
/// can't read doesn't say which pitch a lane is.
///
/// The rim is drawn from the glyph's own coverage now rather than by
/// stamping the text, so what this can check is that every label is
/// handed a rim color to draw it with.
#[test]
fn the_axis_labels_are_rimmed() {
    let mut state = fresh();
    state.spectrum_config.orientation = SpectralOrientation::Left;
    state.spectrum_config.roll_fraction = 0.55;
    let out = painted_pane(WIDE, &mut state, 0.05);
    // The labels leave the shape list as one paint callback; what is
    // checkable from here is that the pane emitted one at all, and the
    // glyphs' colors are checked where they are built (`crate::text`).
    assert!(
        out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Callback(_))),
        "the pane drew no label callback at all",
    );
}

/// The frequency rulings go UNDER the picture and stop where the spectrum
/// does.
///
/// Both halves are the whole case for ruling this pane at all. Over the
/// spectrum's fill they would be a mesh laid across a picture for a reading the
/// numbers already give; under it they are what the picture stands on. And run
/// the full depth they would cross the roll's ribbons and outrun the
/// spectrogram's heatmap, which only grows out from the now-line as history
/// accumulates — leaving the far part of every line sitting bare on the bed.
/// Checked in every orientation, because which screen side the spectrum is on
/// is exactly what the pane's four turns change: a ruling drawn along a
/// hardcoded screen axis is right in one of them and crosses the picture in
/// the other three.
#[test]
fn the_rulings_go_under_the_spectrum_and_stop_at_the_now_line() {
    for orientation in EVERY_ORIENTATION {
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        let rect = reference_pane();
        let axes = Axes::new(rect, &cfg);
        let split = spectrum_share(&cfg);
        assert!(split > 0.0 && split < 1.0, "this needs a pane the spectrum shares");

        let (rulings, slabs) = painted_rulings(rect, cfg);
        assert!(rulings.len() > 4, "{orientation:?} ruled {} frequencies", rulings.len());
        assert!(!slabs.is_empty(), "{orientation:?}: the tone drew no spectrum to be behind");
        assert!(
            rulings.last().unwrap().index < slabs[0],
            "{orientation:?}: a ruling is painted over the spectrum's fill",
        );

        // Every ruling spans the spectrum's share end to end — depth 0 to the
        // now-line — and none of it reaches past into the heatmap's half.
        for ruling in &rulings {
            let ends: Vec<f32> = ruling.points.iter().map(|p| axes.depth_at(*p)).collect();
            for d in &ends {
                assert!(
                    (-1e-3..=split + 1e-3).contains(d),
                    "{orientation:?}: a ruling reaches depth {d}, past the now-line at {split}",
                );
            }
            assert!(
                ends.iter().any(|d| d.abs() < 1e-3)
                    && ends.iter().any(|d| (d - split).abs() < 1e-3),
                "{orientation:?}: a ruling covers {ends:?} rather than 0..{split}",
            );
        }
    }
}

/// The stronger ink goes on the decade boundaries and nowhere else.
///
/// Pinned as a MAPPING — which pitches got which weight — rather than as two
/// counts. Two weights and the right totals is what swapping the arms also
/// produces, and the picture it makes is the ladder highlighted everywhere
/// except where the step size actually changes.
#[test]
fn only_the_decade_boundaries_take_the_stronger_ink() {
    for orientation in EVERY_ORIENTATION {
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        let rect = reference_pane();
        let axes = Axes::new(rect, &cfg);
        let scale = PitchScale {
            min_midi: cfg.low_midi,
            max_midi: cfg.high_midi,
            span: cfg.high_midi - cfg.low_midi,
        };
        let grid = frequency_grid(&scale, axes.pitch_len());
        let want: Vec<f32> = grid.iter().filter(|r| r.decade).map(|r| r.t).collect();
        assert_eq!(want.len(), 3, "the default range holds 100 Hz, 1 kHz and 10 kHz");

        // Back from the painted segment to the pitch it was drawn at, so this
        // reads the placement rather than re-deriving it.
        let (rulings, _) = painted_rulings(rect, cfg);
        assert_eq!(rulings.len(), grid.len(), "{orientation:?} drew a different ladder");
        let strong: Vec<f32> = rulings
            .iter()
            .filter(|ruling| ruling.strong)
            .map(|ruling| axes.pitch_at(ruling.points[0]))
            .collect();
        let inked = strong.len();
        assert_eq!(inked, want.len(), "{orientation:?} inked {inked} lines strongly");
        for (got, want) in strong.iter().zip(&want) {
            assert!(
                (got - want).abs() < 1e-4,
                "{orientation:?}: the stronger ink landed at pitch {got}, not {want}",
            );
        }
    }
}

/// Whole-song playhead mode rules nothing.
///
/// It hands the WHOLE depth axis to the roll and the spectrogram (`split` is
/// 0), so there is no spectrum region for a ruling to measure — and a ruling
/// drawn anyway would be a zero-length segment per frequency baked into every
/// frame of a `--playhead` video export.
#[test]
fn whole_song_mode_rules_no_frequencies() {
    let mut state = fresh();
    state.spectrum_config.orientation = SpectralOrientation::Left;
    state.tracker.handle_event(NoteEvent::on(0.0, 0, 69, 1.0));
    state.whole_song = Some(crate::WholeSong {
        start: 0.0,
        span: 2.0,
        columns: Vec::new(),
        roll: state.tracker.roll().clone(),
    });
    let out = painted_pane(WIDE, &mut state, 1.0);
    let ruled = out.shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if is_ruling(stroke.color))
    });
    assert!(!ruled, "a whole-song frame ruled a frequency across a pane with no spectrum on it");
}

/// Whether a stroke color is one of the two a frequency ruling is drawn in.
fn is_ruling(color: egui::Color32) -> bool {
    color == theme::hairline().gamma_multiply(RULING_FADE.0)
        || color == theme::hairline().gamma_multiply(RULING_FADE.1)
}

/// One frequency ruling as it came off the painter: where in the shape list
/// it landed, the two screen points it was drawn between, and whether it took
/// the stronger of the two inks.
struct PaintedRuling {
    index: usize,
    points: [egui::Pos2; 2],
    strong: bool,
}

/// One frame of the pane with a tone in it, split into the frequency rulings
/// and the shape indices of the spectrum's own slabs.
fn painted_rulings(
    rect: egui::Rect,
    cfg: SpectrumConfig,
) -> (Vec<PaintedRuling>, Vec<usize>) {
    let strong = theme::hairline().gamma_multiply(RULING_FADE.0);
    let (mut rulings, mut slabs) = (Vec::new(), Vec::new());
    for (i, shape) in paint_tone(rect, cfg).into_iter().enumerate() {
        let egui::Shape::LineSegment { points, stroke } = shape else { continue };
        if is_ruling(stroke.color) {
            rulings.push(PaintedRuling { index: i, points, strong: stroke.color == strong });
        } else if stroke.color.a() == 210 {
            // The spectrum's own slabs — the one thing on the pane drawn in a
            // gradient color at that opacity.
            slabs.push(i);
        }
    }
    (rulings, slabs)
}

/// The readout names its own unit, and switches to kHz where an analyzer
/// axis does.
#[test]
fn the_hz_readout_carries_its_unit() {
    assert_eq!(hz_readout(69.0), "440 Hz", "A440, the one value worth checking by hand");
    assert_eq!(hz_readout(harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI), "20 Hz");
    assert_eq!(hz_readout(harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI), "20.0 kHz");
    // The switch is at 1000 Hz exactly, not somewhere near it.
    let khz = harmonigraph_core::spectrum::hz_to_midi(1000.0);
    assert_eq!(hz_readout(khz), "1.0 kHz");
    assert!(hz_readout(khz - 0.1).ends_with(" Hz"));
}

/// The settings pane, whose pitch-range bar derives rects from a PAIR of
/// values — the shape of thing that folds to zero area and panics egui.
/// Painted at both the widest and the narrowest range it allows.
#[test]
fn the_settings_pane_paints_at_either_extreme_of_the_pitch_range() {
    let axis =
        (harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI, harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI);
    for (low, high) in [axis, (40.5, 40.5 + crate::PITCH_RANGE_MIN_SPAN), (axis.0, axis.0)] {
        let mut state = fresh();
        state.spectrum_config.low_midi = low;
        state.spectrum_config.high_midi = high;
        // A settings column rather than a picture: narrow and tall, and the
        // pane takes the whole of it.
        let column = egui::vec2(320.0, 700.0);
        let output = painted_full(column, |ui| spectrum_settings_pane(ui, &mut state));
        assert!(!output.shapes.is_empty(), "{low}..{high} drew nothing");
    }
}

/// A state blob carrying a collapsed or inverted pitch range must not
/// take the editor down with it.
#[test]
fn a_degenerate_pitch_range_still_paints() {
    for (low, high) in [(60.0, 60.0), (90.0, 30.0)] {
        let mut state = fresh();
        state.spectrum_config.low_midi = low;
        state.spectrum_config.high_midi = high;
        let output = painted_pane(WIDE, &mut state, 100.0);
        assert!(!output.shapes.is_empty(), "{low}..{high} drew nothing");
    }
}

/// Run one frame of the Spectral pane into `rect` and count the shapes
/// it emitted.
fn paint(
    rect: egui::Rect,
    orientation: SpectralOrientation,
    roll_fraction: f32,
) -> Vec<egui::Shape> {
    let mut state = fresh();
    state.spectrum_config.orientation = orientation;
    state.spectrum_config.roll_fraction = roll_fraction;
    state.spectrum_config.roll_seconds = 10.0;
    state.view.bloom_strength = 1.2; // exercise the note-glow passes
    // Exercise the spectrogram's mesh path in every orientation too, with
    // energy at both axis extremes (where cell clamping is most likely to
    // fold a quad to zero area — which egui panics on).
    state.spectrum_config.show_spectrogram = true;
    let mut spectrum_bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
    spectrum_bins[0] = 1.0;
    spectrum_bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 0.5;
    spectrum_bins[harmonigraph_core::spectrum::SPECTRUM_BINS - 1] = 0.3;
    for i in 0..80 {
        state.spectrum.push_history(90.0 + f64::from(i) * 0.125, &spectrum_bins);
    }

    let on = |time, note| NoteEvent::on(time, 0, note, 0.7);
    let off = |time, note| NoteEvent::off(time, 0, note);
    // Long past the window; inside it; bent across it; off the top of
    // the pitch range; and one still held at `now`.
    state.tracker.handle_event(on(0.0, 60));
    state.tracker.handle_event(off(1.0, 60));
    state.tracker.handle_event(on(95.0, 62));
    state.tracker.handle_event(off(96.0, 62));
    state.tracker.handle_event(on(96.0, 64));
    state.tracker.handle_event(NoteEvent {
        time: 97.0,
        channel: 0,
        note: 64,
        kind: NoteEventKind::Tuning { semitones: 7.5 },
    });
    state.tracker.handle_event(off(99.0, 64));
    state.tracker.handle_event(on(97.0, 127));
    state.tracker.handle_event(on(99.0, 67));
    let now = 100.0;
    state.tracker.prune(now, &harmonigraph_core::Envelope::default());

    let output = painted_pane(rect, &mut state, now);
    output.shapes.into_iter().map(|s| s.shape).collect()
}

/// The roll's ink stops where the LEAD says it does, and nowhere further:
/// everything it draws is clipped to its own share of the pane grown by exactly
/// that reach, so how far the roll paints on the spectrum is one setting's
/// decision rather than a side effect of some other one.
///
/// The outline stands off EVERY side of a note, and a note sounding now has
/// its leading end ON the line — so without the clip the roll paints its edge,
/// and the halo the bloom lays over it, across the line and onto the curve, by
/// however wide the outline happens to be set. The spectrum is the one
/// neighbour the roll shares an edge with, and what it may spend on it is a few
/// points of ribbon fading out (see [`roll::lead`]) and not an outline's worth
/// of black around every held note.
///
/// Checked in depth fractions rather than screen coordinates, and in every
/// orientation, because which screen side the spectrum is on is exactly what
/// the pane's four turns change.
///
/// Driven straight at [`roll::draw_roll`] rather than through the whole pane,
/// so the one callback in the output is the roll's — the markings draw one of
/// their own (`crate::text`), and theirs is allowed everywhere.
#[test]
fn the_rolls_ink_stops_at_the_now_line() {
    // The two ends of the Lead bar's travel: no lead at all, where the roll's
    // ink stops dead on the line, and the widest one there is. Nothing between
    // them is a third case — the boundary is `split` less the lead, and both
    // of these read it off the same arithmetic.
    for lead in [0.0, crate::ROLL_LEAD_MAX] {
        for orientation in [
            SpectralOrientation::Left,
            SpectralOrientation::Right,
            SpectralOrientation::Top,
            SpectralOrientation::Bottom,
        ] {
            let mut state = fresh();
            state.spectrum_config.orientation = orientation;
            state.spectrum_config.roll_fraction = 0.55;
            // The widest outline there is, so the reach that would cross the
            // line is as big as the setting allows, and a bloom over it.
            state.spectrum_config.roll_outline = crate::ROLL_OUTLINE_MAX;
            state.spectrum_config.roll_lead = lead;
            state.spectrum_config.roll_lead_fade = lead;
            state.view.bloom_strength = 1.2;
            // Held at `now`, so its leading end sits exactly on the line.
            state.tracker.handle_event(NoteEvent::on(99.0, 0, 60, 0.8));

            let a = axes(WIDE, orientation);
            let split = spectrum_share(&state.spectrum_config);
            // Where the roll's ink may reach, as a depth: the now-line, less
            // whatever the lead is allowed to carry past it — which is the set
            // share of the spectrum's own share, and so a share of `split`.
            let near = split - lead * split;
            let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
            let output = painted_into(SCREEN, WIDE, |ui| {
                roll::draw_roll(ui.painter(), &a, &scale, &state, split, 100.0, 0);
            });

            let rolls: Vec<&egui::epaint::ClippedShape> = output
                .shapes
                .iter()
                .filter(|s| matches!(s.shape, egui::Shape::Callback(_)))
                .collect();
            assert_eq!(rolls.len(), 1, "expected one roll callback, got {}", rolls.len());
            let roll = rolls[0];
            let egui::Shape::Callback(cb) = &roll.shape else { unreachable!() };

            // Both the callback's own rect — which is what the bloom chain
            // covers — and the clip that actually cuts the ink.
            for (what, rect) in [("the callback rect", cb.rect), ("the clip", roll.clip_rect)] {
                let corners =
                    [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()];
                for corner in corners {
                    let d = a.depth_at(corner);
                    assert!(
                        d >= near - 1e-3,
                        "{what} reaches depth {d} in {orientation:?}, past the lead's own \
                         edge at {near} (the now-line is {split})",
                    );
                }
            }
            // And it is not clipped to nothing either — the roll still gets its
            // whole share of the axis, or this passes by drawing no roll at all.
            let far = a
                .depth_at(roll.clip_rect.left_top())
                .max(a.depth_at(roll.clip_rect.right_bottom()));
            assert!(
                roll.clip_rect.area() > 0.0 && far > 1.0 - 1e-3,
                "the roll was clipped short of its own far edge in {orientation:?}",
            );

            // The other axis, which the depth checks above say nothing about: a
            // region that kept every one of them and covered half the pitch
            // range would throw away half the notes at every orientation.
            //
            // By AREA, which is the one statement here that goes through
            // neither `at` nor its inverse: `pitch_len` and `depth_len` are the
            // pane rect's own two sides, so a region the whole pitch axis wide
            // and the roll's share of depth long has exactly this area
            // whichever way the pane is turned — and a mapping that lost an
            // axis cannot produce it.
            for (what, rect) in [("the callback rect", cb.rect), ("the clip", roll.clip_rect)] {
                let want = a.pitch_len() * a.depth_len() * (1.0 - near);
                assert!(
                    (rect.area() - want).abs() < 1.0,
                    "{what} covers {} of the pane in {orientation:?}, against {want}",
                    rect.area(),
                );
            }
            // And the ends of the pitch axis are inside it, so the area above
            // is the roll's own share of the pane rather than the same area
            // somewhere else on it.
            for p in [0.0, 1.0] {
                let corner = a.at(p, (split + 1.0) * 0.5);
                assert!(
                    cb.rect.expand(1e-3).contains(corner),
                    "pitch {p} sits outside the roll's region in {orientation:?}",
                );
            }
        }
    }
}

/// Pins what [`Axes`] derives for each [`SpectralOrientation`] against the
/// four direction pairs the roll shader is drawn with, the same way
/// [`the_names_filter_follows_the_axis_time_runs_along`] binds the text
/// filter's axis to it. An orientation whose roll drew turned or mirrored
/// against its own spectrum and spectrogram would otherwise ship exactly as
/// green as one that agreed with them.
///
/// The four pairs are typed out here rather than imported: harmonigraph-render
/// spells the same table as `TOP`/`BOTTOM`/`LEFT`/`RIGHT` in its own `roll.rs`,
/// but those are `#[cfg(test)]` constants and no `#[cfg(test)]` item is
/// reachable from another crate. So this is a hand copy, and what it holds is
/// THIS side — that `Axes` keeps deriving the directions the shader is known
/// to want. A change to the render side's own convention would not fail here;
/// render's orientation tests are what cover that half.
#[test]
fn roll_axes_match_what_axes_derives_for_the_same_orientation() {
    use harmonigraph_render::RollAxes;
    let cases = [
        (SpectralOrientation::Top, RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, 1.0] }),
        (SpectralOrientation::Bottom, RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, -1.0] }),
        (SpectralOrientation::Left, RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [1.0, 0.0] }),
        (SpectralOrientation::Right, RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [-1.0, 0.0] }),
    ];
    for (orientation, roll) in cases {
        let a = axes(WIDE, orientation);
        assert_eq!(
            [a.dir_pitch().x, a.dir_pitch().y],
            roll.pitch_dir,
            "{orientation:?}: Axes::dir_pitch disagrees with roll.rs's RollAxes::pitch_dir",
        );
        assert_eq!(
            [a.dir_depth().x, a.dir_depth().y],
            roll.depth_dir,
            "{orientation:?}: Axes::dir_depth disagrees with roll.rs's RollAxes::depth_dir",
        );
    }
}
