use super::*;
use crate::panes::spectral::DEPTH_ZOOM_PER_DRAG_POINT;
use egui::Color32;
use harmonigraph_core::spectrum::SPECTRUM_BINS;

/// One column: its date and its raw power per bucket.
type Col = (f64, Box<harmonigraph_core::spectrum::SpectrumBuckets>);

/// A column at `time` with the given (bin, power) energy, rest silent.
fn col(time: f64, energy: &[(usize, f32)]) -> Col {
    let mut power = Box::new([0.0f32; SPECTRUM_BINS]);
    for &(i, p) in energy {
        power[i] = p;
    }
    (time, power)
}

/// The pitch range the scroll sweeps hold fixed while they move time.
const SWEEP_SCALE: PitchScale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };

/// The stored byte a power lands on — what the aggregation compares, and
/// so what the aggregation tests below assert against.
fn q(power: f32) -> BucketDb {
    harmonigraph_core::spectrogram::quantize(power)
}

/// A pyramid fed `columns` in order, as the store feeds it.
fn pyramid_of(columns: impl IntoIterator<Item = Col>) -> SlabPyramid {
    let mut pyramid = SlabPyramid::default();
    for (time, power) in columns {
        pyramid.push(time, &power);
    }
    pyramid
}

/// Level `level` of a pyramid fed `columns` in order: its slabs' centres in
/// absolute time and the slabs, oldest first.
fn level_of(columns: &[Col], level: usize) -> (Vec<f64>, Vec<SpectrogramSlab>) {
    let (first_key, slabs) = pyramid_of(columns.iter().cloned()).level(level);
    let w = width(level);
    let centres = (0..slabs.len()).map(|i| ((first_key + i as i64) as f64 + 0.5) * w).collect();
    (centres, slabs)
}

#[test]
fn a_short_loud_column_contributes_its_linear_power_mean() {
    // A brief loud note between quiet columns: each contributes once,
    // including the quiet measurements. Mean dB would give a different result.
    let cols = [
        col(0.00, &[(5, 0.001)]),
        col(0.02, &[(5, 1.0)]), // the short note
        col(0.04, &[(5, 0.002)]),
    ];
    // Level 6's slabs are 1.024 s wide, which holds all three in its first.
    let (centers, power) = level_of(&cols, 6);
    assert_eq!(centers.len(), 1, "one slab of width 1.024 s holds all three");
    assert_eq!(power[0][5], q((0.001 + 1.0 + 0.002) / 3.0), "mean power, not max or mean dB");
}

/// A stall in the analyzer leaves a hole in the column stream — switching
/// the FFT window empties its ring for a whole window's worth of samples,
/// and nothing is measured until it refills. Every slab of elapsed time
/// still needs its row: the texture's time axis is uniform, so without
/// them the columns either side of the hole become neighbouring texels and
/// the quad stretches that pair across the entire silent stretch — a band
/// of one flat color, exactly as wide as the stall.
#[test]
fn a_gap_in_the_columns_keeps_a_row_for_every_silent_slab() {
    // Two columns 1.1 s apart, in level 4's 0.256 s slabs: the second lands
    // four slabs on, with three of silence between them.
    let cols = [col(0.0, &[(5, 1.0)]), col(1.1, &[(5, 0.5)])];
    let (centers, power) = level_of(&cols, 4);
    assert_eq!(centers.len(), 5, "one row per slab, silent ones included");
    let at = |slab: usize| power[slab][5];
    assert_eq!(at(0), q(1.0), "the column before the gap");
    assert_eq!([at(1), at(2), at(3)], [0, 0, 0], "the gap reads as silence, not as a smear");
    assert_eq!(at(4), q(0.5), "the column after it");
    // Evenly spaced centers are exactly what the texture mapping assumes.
    for pair in centers.windows(2) {
        assert!(
            (pair[1] - pair[0] - width(4)).abs() < 1e-9,
            "slabs must stay uniform: {centers:?}"
        );
    }
}

#[test]
fn a_slab_is_anchored_to_absolute_time_not_ring_position() {
    // The same note must land in the same slab whether or not older columns
    // are present — otherwise scrolling would shift it and it would shimmer.
    // A note at t=2.6 sits in level 6's slab floor(2.6 / 1.024) = 2, centred
    // on 2.56.
    let centre = 2.5 * width(6);
    let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
    let (c_full, _) = level_of(&with_old, 6);
    let just_note = [col(2.6, &[(0, 0.5)])];
    let (c_scrolled, _) = level_of(&just_note, 6);
    assert_eq!(c_full.last(), Some(&centre), "the note's slab with old columns");
    assert_eq!(c_scrolled, [centre], "and the same slab after they scroll off");
}

/// Storing a bucket as a byte of dB is a memory decision, and it is only
/// allowed to be one: the colour a cell ends up must be the colour the
/// power itself would have produced, to within half the grid step it was
/// put on. Anything wider would be a look change wearing an optimization's
/// clothes. (The step itself was judged by eye against a sixteen-bit store
/// and found invisible; this is what keeps it from drifting after.)
#[test]
fn quantizing_a_bucket_does_not_move_its_colour() {
    use crate::panes::spectral::axes::{loudness_db, power_db};
    let mut cfg = SpectrumConfig::default();
    // The curve and heatmap have independent windows in the composed
    // defaults. This fixture makes them equal so the only difference under
    // measurement is the heatmap store's byte quantization.
    cfg.volume_floor_db = cfg.floor_db;
    cfg.volume_ceiling_db = cfg.ceiling_db;
    let tolerance =
        0.5 * harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
    for tilt in [0.0, 3.0, -3.0] {
        cfg.tilt = tilt;
        for midi in [20.0f32, 60.0, 100.0, 130.0] {
            for power in [1e-8f32, 1e-6, 1e-4, 1e-2, 0.1, 0.5, 1.0, 4.0] {
                let exact = loudness_db(&cfg, power_db(power), midi);
                let stored = bin_level_for_test(&cfg, q(power), midi);
                assert!(
                    (stored - exact).abs() <= tolerance,
                    "power {power} at MIDI {midi} (tilt {tilt}): \
                         {exact} exact vs {stored} stored",
                );
            }
        }
    }
    // And silence stays exactly silent rather than creeping up off the
    // quantizer's floor, whatever the dB window is set to.
    cfg.floor_db = -120.0;
    assert_eq!(bin_level_for_test(&cfg, 0, 60.0), 0.0, "an empty bucket must read as silence");
}

/// The quiet end of the ramp must FADE to black, not fall off a cliff into
/// it. A shortcut answering everything under -90 dB as silence is
/// invisible while the dB window bottoms out above that, and becomes a
/// hard edge — faintest colour straight to black — as soon as the window
/// can be dragged below it. Nothing between two adjacent stored bytes may
/// move the level by more than the step between them.
///
/// The sweep runs past [`LEVEL_MIN_DB`](crate::LEVEL_MIN_DB) on purpose: the
/// Level range bar stops at -100, and a hand-edited blob does not.
#[test]
fn the_quiet_end_of_the_ramp_fades_instead_of_cutting_off() {
    let mut cfg = SpectrumConfig { volume_ceiling_db: 0.0, ..SpectrumConfig::default() };
    for floor in [-60.0f32, -90.0, -100.0, -120.0] {
        cfg.volume_floor_db = floor;
        // One stored step, as a fraction of the window it is drawn in; the
        // levels either side of any stored byte may differ by that and no
        // more.
        let step = harmonigraph_core::spectrogram::DB_STEP / (cfg.volume_ceiling_db - floor);
        for bucket in 0..BucketDb::MAX {
            let here = bin_level_for_test(&cfg, bucket, 60.0);
            let next = bin_level_for_test(&cfg, bucket + 1, 60.0);
            assert!(
                next - here <= step * 1.001 && next >= here,
                "floor {floor}: byte {bucket} ({here}) -> {next} jumps by {}, \
                     one step is {step}",
                next - here,
            );
        }
        // And the bottom byte is level 0 at every window, so silence
        // still draws the gradient's floor, the region's bed, rather than
        // glowing.
        assert_eq!(
            bin_level_for_test(&cfg, 0, 60.0),
            0.0,
            "floor {floor}: silence must be level 0"
        );
    }
}

/// Every preset, which is what a fresh install and a Palette press can put
/// on screen — where the field itself accepts any gradient, that being the
/// point of the bars.
#[test]
fn cells_are_opaque_and_run_dark_to_bright() {
    for preset in crate::SpectrogramPreset::ALL {
        let g = preset.gradient();
        let cell_color = crate::panes::spectral::spectrogram::cell_color;
        let (quiet, loud) = (cell_color(g, 0.0), cell_color(g, 1.0));
        // Opaque throughout: a cell's level is its COLOUR, never its alpha,
        // so silence recedes by being dark rather than by being
        // see-through.
        assert_eq!(quiet.a(), 255, "{preset:?}");
        assert_eq!(loud.a(), 255, "{preset:?}");
        // Silence is the dark end; loud is brighter. In `L*`, which is the
        // units the gradient is authored in — a channel sum weights blue
        // like green and would call a violet and a yellow of one sum
        // equally bright.
        let lightness = |c: Color32| {
            let v = |b: u8| f64::from(b) / 255.0;
            harmonigraph_scene::color::lightness_of_encoded(v(c.r()), v(c.g()), v(c.b()))
        };
        assert!(
            lightness(quiet) < 0.5,
            "{preset:?}: silence must sit on the ramp's black end, got {quiet:?}"
        );
        assert!(lightness(loud) > lightness(quiet), "{preset:?}");
    }
}

/// A preset's six numbers ARE the two ends each of its bars reads out.
///
/// [`SpectrogramPreset::gradient`] states each stretch as the pair a reader
/// can name — the `L*` and the chroma share that silence and a full bucket
/// are drawn at — and composes the middle-and-signed-ramp the gradient
/// stores. That composition is arithmetic with no picture of its own, so
/// nothing downstream can catch it being wrong: a `chroma` written as
/// `c_lo` rather than as the midpoint draws every colored preset visibly
/// duller and leaves every other assertion in the crate standing.
///
/// Asserted against the pair the ARM names rather than against a constant
/// restated here — a second copy of the numbers would agree with itself
/// and the arm at once, and stop discriminating the moment either moved.
#[test]
fn a_preset_composes_the_pair_its_arm_names() {
    // The pairs, spelled once, beside the arm that has to produce them.
    let ends = |p: crate::SpectrogramPreset| -> ((f32, f32), (f32, f32)) {
        use crate::SpectrogramPreset::*;
        match p {
            Ice => ((0.0, 92.0), (0.635, 0.985)),
            Aurora => ((0.0, 88.0), (0.518, 0.968)),
            Magma => ((0.0, 90.0), (0.819, 0.969)),
        }
    };
    // A middle and a signed ramp do not recompose to their own ends bit for
    // bit — `Spread::legal` says why, a hundredth being no binary fraction —
    // so the ends are compared to within an ulp or two rather than exactly.
    // Six orders under the smallest difference any real slip makes: writing
    // `c_lo` for the midpoint moves Aurora's low end by 0.11.
    let close = |got: f32, want: f32| (got - want).abs() < 1e-5;
    for preset in crate::SpectrogramPreset::ALL {
        let g = preset.gradient();
        let ((l_lo, l_hi), (c_lo, c_hi)) = ends(preset);
        let half = g.lightness_ramp * 0.5;
        assert!(
            close(g.lightness - half, l_lo) && close(g.lightness + half, l_hi),
            "{preset:?}: the brightness bar would read out \
                 {} -> {}, not {l_lo} -> {l_hi}",
            g.lightness - half,
            g.lightness + half,
        );
        // Off the curve rather than off the fields, so this measures what a
        // cell is actually drawn with: `chroma_at` is what resolves the pair
        // at a level, and the bars read out its ends.
        let (got_lo, got_hi) = (g.chroma_at(0.0) as f32, g.chroma_at(1.0) as f32);
        assert!(
            close(got_lo, c_lo) && close(got_hi, c_hi),
            "{preset:?}: the chroma bar would read out \
                 {got_lo} -> {got_hi}, not {c_lo} -> {c_hi}",
        );
        // And the gradient accepts all six untouched. Sanitize bounds a ramp
        // by what its middle leaves on the axis, so a preset that overran
        // would be silently pulled in and the pane would draw a picture the
        // preset did not ask for — passing every test above, which read the
        // clamped numbers back.
        assert_eq!(g.sanitized(), g, "{preset:?}: sanitize moved a preset");
    }
}

/// The heatmap's ramp reaches the two `L*` ends its brightness bar reads
/// out, which is the whole of what the pane promises about a gradient it did
/// not author: a cell at level 0 or 1 has to be drawn at them.
///
/// Through [`cell_color`](crate::panes::spectral::spectrogram::cell_color) rather
/// than through the curve, so what is measured
/// is the table the texels come off — including its quantization to a byte,
/// which is what the tolerance is for. The numbers it is held to are
/// `sanitized`'s, which is honest only because
/// `a_preset_composes_the_pair_its_arm_names` proves sanitize moves none of
/// them; without that, this would be measuring the clamped picture against
/// the clamped numbers and agreeing with itself.
#[test]
fn a_gradient_draws_its_own_ends() {
    for preset in crate::SpectrogramPreset::ALL {
        let g = preset.gradient().sanitized();
        let half = g.lightness_ramp * 0.5;
        let (lo, hi) = (g.lightness - half, g.lightness + half);
        for (level, want) in [(0.0, lo), (1.0, hi)] {
            let c = crate::panes::spectral::spectrogram::cell_color(g, level);
            let v = |b: u8| f64::from(b) / 255.0;
            let got = harmonigraph_scene::color::lightness_of_encoded(v(c.r()), v(c.g()), v(c.b()));
            assert!(
                (got - f64::from(want)).abs() < 0.6,
                "{preset:?} at level {level}: asked for L* {want}, drew {got:.2} ({c:?})"
            );
        }
    }
}

/// A Clear during playback empties the history, open slabs included. The
/// first column after it lands in the slab the last one before it was folded
/// into, where the open slab's running sum would otherwise carry the old
/// power on: the finest slab is 16 ms and the columns either side of the
/// Clear 4 ms apart.
#[test]
fn a_clear_keeps_the_open_slab_from_averaging_in_what_came_before() {
    let mut spectrum = crate::AudioSpectrum::default();
    let loud = [1.0; SPECTRUM_BINS];
    let mut quiet = [0.0; SPECTRUM_BINS];
    quiet[4] = 0.2;
    for t in [0.0, 0.004, 0.008] {
        spectrum.push_history(t, &loud);
    }
    spectrum.clear_history();
    for t in [0.012, 0.014] {
        spectrum.push_history(t, &quiet);
    }
    let (first_key, slabs) = spectrum.pyramid().level(0);
    assert_eq!((first_key, slabs.len()), (0, 1), "the fixture keeps the slab open across it");
    assert_eq!(slabs[0][4], q(0.2), "the open slab averaged in power from before the Clear");
    assert_eq!(slabs[0][5], 0, "a bucket only the old columns lit");
}

/// The layout arithmetic every frame rides on, decided without a frame.
///
/// This is where the pipeline's cliffs live — a slab against the analyzer's
/// lag, a slab against the column rate, a slab count against the ring a
/// level keeps — and with [`Plan`] split out of the draw it is exercised
/// without an `egui::Context` at all. Each assertion below is a bug this
/// pane has actually had.
#[test]
fn the_plan_decides_the_layout_without_a_frame() {
    let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
    let view = |ppp: f32, pitch_len: f32, depth_len: f32, window: f64| PaneView {
        ppp,
        pitch_len,
        depth_len,
        window,
        scale,
        cfg: SpectrumConfig::default(),
    };
    let plan = |v: &PaneView| Plan::new(v, 0.0, None);

    // Rows are PIXELS, not points: on a 2x screen the picture is read at
    // the density it will be drawn at, rather than upsampled from half of
    // it — and to the pixel, since a row costs a fragment's arithmetic and
    // no memory at all.
    let rows_at = |ppp: f32, pitch: f32| plan(&view(ppp, pitch, 800.0, 12.0)).rows as f32;
    for (ppp, pitch) in [(1.0, 300.0), (2.0, 300.0), (1.0, 517.0), (2.0, 517.0)] {
        assert_eq!(rows_at(ppp, pitch), (pitch * ppp).round(), "rows are the pane's pixels");
    }
    // Twice the density really is twice the picture.
    assert!((rows_at(2.0, 517.0) / rows_at(1.0, 517.0) - 2.0).abs() < 0.01);

    // The curve counts the same axis in the same pixels — see
    // [`pitch_pixels`], and `the_curve_and_the_heatmap_read_a_run_of_buckets_alike`
    // for the operator those pixels are read through.
    for (ppp, pitch) in [(1.0, 300.0), (2.0, 300.0), (1.5, 517.0), (3.0, 91.0)] {
        assert_eq!(
            curve_cols(pitch, ppp) as f32,
            rows_at(ppp, pitch),
            "the curve drew {} columns over a heatmap of {} pixels at {ppp}x",
            curve_cols(pitch, ppp),
            rows_at(ppp, pitch),
        );
    }

    // A slab is never finer than the columns arrive, whatever the pane's
    // width asks for: a shorter one leaves empty slabs between columns, and
    // the uniform time axis then stretches the edge columns into flat
    // streaks. Two columns to a slab, since the grids share the ladder's
    // period but not its phase.
    let floor = crate::spectrogram::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
    let dense = plan(&view(2.0, 300.0, 4000.0, 1.0));
    assert!(dense.bucket() >= floor, "slab {} is finer than the data", dense.bucket());
    // And it is always ON the ladder, so it reads a level the pyramid keeps.
    for window in [1.0f64, 12.0, 30.0, 600.0] {
        let steps = (plan(&view(2.0, 300.0, 4000.0, window)).bucket() / floor).log2();
        assert!((steps - steps.round()).abs() < 1e-9, "a {window} s Span fell off the ladder");
    }

    // And the live image is never cut into more slabs than a level's ring
    // holds.
    for window in [1.0f64, 12.0, 60.0, 600.0] {
        let p = plan(&view(2.0, 300.0, 4000.0, window));
        let slabs = window / p.bucket();
        assert!(
            slabs <= LIVE_SLAB_CAP as f64 + 1.0,
            "a {window} s Span asks for {slabs} slabs, past the {LIVE_SLAB_CAP} cap",
        );
    }
    // A wider pane buys finer slabs, up to the cap — the pane's size is an
    // INPUT to the level, which is why resizing can move it.
    let narrow = plan(&view(1.0, 300.0, 200.0, 30.0));
    let wide = plan(&view(1.0, 300.0, 900.0, 30.0));
    assert!(wide.bucket() < narrow.bucket(), "a wider pane should resolve time more finely");

    // The slab width is the plan's one input to which slabs are read: a
    // density change that moves no slab boundary re-reads the run it already
    // has, while one that crosses a ladder rung does not.
    let at_1x = plan(&view(1.0, 300.0, 800.0, 12.0)).level;
    assert_eq!(at_1x, plan(&view(1.0, 300.0, 800.0, 12.0)).level);
    assert_eq!(
        at_1x,
        plan(&view(1.0, 900.0, 800.0, 12.0)).level,
        "a taller pane reads the same slabs and must not change level",
    );
    assert_ne!(
        at_1x,
        plan(&view(1.0, 300.0, 200.0, 12.0)).level,
        "a quarter of the depth pixels crosses a rung, which is another level",
    );
}

/// Dragging the Span must not re-lay the grid on every frame of the drag.
///
/// This is what [`live_slab`]'s ladder is for. A slab width taken straight
/// from the window moves whenever the window does, and a moved slab width
/// reads another level — so a drag would pay a full upload, every frame, for
/// as long as it lasted. On the ladder the width holds across a whole rung.
///
/// Swept in both directions, a frame of drag at a time.
#[test]
fn dragging_the_span_holds_the_grid_between_ladder_steps() {
    let cols = LIVE_SLAB_CAP as usize;
    // One rung, end to end: at the 1024-slab cap a width holds while the
    // Span runs from 512 of them to 1024 of them, which here is 16.4 s to
    // 32.8 s. The sweep stops just inside both ends, since crossing a rung
    // is a real re-layout and not what this is about.
    let rung = live_slab(30.0, cols);
    let (lo, hi) = (rung * 520.0, rung * 1010.0);
    let widths: std::collections::BTreeSet<i64> = (0..=1200)
        .map(|frame| {
            let phase = f64::from(frame) / 600.0;
            let span =
                if phase <= 1.0 { hi - (hi - lo) * phase } else { lo + (hi - lo) * (phase - 1.0) };
            // Microseconds, to compare exactly.
            (live_slab(span, cols) * 1e6).round() as i64
        })
        .collect();
    assert_eq!(
        widths.len(),
        1,
        "the slab width took {} values inside one rung ({:?} us) — every one of them \
             reads another level and re-uploads the texture",
        widths.len(),
        (widths.first(), widths.last()),
    );
}

/// A Span drag CROSSING ladder rungs changes level once per rung, not once
/// per frame.
///
/// [`dragging_the_span_holds_the_grid_between_ladder_steps`] stops just
/// inside both ends of one rung, so the crossing itself is the regime it
/// leaves out — and a crossing is the whole of what a horizontal zoom asks
/// of the pyramid. A crossed rung reads another level, which re-uploads
/// the run.
///
/// Driven as the pane drives it. The Span is exponential in drag distance
/// (`roll_seconds * (-along * DEPTH_ZOOM_PER_DRAG_POINT).exp()`) and the
/// rungs are powers of two, so the crossings are EVENLY spaced along the
/// drag — one every `ln 2 / DEPTH_ZOOM_PER_DRAG_POINT`, 116 points of
/// travel.
#[test]
fn a_span_drag_changes_level_once_per_rung_crossed() {
    let cols = LIVE_SLAB_CAP as usize;
    // Two rungs of travel each way: 50 s sits in the 64 ms rung and 10 s on
    // the ladder's floor, so a drag between them crosses at 32.768 s and
    // 16.384 s. Below the floor the width cannot move at all, which is why
    // the low end is not pushed further.
    let (hi, lo) = (50.0f64, 10.0f64);
    let mut widths = std::collections::BTreeSet::new();
    // A steady drag: 1.5 points of travel per frame. A slower one only adds
    // frames, never crossings, which is the whole shape of the claim.
    let step = (-1.5f64 * DEPTH_ZOOM_PER_DRAG_POINT as f64).exp();

    let mut span = hi;
    let (mut frames, mut moves) = (0u32, 0u32);
    let mut held = None;
    // Down the range and back up.
    for narrowing in [true, false] {
        loop {
            span = if narrowing { span * step } else { span / step };
            if narrowing && span < lo || !narrowing && span > hi {
                break;
            }
            frames += 1;
            let bucket = held_slab(span, cols, held);
            widths.insert((bucket * 1e6).round() as i64);
            moves += u32::from(held.is_some_and(|b| b != bucket));
            held = Some(bucket);
        }
    }

    assert_eq!(widths.len(), 3, "the sweep did not cross two rungs: {widths:?} us");
    assert!(frames > 300, "the sweep never ran: {frames} frames");
    assert_eq!(moves, 4, "a {frames}-frame Span drag across 2 rungs each way moved {moves} times");
}

/// A Span sitting ON a rung boundary settles on one slab width instead of
/// alternating, so the picture does not stop under a resting finger.
///
/// [`a_span_drag_changes_level_once_per_rung_crossed`] bounds a drag that
/// TRAVELS. This is the other regime, and it is not a corner: the bare
/// ladder re-decides the rung from the window alone on every frame, and one
/// drag-point is 0.6% of the Span — which a resting finger clears on tremor
/// alone. Every move reads another level and re-uploads the run.
///
/// [`held_slab`] is what settles it, and the claim is the one the hysteresis
/// exists to make: the tremor below straddles the boundary the whole way, so
/// the ladder ALONE answers two widths on all 300 frames.
#[test]
fn a_span_dithering_across_a_rung_settles_on_one_width() {
    let cols = LIVE_SLAB_CAP as usize;
    // The boundary between the 32 ms and 64 ms rungs: `live_slab` steps up
    // exactly where the window stops fitting in `cols` slabs.
    let boundary = 0.032 * cols as f64;
    let step = (-(DEPTH_ZOOM_PER_DRAG_POINT as f64)).exp();

    let mut held: Option<f64> = None;
    // How often the width MOVES, which is what an upload is paid for — the
    // set of widths seen cannot tell one step up from three hundred.
    let (mut moves, mut bare_moves) = (0u32, 0u32);
    let mut bare_prev: Option<f64> = None;
    // One point of tremor either way, alternating: the hand is still and the
    // Span is not.
    let frames = 300u32;
    for i in 0..frames {
        let span = if i % 2 == 0 { boundary * step } else { boundary / step };
        let raw = live_slab(span, cols);
        bare_moves += u32::from(bare_prev.is_some_and(|p| p != raw));
        bare_prev = Some(raw);
        let bucket = held_slab(span, cols, held);
        moves += u32::from(held.is_some_and(|p| p != bucket));
        held = Some(bucket);
    }

    // The fixture reaches the thing it is about: without the hold this
    // tremor moves the width on every frame but the first.
    assert_eq!(
        bare_moves,
        frames - 1,
        "the dither did not straddle a rung on every frame: {bare_moves} of {frames}",
    );
    // One step UP, at the first frame that does not fit the finer rung, and
    // nothing after it: the hold takes the boundary out of the tremor's reach.
    assert_eq!(moves, 1, "the width moved {moves} times under a still hand");
}

/// A slow drag with a hand's tremor on it crosses a rung ONCE, in either
/// direction.
///
/// [`a_span_dithering_across_a_rung_settles_on_one_width`] holds the still
/// hand. This is the regime between that and a travelling drag, and it is
/// the one [`RUNG_HOLD`]'s size is actually chosen against: a hand crossing
/// a boundary at a fifth of a drag-point a frame spends tens of frames
/// within tremor's reach of it, and every one of those the bare ladder
/// answers differently reads another level and re-uploads the run.
///
/// Two drag-points of tremor is 1.2% of the Span, against a margin of 5%.
/// The `bare` count is what says the fixture is inside the boundary's reach
/// at all — a drag that merely passed it would read 1 there too.
#[test]
fn a_slow_drag_with_tremor_crosses_a_rung_once() {
    let cols = LIVE_SLAB_CAP as usize;
    // The 32/64 ms rungs part where a window stops fitting in `cols` slabs.
    let boundary = 0.032 * cols as f64;
    let pt = f64::from(DEPTH_ZOOM_PER_DRAG_POINT);
    const TREMOR_PTS: f64 = 2.0;
    const PER_FRAME_PTS: f64 = 0.2;

    for travel in [PER_FRAME_PTS, -PER_FRAME_PTS] {
        // Open far enough the other side of the boundary that the drag
        // reaches it well inside the run below.
        let opening = if travel > 0.0 { 0.94 } else { 1.06 };
        let mut window = boundary * opening;
        let (mut held, mut bare) = (None, None);
        let (mut moves, mut bare_moves) = (0u32, 0u32);
        for i in 0..400 {
            // Tremor as a smooth deterministic wobble; the claim is about
            // its AMPLITUDE, and a hand does not shake in a straight line.
            let shake = (i as f64 * 2.399_963).sin() * TREMOR_PTS * pt;
            let w = window * shake.exp();
            let raw = live_slab(w, cols);
            bare_moves += u32::from(bare.is_some_and(|p| p != raw));
            bare = Some(raw);
            let b = held_slab(w, cols, held);
            moves += u32::from(held.is_some_and(|p| p != b));
            held = Some(b);
            window *= (travel * pt).exp();
        }
        assert!(
            bare_moves > 4,
            "the drag never lingered at the boundary: {bare_moves} bare moves at {travel} pt/frame",
        );
        assert_eq!(moves, 1, "a {travel} pt/frame drag re-cut the grid {moves} times");
    }
}

/// The hold reaches the pane through the SURFACE: a Span dithering on a
/// boundary changes level once, not once a frame.
///
/// [`a_span_dithering_across_a_rung_settles_on_one_width`] is the same claim
/// about [`held_slab`] alone. This one is about the wiring, which is the
/// half that can be right in the ladder and wrong in the pane: the level a
/// frame settles on has to be written back where the NEXT frame's plan
/// reads it ([`run_for`]), and a plan built without it is the bare ladder
/// with extra steps.
#[test]
fn the_held_width_survives_a_frame_through_the_surface() {
    let interval = crate::spectrogram::FFT_INTERVAL;
    let mut spectrum = crate::AudioSpectrum::default();
    let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
    let mut bins = [0.0f32; SPECTRUM_BINS];
    bins[1000] = 0.8;
    // The pane below asks for 1024 slabs, so the 32/64 ms rungs part at a
    // Span of 32.768 s; fill past it and settle.
    let boundary = 0.032 * LIVE_SLAB_CAP as f64;
    let mut clock = 0.0;
    while clock < boundary * 2.0 {
        spectrum.push_history(clock, &bins);
        clock += interval;
    }
    let view = |window: f64| PaneView {
        ppp: 2.0,
        pitch_len: 300.0,
        depth_len: LIVE_SLAB_CAP * 0.5,
        window,
        scale: SWEEP_SCALE,
        cfg: SpectrumConfig { roll_seconds: window as f32, ..SpectrumConfig::default() },
    };
    let step = (-(DEPTH_ZOOM_PER_DRAG_POINT as f64)).exp();

    // One drag-point of tremor either way, which is the hand holding still.
    let (mut levels, mut moves) = (std::collections::BTreeSet::new(), 0);
    for i in 0..200 {
        let window = if i % 2 == 0 { boundary * step } else { boundary / step };
        for _ in 0..2 {
            spectrum.push_history(clock, &bins);
            clock += interval;
        }
        let held = surfaces.at(0).held_level;
        let plan = Plan::new(&view(window), clock - window, held);
        levels.insert(plan.level);
        moves += u32::from(held.is_some_and(|level| level != plan.level));
        run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).expect("a run to draw");
    }

    // The opening frame's rung and the one it steps up to, and nothing after.
    assert_eq!(levels.len(), 2, "the pane saw more than one step: {levels:?}");
    assert_eq!(moves, 1, "a still hand on a rung boundary changed level {moves} times in 200");
}

#[test]
fn unchanged_columns_reuse_the_folded_snapshot() {
    let mut spectrum = crate::AudioSpectrum::default();
    let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
    let bins = [0.5; SPECTRUM_BINS];
    for i in 0..200 {
        spectrum.push_history(90.0 + i as f64 * 0.01, &bins);
    }
    let view = PaneView {
        ppp: 2.0,
        pitch_len: 300.0,
        depth_len: 600.0,
        window: 1.5,
        scale: SWEEP_SCALE,
        cfg: SpectrumConfig::default(),
    };
    let plan = Plan::new(&view, 91.99 - 1.5, None);
    let layout = run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).unwrap();
    let original = surfaces.at(0).folded.grid().unwrap();
    assert_eq!(Some(layout), run_for(spectrum.pyramid(), &mut surfaces, 0, &plan));
    assert!(Arc::ptr_eq(&original.run, &surfaces.at(0).folded.grid().unwrap().run));
    spectrum.push_history(92.0, &bins);
    run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).unwrap();
    assert!(!Arc::ptr_eq(&original.run, &surfaces.at(0).folded.grid().unwrap().run));
    assert_eq!(surfaces.at(0).folded.full_uploads(), 0, "folding is not an upload");
}

/// What makes the renderer's comparison cheap: while audio streams in,
/// consecutive snapshots share every slab but the one being filled and the
/// one just opened, so all the rest compare by pointer. The fixture keeps
/// a slab open across several frames, leaves one slab empty, which shares
/// the pyramid's silent slab, and moves the window's far edge on, so each of
/// those paths is taken.
#[test]
fn a_streaming_run_shares_every_slab_but_the_ones_it_is_filling() {
    // Level 1: 32 ms slabs, four columns each.
    let (level, bucket) = (1, 4.0 * crate::spectrogram::FFT_INTERVAL);
    assert_eq!(pyramid::width(level), bucket);
    let window = 64.0 * bucket;
    let mut pyramid = SlabPyramid::default();
    let mut previous: Option<(i64, Vec<SpectrogramSlab>)> = None;
    let (mut trimmed, mut gap) = (false, false);
    for i in 0..1200 {
        // A slab and a half of missed columns, once: exactly one slab
        // empty.
        if (600..606).contains(&i) {
            continue;
        }
        let time = i as f64 * crate::spectrogram::FFT_INTERVAL;
        let (t, power) = col(time, &[(5, 0.1 + (i % 7) as f32 * 0.1)]);
        pyramid.push(t, &power);
        let Some(run) = pyramid.run(level, time - window) else { continue };
        let first_key = run.first_key;
        let run: Vec<_> = run.slabs.cloned().collect();
        if let Some((was_first, was)) = &previous {
            trimmed |= first_key > *was_first;
            let moved = run
                .iter()
                .enumerate()
                .filter(|&(j, slab)| {
                    usize::try_from(first_key + j as i64 - was_first)
                        .ok()
                        .and_then(|at| was.get(at))
                        .is_none_or(|old| !Arc::ptr_eq(old, slab))
                })
                .count();
            assert!(moved <= 2, "column {i}: {moved} of {} slabs moved", run.len());
        }
        gap |= run.iter().any(|slab| Arc::ptr_eq(slab, pyramid.silence()));
        previous = Some((first_key, run));
    }
    assert!(trimmed, "the window's far edge never moved on");
    assert!(gap, "no empty slab shared the silent one");
}

/// The snapshot a surface holds changes only when the run does: a far edge
/// moving inside its slab reads the same run, and one crossing into the
/// next slab reads a new run of the same slabs, one shorter.
#[test]
fn the_folded_snapshot_changes_only_at_oldest_slab_boundaries() {
    let view = PaneView {
        ppp: 1.0,
        pitch_len: 64.0,
        depth_len: 256.0,
        window: 4.0,
        scale: SWEEP_SCALE,
        cfg: SpectrumConfig::default(),
    };
    let bucket = live_slab(view.window, 256);
    let mut pyramid = pyramid_of(
        [(0.0, 1.0), (0.2, 0.1), (0.4, 0.01), (2.2, 0.5), (3.2, 0.2)]
            .map(|(t, p)| col(t * bucket, &[(1000, p)])),
    );
    let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
    // The far edge one slab in, so the run starts one slab before it, at the
    // oldest slab.
    let initial = Plan::new(&view, bucket, None);
    assert_eq!(initial.bucket(), bucket);
    let layout = run_for(&mut pyramid, &mut surfaces, 0, &initial).unwrap();
    let snapshot = surfaces.at(0).folded.grid().unwrap();
    assert_eq!(snapshot.first_key, 0);
    for edge in [1.25, 1.5, 1.75] {
        let plan = Plan::new(&view, edge * bucket, None);
        assert_eq!(run_for(&mut pyramid, &mut surfaces, 0, &plan), Some(layout));
        assert!(Arc::ptr_eq(&snapshot.run, &surfaces.at(0).folded.grid().unwrap().run));
    }
    let next = Plan::new(&view, 3.0 * bucket, None);
    let advanced = run_for(&mut pyramid, &mut surfaces, 0, &next).unwrap();
    let grid = surfaces.at(0).folded.grid().unwrap();
    assert_eq!(grid.first_key, 2);
    assert_eq!(advanced.t_origin, 2.0 * bucket);
    assert_eq!(grid.run.as_ref(), &snapshot.run[2..]);
    assert!(!Arc::ptr_eq(&snapshot.run, &grid.run));
    // A new snapshot, and the same slabs: what the renderer compares by
    // pointer rather than by reading them.
    assert!(grid.run.iter().zip(&snapshot.run[2..]).all(|(a, b)| Arc::ptr_eq(a, b)));
}

/// The gradient table is keyed on what decides a texel of it, so two
/// gradients that draw one picture share a table and every other change
/// builds a new one.
///
/// Both directions matter and they fail differently. A knob the key misses
/// leaves the OLD table on the GPU under a gradient that no longer draws it
/// — a wrong picture, which no frame counter reports. A knob the key watches
/// that decides nothing rebuilds and re-uploads 4096 entries on every frame
/// of the drag that moves it, which is the cost
/// [`what_decides_a_texel`] exists to remove.
///
/// The folded pairs are checked as PICTURES and not just as keys: the fold
/// claims two gradients draw the same table, so the tables themselves are
/// compared. A fold that were merely a key equality would serve one
/// gradient's colours under another's settings.
#[test]
fn the_lut_key_folds_two_gradients_that_draw_one_picture() {
    // Keep every gradient knob away from its clamp. The captured default
    // spends the whole lightness axis, where moving its midpoint alone is
    // correctly sanitized back to the same pair and would not exercise a
    // changed picture.
    let cfg = SpectrumConfig {
        spectrogram_gradient: crate::SpectrogramPreset::Aurora.gradient(),
        ..SpectrumConfig::default()
    };
    let mut gpu = FoldedGrid::default();
    // A table built for `c` alone, so a folded pair can be compared as
    // pixels rather than as keys.
    let table = |c: &SpectrumConfig| {
        let mut fresh = FoldedGrid::default();
        fresh.shades(c).lut
    };
    // Whether the table `c` is served is a NEW one: its allocation changes
    // only where the fold does, whatever order these are asked in.
    let mut rebuilds = |c: &SpectrumConfig| {
        let before = gpu.shades(&cfg).lut;
        !Arc::ptr_eq(&gpu.shades(c).lut, &before)
    };

    // Every knob of the gradient recolours every texel without moving a
    // slab, so each has to move the key on its own.
    let edited = |edit: fn(&mut Gradient)| {
        let mut c = cfg;
        edit(&mut c.spectrogram_gradient);
        c
    };
    for edit in [
        (|g: &mut Gradient| g.hue_start += 10.0) as fn(&mut Gradient),
        |g: &mut Gradient| g.hue_span -= 10.0,
        |g: &mut Gradient| g.lightness -= 5.0,
        |g: &mut Gradient| g.lightness_ramp -= 5.0,
        |g: &mut Gradient| g.chroma -= 0.1,
        |g: &mut Gradient| g.chroma_ramp += 0.1,
    ] {
        let moved = edited(edit);
        assert!(rebuilds(&moved), "a gradient knob left the old table on the GPU");
        assert_ne!(table(&moved), table(&cfg), "the knob was expected to move a texel");
    }

    // And the level window does NOT: the dB floor, the ceiling and the tilt
    // reach a texel through the LEVEL it is looked up at, which is a
    // uniform, so a Level drag moves no entry of the table.
    for edit in [
        (|c: &mut SpectrumConfig| c.volume_floor_db -= 6.0) as fn(&mut SpectrumConfig),
        |c: &mut SpectrumConfig| c.volume_ceiling_db -= 6.0,
        |c: &mut SpectrumConfig| c.tilt += 1.0,
        |c: &mut SpectrumConfig| c.roll_seconds *= 1.01,
        |c: &mut SpectrumConfig| c.roll_fraction += 0.01,
        |c: &mut SpectrumConfig| c.atmosphere.pitch_softness = 250.0,
        |c: &mut SpectrumConfig| c.atmosphere.time_softness = 1800.0,
        |c: &mut SpectrumConfig| c.atmosphere.cloud_depth = 0.9,
        |c: &mut SpectrumConfig| c.atmosphere.material_settings.wash_refract = 0.8,
    ] {
        let mut moved = cfg;
        edit(&mut moved);
        assert!(!rebuilds(&moved), "a drag on this rebuilds the table on every frame");
    }

    // The hue pair at NO CHROMA, a grey ramp and the one place
    // a gradient knob decides nothing. `chroma_at` is 0 at every level, so
    // the absolute chroma is 0 whatever the hue, and Oklab's `a` and `b` are
    // `c * cos(h)` and `c * sin(h)` — identically 0. Every texel is the same
    // grey at every angle, and the spectrum bar's track is a DRAG.
    let mono = harmonigraph_scene::Gradient {
        lightness: 50.0,
        lightness_ramp: 100.0,
        chroma: 0.0,
        chroma_ramp: 0.0,
        ..crate::SpectrogramPreset::Ice.gradient()
    };
    let toneless = SpectrumConfig { spectrogram_gradient: mono, ..cfg };
    for turned in [30.0f32, 180.0, 359.0] {
        let mut c = toneless;
        c.spectrogram_gradient.hue_start = (mono.hue_start + turned).rem_euclid(360.0);
        c.spectrogram_gradient.hue_span = turned;
        assert_eq!(
            what_decides_a_texel(c.spectrogram_gradient.sanitized()),
            what_decides_a_texel(toneless.spectrogram_gradient.sanitized()),
            "a hue turned {turned} degrees at no chroma rebuilds a table that was \
                 still good",
        );
        assert_eq!(table(&c), table(&toneless), "...and it really does draw the same table");
    }

    // The same fact along the OTHER axis: a Brightness pair closed on either
    // end of the `L*` axis. `HUE_FLOOR` is 0 at both 0 and 100, so
    // `chroma_of` answers 0 for every fraction and every hue, and
    // `oklab_srgb` is black (white at 100) whichever way the arc runs — the
    // whole picture is one colour, and neither the hue pair nor the chroma
    // pair decides a texel of it.
    //
    // Reachable in one gesture and landed on exactly, not by luck:
    // `Spread::snapped` rounds both handles to whole `L*`, and every preset
    // already opens with silence at 0, so closing the pair down to the wall
    // is where a reader ends up. Reaching for the hue track or the Chroma
    // bar to get back OUT is then a drag rebuilding the table every frame
    // while changing not one entry.
    for l in [0.0f32, 100.0] {
        let flat = SpectrumConfig {
            spectrogram_gradient: Gradient {
                lightness: l,
                lightness_ramp: 0.0,
                chroma: 0.5,
                chroma_ramp: 0.0,
                hue_start: 40.0,
                hue_span: 120.0,
                ..Gradient::default()
            },
            ..cfg
        };
        for edit in [
            (|g: &mut Gradient| g.hue_start = (g.hue_start + 90.0).rem_euclid(360.0))
                as fn(&mut Gradient),
            |g: &mut Gradient| g.hue_span = -300.0,
            |g: &mut Gradient| g.chroma = 0.9,
            |g: &mut Gradient| g.chroma_ramp = 0.2,
        ] {
            let mut moved = flat;
            edit(&mut moved.spectrogram_gradient);
            assert_eq!(
                what_decides_a_texel(moved.spectrogram_gradient.sanitized()),
                what_decides_a_texel(flat.spectrogram_gradient.sanitized()),
                "at a Brightness pair closed on L* {l} the picture is one colour, and a \
                     drag on the hue or the chroma rebuilds the table every frame",
            );
            assert_eq!(table(&moved), table(&flat), "...and the table really is the same");
        }
    }

    // The curve's switches, folded the same three ways: a straight curve
    // applies to nothing, so which channels it is on decides nothing; a pair
    // closed on a wall is one colour whatever the curve; and without tone the
    // hue and chroma switches decide only the spacing, through whether either
    // is on. Each pair is a key AND a table, as above.
    let bend = |at, hue, lightness, chroma| harmonigraph_scene::Bend {
        at,
        share: 0.5,
        hue,
        lightness,
        chroma,
    };
    let with = |c: SpectrumConfig, b| SpectrumConfig {
        spectrogram_gradient: Gradient { bend: b, ..c.spectrogram_gradient },
        ..c
    };
    let unlit = SpectrumConfig {
        spectrogram_gradient: Gradient { lightness: 0.0, lightness_ramp: 0.0, ..mono },
        ..cfg
    };
    for (base, a, b, why) in [
        (cfg, bend(0.5, true, false, false), bend(0.5, false, true, true), "a straight curve"),
        (unlit, bend(0.2, true, true, true), bend(0.8, false, false, true), "a closed pair"),
        (toneless, bend(0.2, true, false, false), bend(0.2, true, false, true), "no tone, hue"),
        (toneless, bend(0.2, false, false, true), bend(0.2, true, false, true), "no tone, chroma"),
    ] {
        let (a, b) = (with(base, a), with(base, b));
        assert_eq!(
            what_decides_a_texel(a.spectrogram_gradient.sanitized()),
            what_decides_a_texel(b.spectrogram_gradient.sanitized()),
            "{why}: a switch that decides no texel rebuilds the table",
        );
        assert_eq!(table(&a), table(&b), "{why}: ...and the table really is the same");
    }

    // The converses, so neither fold can be a blanket "ignore the hues": a
    // picture WITH chroma watches them, and so does a brightness pair closed
    // anywhere BETWEEN the ends, which still draws a colour.
    let mut coloured = toneless;
    coloured.spectrogram_gradient.chroma = 0.5;
    let mut turned = coloured;
    turned.spectrogram_gradient.hue_start += 40.0;
    assert_ne!(table(&turned), table(&coloured), "a coloured ramp must watch its hues");
    let mid = SpectrumConfig {
        spectrogram_gradient: Gradient {
            lightness: 50.0,
            lightness_ramp: 0.0,
            chroma: 0.5,
            chroma_ramp: 0.0,
            hue_start: 40.0,
            hue_span: 120.0,
            ..Gradient::default()
        },
        ..cfg
    };
    let mut mid_turned = mid;
    mid_turned.spectrogram_gradient.hue_start += 90.0;
    assert_ne!(
        what_decides_a_texel(mid_turned.spectrogram_gradient.sanitized()),
        what_decides_a_texel(mid.spectrogram_gradient.sanitized()),
        "a flat ramp between the ends still draws a colour",
    );
}

/// The affine and the table the UI hands the shader stand in for the
/// mapping they replaced, byte for byte.
///
/// The shader is given three scalars and a table of [`SHADES`] colours in
/// place of `spectrogram_level_db` and `cell_color`; if the pair parts
/// company with the mapping, the heatmap draws a colour the curve beside it
/// never would, at every level rather than at one. A table indexed by level
/// cannot be exact for every row at once (see [`SHADES`]), so the claim is
/// one level of one channel — and the RATE as well as the size, because a
/// row read out of the CLAMPED mapping is flat and every texel of it would
/// still be "within one level" of black.
///
/// CPU, because both halves are pure functions of the config: `level_affine`
/// is what [`read_of`] uploads and [`FoldedGrid::shades`] is the table beside
/// it. What the SHADER makes of the pair is
/// `gpu::the_curve_and_the_heatmap_read_a_run_of_buckets_alike`'s.
#[test]
fn the_shade_table_matches_the_mapping_it_replaces() {
    use crate::panes::spectral::axes::spectrogram_level_db;
    let cell_color = crate::panes::spectral::spectrogram::cell_color;
    let (mut worst, mut differing, mut total) = (0i32, 0u64, 0u64);
    for ramp in crate::SpectrogramPreset::ALL.map(|p| p.gradient()) {
        // Including a window narrower than the guard allows, so the table
        // inherits the same collapse `spectrogram_window` protects against.
        for &(floor, ceiling) in &[(-60.0, 0.0), (-120.0, 6.0), (-30.0, -20.0), (-10.0, -10.0)] {
            for tilt in [0.0, 3.0, -3.0, 6.0] {
                let cfg = SpectrumConfig {
                    spectrogram_gradient: ramp,
                    volume_floor_db: floor,
                    volume_ceiling_db: ceiling,
                    tilt,
                    ..SpectrumConfig::default()
                };
                let lut = FoldedGrid::default().shades(&cfg).lut;
                // A row per octave across the spectrum's whole reach, so
                // the tilt is sampled where it is largest as well as at the
                // pivot.
                for i in 0..12 {
                    let midi = SPECTRUM_MIN_MIDI + i as f32 * 12.0;
                    for byte in 0..=BucketDb::MAX {
                        let want = cell_color(ramp, spectrogram_level_db(&cfg, db_of(byte), midi));
                        // The index the shader takes: the affine it is
                        // handed, truncated into the table.
                        let level = bin_level_for_test(&cfg, byte, midi);
                        let got = lut[((level * SHADES as f32) as usize).min(SHADES - 1)];
                        total += 1;
                        let d = [
                            (i32::from(got[0]) - i32::from(want.r())).abs(),
                            (i32::from(got[1]) - i32::from(want.g())).abs(),
                            (i32::from(got[2]) - i32::from(want.b())).abs(),
                        ]
                        .into_iter()
                        .max()
                        .expect("three channels");
                        differing += u64::from(d > 0);
                        assert!(
                            d <= 1,
                            "{ramp:?} floor {floor} ceiling {ceiling} tilt {tilt}: midi \
                                 {midi} byte {byte} moved a channel by {d} levels ({got:?} \
                                 against {want:?})",
                        );
                        worst = worst.max(d);
                    }
                }
            }
        }
    }
    assert!(
        differing * 20 < total,
        "{differing} of {total} texels differ (worst {worst} level) — the table is meant \
             to stand in for the mapping, not to approximate it",
    );
}

/// The surface's snapshot changes with WHICH COLUMNS were folded, and with
/// nothing about how they are read.
///
/// Both directions again. An input that decides the run and does not reach
/// the surface's memo draws a stale run — the picture stops at a column that
/// has already scrolled past. One that decides nothing about the run
/// re-uploads it on every frame of the drag that moves it, and every one of
/// those is a drag that moves a uniform: the zoom, the row count and the
/// palette all read the same bytes a different way.
#[test]
fn the_snapshot_follows_the_fold_and_not_the_read() {
    let interval = crate::spectrogram::FFT_INTERVAL;
    let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
    let base_view = PaneView {
        ppp: 2.0,
        pitch_len: 300.0,
        depth_len: 800.0,
        window: 12.0,
        scale,
        cfg: SpectrumConfig::default(),
    };
    // Off the slab grid by a quarter interval, so columns 4 and 5 share a
    // slab at the two-interval rung this view settles on.
    let at = |i: usize| (i as f64 + 0.25) * interval;
    let mut spectrum = crate::AudioSpectrum::default();
    let power = |p: f32| {
        let mut bins = [0.0; SPECTRUM_BINS];
        bins[1000] = p;
        bins
    };
    for i in 0..599 {
        spectrum.push_history(at(i), &power(0.5));
    }
    let from = at(4);
    let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
    let base_plan = Plan::new(&base_view, from, None);
    assert_eq!(base_plan.bucket(), 2.0 * interval, "the fixture's slab width");
    let mut draw = |spectrum: &mut crate::AudioSpectrum, v: &PaneView, from: f64| {
        let plan = Plan::new(v, from, None);
        run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).expect("a run to draw");
        surfaces.at(0).folded.grid().unwrap().run
    };
    let base = draw(&mut spectrum, &base_view, from);
    let same = |run: Arc<[SpectrogramSlab]>, what: &str| {
        assert!(Arc::ptr_eq(&base, &run), "{what} handed out a new run");
    };
    same(draw(&mut spectrum, &base_view, from), "the same frame");

    // A far edge moving inside the same slab reads the same slabs.
    same(draw(&mut spectrum, &base_view, at(5)), "a far edge inside a slab");

    // And everything that is a UNIFORM: the rows, the pitch range and every
    // colour input read the run rather than deciding it, so none of them may
    // hand out another.
    let mut read_only = |v: PaneView| same(draw(&mut spectrum, &v, from), "a uniform change");
    read_only(PaneView { pitch_len: 900.0, ..base_view });
    read_only(PaneView { ppp: 1.0, pitch_len: 600.0, ..base_view });
    read_only(PaneView {
        scale: PitchScale { min_midi: 60.0, max_midi: 84.0, span: 24.0 },
        ..base_view
    });
    for edit in [
        (|c: &mut SpectrumConfig| c.volume_floor_db -= 6.0) as fn(&mut SpectrumConfig),
        |c: &mut SpectrumConfig| c.volume_ceiling_db -= 6.0,
        |c: &mut SpectrumConfig| c.tilt += 1.0,
        |c: &mut SpectrumConfig| c.spectrogram_gradient.hue_start += 40.0,
        |c: &mut SpectrumConfig| c.roll_fraction += 0.01,
    ] {
        let mut cfg = base_view.cfg;
        edit(&mut cfg);
        read_only(PaneView { cfg, ..base_view });
    }

    // What decides the run reaches the one memo of it. A fresh column, into
    // the slab still open, so the run keeps its length:
    spectrum.push_history(at(599), &power(0.25));
    let fresh = draw(&mut spectrum, &base_view, from);
    assert_eq!(fresh.len(), base.len(), "the fresh column opened a slab");
    assert!(!Arc::ptr_eq(&base, &fresh), "a new column reused the old run");
    assert_ne!(fresh.last(), base.last(), "the open slab kept its old average");
    // And the slab width, through the ladder.
    let coarse_view = PaneView { depth_len: 200.0, ..base_view };
    assert_ne!(Plan::new(&coarse_view, from, None).level, base_plan.level);
    let coarser = draw(&mut spectrum, &coarse_view, from);
    assert!(!Arc::ptr_eq(&fresh, &coarser), "a new rung reused the old run");
}

/// The strip is drawn out to the now-line, but the newest column is always
/// older than that — half an analysis window, by construction. The shader
/// clamps a tap into the run, so a coordinate allowed to run on holds the
/// newest slab's colour across the whole sliver rather than reading a slab
/// that does not exist — but only the coordinate says where the newest slab
/// IS, and it must stop at that slab's CENTRE, which is where the run's own
/// filtering stops having two taps to blend.
#[test]
fn the_leading_sliver_holds_the_newest_slabs_centre() {
    // A live window at the settings that make the sliver widest: a short
    // span, so slabs sit on the ladder's lowest rung and the analyzer's lag
    // spans several of them.
    let bucket = crate::spectrogram::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
    let window = 2.0;
    let visible = (window / bucket) as usize; // 62 slabs
    let layout = TexLayout { bucket, t_origin: 400.0, tex_span: visible as f64 * bucket };
    // The now-line: the newest column lags by half a Precise window, and
    // its slab has to finish before the next one starts.
    let now = layout.t_origin + layout.tex_span + 0.171;

    let newest = slab_drawn(&layout, now);
    assert!(
        newest <= visible as f32 - 0.5 + 1e-4,
        "the sliver reached {newest} slabs in, past the newest slab's centre at {}",
        visible as f32 - 0.5,
    );
    // And it is the newest slab it holds, not something short of it.
    assert!(newest >= visible as f32 - 0.5 - 1e-4, "held short of the newest slab");
    // Worth pinning for, because an unheld mapping runs several slabs past
    // the run rather than a fraction of one.
    let unheld = slab_at(&layout, now);
    assert!(unheld > visible as f32 + 4.0, "expected a multi-slab overrun, got {unheld}");
}

/// Everything BEFORE the hold is untouched by it: the drawn mapping is the
/// plain one over the data, so the picture still tracks the notes slab for
/// slab, and the hold is a corner rather than a bend that creeps inward.
#[test]
fn holding_the_sliver_leaves_the_data_mapping_alone() {
    let bucket = 0.05;
    let layout = TexLayout { bucket, t_origin: 10.0, tex_span: 20.0 * bucket };
    let hold = hold_time(&layout);
    assert!((hold - (layout.t_origin + layout.tex_span - 0.5 * bucket)).abs() < 1e-9);
    let mut t = layout.t_origin - 2.0 * bucket;
    while t <= hold {
        assert_eq!(slab_drawn(&layout, t), slab_at(&layout, t), "bent at {t}");
        t += bucket / 8.0;
    }
    // Past it, pinned — however far past, and however long the analyzer
    // stalls for.
    let pinned = slab_at(&layout, hold);
    for t in [hold + 1e-6, hold + bucket, hold + 10.0] {
        assert_eq!(slab_drawn(&layout, t), pinned, "ran on at {t}");
    }
}

/// The time -> slab mapping has to be a straight line, including across the
/// slab boundary the newest column sits on. Clamping it to the run's end
/// pins it for part of every slab and lets it slide for the rest, and since
/// this is a VERTEX attribute that rescales the whole picture once per slab
/// — visible as the heatmap jittering. Which is why the one place the
/// coordinate does stop ([`slab_drawn`]) is a corner the mesh is SPLIT on,
/// leaving no quad to interpolate across it and the data quad straight from
/// end to end.
#[test]
fn the_time_to_slab_mapping_is_a_straight_line() {
    let bucket = 0.08;
    let layout = TexLayout { bucket, t_origin: 100.0, tex_span: 8.0 * bucket };
    let step = bucket / 4.0;
    let at = |i: i32| slab_at(&layout, layout.t_origin + i as f64 * step);

    // Equal steps in time, equal steps in the coordinate — everywhere,
    // including past BOTH ends of the run where the quad reaches for its
    // slivers.
    let expected = at(1) - at(0);
    assert!(expected > 0.0, "u must advance with time");
    for i in -4..40 {
        let moved = at(i + 1) - at(i);
        assert!(
            (moved - expected).abs() < 1e-6,
            "step {i} bent the mapping: expected {expected}, got {moved}",
        );
    }
}

/// The heatmap as the GPU draws it: the claims whose subject is the picture
/// rather than the bookkeeping under it.
///
/// Every test here returns early where the machine has no GPU adapter, which is
/// what [`SpectrogramHeadless::new`] answers `None` for.
mod gpu {
    use super::*;
    use crate::panes::spectral::axes::spectrogram_level_db;
    use crate::panes::spectral::spectrogram::footprint_mean_db;
    use harmonigraph_render::{SpectrogramHeadless, SpectrogramVertex};

    /// Pixels across a test frame, and so slabs across it: a readback row must
    /// be 256-byte aligned, which puts the width at a multiple of 64.
    const W: u32 = 64;

    /// The pitch a fragment at the centre of pixel row `r` covers, as
    /// spectrogram.wgsl lays the footprint out.
    ///
    /// Here to PLACE a fixture — which buckets a pixel will reach for — and
    /// to write the controls that give a claim its teeth. It never stands
    /// in for the read: what a pixel comes out at is measured off the
    /// frame.
    struct Foot {
        lo_t: f32,
        hi_t: f32,
        midi: f32,
    }

    fn foot_of(read: &SpectrogramRead, r: u32) -> Foot {
        let rows = read.rows as f32;
        let half = 0.5 / rows;
        let t = (r as f32 + 0.5) / rows;
        Foot { lo_t: t - half, hi_t: t + half, midi: read.min_midi + t * read.span }
    }

    fn bucket_of(read: &SpectrogramRead, t: f32) -> usize {
        let midi = read.min_midi + t * read.span;
        let b = ((midi - read.spectrum_min_midi) * read.bins_per_semitone).floor();
        b.clamp(0.0, SPECTRUM_BINS as f32 - 1.0) as usize
    }

    /// The buckets pixel row `r` reads, whichever arm it takes.
    fn run_of(read: &SpectrogramRead, r: u32) -> std::ops::Range<usize> {
        let foot = foot_of(read, r);
        let idx = bucket_of(read, foot.lo_t);
        idx..(bucket_of(read, foot.hi_t) + 1).min(SPECTRUM_BINS)
    }

    /// The whole pitch axis, which is where a row spans the most buckets and
    /// the row geometry is worked hardest.
    fn whole_axis() -> PitchScale {
        let span = SPECTRUM_BINS as f32 / BINS_PER_SEMITONE as f32;
        PitchScale { min_midi: SPECTRUM_MIN_MIDI, max_midi: SPECTRUM_MIN_MIDI + span, span }
    }

    /// The half of a [`PaneView`] [`read_of`] reads; the rest decides a fold
    /// that no test here performs.
    fn view_of(scale: PitchScale, cfg: SpectrumConfig) -> PaneView {
        PaneView { ppp: 1.0, pitch_len: 1.0, depth_len: 1.0, window: 1.0, scale, cfg }
    }

    /// A grid the callback takes, keyed from 0 in a ring exactly the run's
    /// size — so `key mod capacity` is the identity and a fixture about the
    /// READ says nothing about slots.
    ///
    fn grid_of(run: Vec<u8>) -> SpectrogramGrid {
        let slabs = run.len() / SPECTRUM_BINS;
        SpectrogramGrid {
            full_uploads: Arc::default(),
            capacity: slabs as u32,
            bins: SPECTRUM_BINS as u32,
            first_key: 0,
            run: SpectrogramGrid::slabs_of(&run, SPECTRUM_BINS),
        }
    }

    /// A quad over `size`, mapping pixel column `px` to slab `px + 0.5` and
    /// pixel row `py` to the pane pixel of the same index.
    ///
    /// Slab `px + 0.5` is a slab's own centre, where the time axis' blend
    /// collapses onto one slab; the pitch coordinate is the plain fraction
    /// the pane's own mesh carries, so a pixel of the frame reads the
    /// footprint pixel `py` of the pane reads and a claim about a pixel can
    /// be read off it. That costs the width and the height: `size` must be
    /// the slab count by the pane's row count.
    fn texel_quad(read: &SpectrogramRead, slabs: u32, size: [u32; 2]) -> Vec<SpectrogramVertex> {
        assert_eq!(size, [slabs, read.rows], "a texel per pixel means one pixel per texel");
        let (w, h) = (size[0] as f32, size[1] as f32);
        let v = |x: f32, y: f32| SpectrogramVertex {
            pos: [x, y],
            slab: x / w * slabs as f32,
            t: y / h,
        };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    }

    fn pixel(frame: &[u8], size: [u32; 2], px: u32, py: u32) -> [u8; 4] {
        let i = ((py * size[0] + px) * 4) as usize;
        frame[i..i + 4].try_into().expect("four channels")
    }

    /// A gradient table that reads back what the shader READ: entry `i` is the
    /// grey `i`, which [`probe_read`]'s level mapping indexes exactly, so a
    /// pixel's red channel is the stored dB byte the row came out at.
    ///
    /// An instrument rather than the picture. These claims are about the read,
    /// and getting a number back out of a real ramp would mean inverting a
    /// gamut solve.
    fn probe_shades() -> SpectrogramShades {
        SpectrogramShades {
            lut: Arc::new((0..256).map(|i| [i as u8, i as u8, i as u8, 255]).collect()),
        }
    }

    /// [`read_of`]'s scalars with the level mapping replaced by
    /// `(byte + 0.5) / 256`.
    ///
    /// [`probe_shades`] then lands on the byte itself: the shader's index is
    /// `min(u32(level * 256), 255)` and the division is exact in binary, so
    /// the half puts a whole byte at the CENTRE of its table slice and the
    /// truncating index becomes a rounding. Without it the read's last bit
    /// decides the answer — a footprint over thirty copies of one level
    /// sums and divides to that level plus or minus a rounding, and on the
    /// truncating index half of those land an entry low.
    fn probe_read(view: &PaneView, rows: usize) -> SpectrogramRead {
        SpectrogramRead {
            level0: 0.5 / 256.0,
            level_per_step: 1.0 / 256.0,
            level_per_midi: 0.0,
            ..read_of(view, rows)
        }
    }

    /// A quad over the whole run and the whole visible pitch range.
    ///
    /// Not the pane's own mesh, deliberately: the pane cuts its strip to the
    /// depths the data reaches and holds a sliver past the newest slab's
    /// centre, so a slab at either end of the run can be off-picture — and a
    /// fixture drawn through it would not reach the slot it claims to be
    /// checking. Where the strip is cut and where the sliver holds is
    /// `the_leading_sliver_holds_the_newest_slabs_centre`'s claim, not this
    /// one's.
    fn run_quad(slabs: u32, size: [u32; 2]) -> Vec<SpectrogramVertex> {
        let (w, h) = (size[0] as f32, size[1] as f32);
        let v = |x: f32, y: f32| SpectrogramVertex {
            pos: [x, y],
            slab: x / w * slabs as f32,
            t: 1.0 - y / h,
        };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    }

    /// One quad per slab, each carrying its slab coordinate CONSTANT across its
    /// own block of pixel columns.
    ///
    /// A block per slab and no blend between them, so a column of the frame
    /// names exactly one slot: a mis-slotted slab is a whole block of the wrong
    /// value rather than a seam a filter could explain. The run may then be any
    /// width the fold produces, where [`texel_quad`] needs it to be the frame's.
    fn slab_blocks(slabs: u32, size: [u32; 2]) -> Vec<SpectrogramVertex> {
        let (w, h) = (size[0] as f32, size[1] as f32);
        (0..slabs)
            .flat_map(|j| {
                let (x0, x1) = (w * j as f32 / slabs as f32, w * (j + 1) as f32 / slabs as f32);
                let v = |x: f32, y: f32| SpectrogramVertex {
                    pos: [x, y],
                    slab: j as f32 + 0.5,
                    t: 1.0 - y / h,
                };
                [v(x0, 0.0), v(x1, 0.0), v(x1, h), v(x0, 0.0), v(x1, h), v(x0, h)]
            })
            .collect()
    }

    /// The pixel column comfortably inside slab `j`'s block — its centre,
    /// which is at least half a block from either seam.
    fn block_centre(j: u32, slabs: u32, size: [u32; 2]) -> u32 {
        ((j as f32 + 0.5) / slabs as f32 * size[0] as f32) as u32
    }

    /// The pane's own geometry and settings, sized so a run fits inside a
    /// frame's pixel columns with room to see each slab.
    fn pane(window: f64, depth_len: f32, rows: f32) -> PaneView {
        PaneView {
            ppp: 1.0,
            pitch_len: rows,
            depth_len,
            window,
            scale: whole_axis(),
            cfg: SpectrumConfig { roll_seconds: window as f32, ..SpectrumConfig::default() },
        }
    }

    /// A column whose whole spectrum is the one stored byte `db`, so a slab
    /// folded from it reads that byte at every row and a block of the frame is
    /// one number.
    fn flat_column(time: f64, db: BucketDb) -> Col {
        let power = 10f32.powf(0.1 * harmonigraph_core::spectrogram::db_of(db));
        col(time, &(0..SPECTRUM_BINS).map(|b| (b, power)).collect::<Vec<_>>())
    }

    /// What the GPU holds equals a full upload of the same run, through every
    /// way the run can move.
    ///
    /// The grid's GPU copy is the one cached thing left in this pipeline, and
    /// its key is a statement about which slabs are in which slots. A slot the
    /// key believes and the buffer contradicts is a WRONG COLUMN — a lap-old
    /// slice of audio drawn as if it were now — and nothing on the CPU can see
    /// one: the run is right, the geometry is right, and the picture is
    /// plausible. So the delta is measured against the only thing that can
    /// contradict it, a frame built from the run alone.
    ///
    /// Every event that moves the run is scripted here and counted, because a
    /// sequence that quietly never folded, never advanced or never wrapped would
    /// pass this by drawing one picture twice.
    /// What the sequence below has to have reached for its equality to mean
    /// anything, counted as it plays.
    #[derive(Default)]
    struct Tally {
        /// The clock the fixture's columns are stamped on, and how many have
        /// been pushed.
        clock: f64,
        pushed: u32,
        folds: u32,
        advances: u32,
        reveals: u32,
        crossings: u32,
        rebuilds: u32,
        /// The previous frame's slab width and first key, which is what makes
        /// a crossing, a reveal and a boundary advance tellable apart.
        last: Option<(f64, i64)>,
        /// Fresh pane IDs for the full-upload side, so it never inherits
        /// its own previous frame.
        fresh: u64,
    }

    #[test]
    fn the_gpu_grid_equals_a_full_upload_after_any_sequence() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        const SIZE: [u32; 2] = [128, 64];
        let interval = crate::spectrogram::FFT_INTERVAL;
        let cfg = SpectrumConfig::default();
        let mut spectrum = crate::AudioSpectrum::default();
        let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
        let mut tally = Tally { clock: 100.0, ..Tally::default() };

        // A moving peak over a low bed, so consecutive slabs differ and a slab
        // written into the wrong slot draws a different picture.
        let push = |t: &mut Tally, spectrum: &mut crate::AudioSpectrum, n: u32, gap: f64| {
            t.clock += gap;
            for _ in 0..n {
                let mut bins = [0.02f32; SPECTRUM_BINS];
                bins[400 + (t.pushed as usize * 13) % 3000] = 1.0;
                spectrum.push_history(t.clock, &bins);
                t.clock += interval;
                t.pushed += 1;
            }
        };

        let frame = |t: &mut Tally,
                     spectrum: &mut crate::AudioSpectrum,
                     surfaces: &mut crate::spectrogram::SpectrogramSurfaces,
                     headless: &mut SpectrogramHeadless,
                     view: &PaneView|
         -> (SpectrogramGrid, SpectrogramRead, Vec<u8>) {
            let plan = Plan::new(view, t.clock - view.window, None);
            let uploads = surfaces.at(0).folded.full_uploads();
            let held = surfaces.at(0).folded.grid().map(|grid| grid.run);
            run_for(spectrum.pyramid(), surfaces, 0, &plan).expect("a run to draw");
            let run = surfaces.at(0).folded.grid().expect("a run to draw").run;
            t.folds += u32::from(held.is_none_or(|held| !Arc::ptr_eq(&held, &run)));
            let (grid, shades) = frame_data(surfaces, 0, &cfg).expect("a grid to draw");
            let slabs = grid.run.len() as u32;
            let read = read_of(view, plan.rows);
            let vertices = run_quad(slabs, SIZE);

            let bucket = plan.bucket();
            if let Some((was, first_key)) = t.last {
                t.crossings += u32::from(was != bucket);
                t.reveals += u32::from(grid.first_key < first_key && was == bucket);
                t.advances += u32::from(grid.first_key > first_key && was == bucket);
            }
            t.last = Some((bucket, grid.first_key));

            let delta = headless.frame(
                0,
                SIZE,
                vertices.clone(),
                grid.clone(),
                read.clone(),
                shades.clone(),
            );
            t.rebuilds += u32::from(surfaces.at(0).folded.full_uploads() > uploads);
            // An unused pane ID forces the reference to upload the whole run.
            t.fresh += 1;
            let whole = SpectrogramGrid { full_uploads: Arc::default(), ..grid.clone() };
            let full = headless.frame(t.fresh, SIZE, vertices, whole, read.clone(), shades);
            assert_eq!(delta, full, "a slot the delta wrote disagrees with the run it named");
            (grid, read, delta)
        };

        // Spans that straddle a rung of `live_slab`'s ladder at this pane
        // width: 0.6 s cuts into 16 ms slabs and 0.8 s into 32 ms ones.
        let (near, mid, far) = (0.3f64, 0.6, 0.8);
        let view = |window: f64| {
            let mut view = pane(window, 40.0, 64.0);
            // A colour window the bed and the peak both land inside, as the
            // one of 2026-09-26 did: one they both clip at the top of draws
            // every slot the same white, and a mis-slotted slab with it.
            view.cfg.volume_floor_db = -64.853_1;
            view.cfg.volume_ceiling_db = -28.842_316;
            view
        };
        push(&mut tally, &mut spectrum, 120, 0.0);
        for _ in 0..3 {
            frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
            push(&mut tally, &mut spectrum, 4, 0.0);
        }
        // A window scrolling by less than a slab: its first slab keeps its key
        // while columns leave it; its complete average remains unchanged.
        for _ in 0..6 {
            push(&mut tally, &mut spectrum, 1, 0.0);
            frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        }
        // A window narrowed and widened again: the widening reaches back to
        // slabs the frame before it did not draw, and the far end crosses a
        // rung on the way.
        for span in [near, near, mid, far, far, mid] {
            frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(span));
            push(&mut tally, &mut spectrum, 4, 0.0);
        }
        // One empty slab: a gap in the sample stream, which a level fills
        // with silence.
        push(&mut tally, &mut spectrum, 6, 0.024);
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        // Clear display history, and the history starting again after it:
        // no delta can describe that. A few columns after it, which without
        // the Clear would move a slab or two of the run and go as a delta.
        spectrum.clear_history();
        push(&mut tally, &mut spectrum, 6, 0.0);
        let rebuilds = tally.rebuilds;
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        assert!(tally.rebuilds > rebuilds, "the run after a Clear was sent as a delta");
        push(&mut tally, &mut spectrum, 40, 0.0);
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        push(&mut tally, &mut spectrum, 8, 0.0);
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        // A context that went away: the copy is gone and nothing about it can
        // be assumed.
        headless.reset_resources();
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));
        push(&mut tally, &mut spectrum, 8, 0.0);
        frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid));

        for (count, what) in [
            (tally.folds, "folds"),
            (tally.advances, "oldest slab advances"),
            (tally.reveals, "slabs revealed by a widening window"),
            (tally.crossings, "ladder rungs crossed"),
            (tally.rebuilds, "full uploads"),
        ] {
            assert!(count > 0, "the sequence never reached any {what}");
        }

        // Fixture sensitivity: advancing the history must change actual pixels.
        let before = frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid)).2;
        push(&mut tally, &mut spectrum, 8, 0.0);
        let after = frame(&mut tally, &mut spectrum, &mut surfaces, &mut headless, &view(mid)).2;
        assert_ne!(before, after, "the upload sequence never changed the picture");
    }

    /// Keys before zero and a run that wraps past the end of the ring place
    /// each slab where the shader reads it.
    ///
    /// Three readers work out a slot from a key and no compiler checks that
    /// they agree: [`RING_SLOTS`] sizes the ring, the render crate scatters
    /// a slab into `key mod capacity`, and the shader walks forward from the
    /// run's first slot with the same modulus. Two of the three are only ever
    /// exercised together at the two places the arithmetic is not the identity
    /// — a key before zero, where a truncating remainder answers negative, and
    /// a run that crosses the ring's end, where the forward walk has to come
    /// round.
    ///
    /// The render crate holds the same pair against a grid it builds itself;
    /// this one goes through the fold, so the capacity and the keys are the
    /// pane's own rather than a fixture's.
    #[test]
    fn slab_keys_before_zero_and_a_wrapping_run_land_where_the_shader_reads() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        const SIZE: [u32; 2] = [64, 4];
        let interval = crate::spectrogram::FFT_INTERVAL;
        let cfg = SpectrumConfig::default();
        let view = pane(0.3, 24.0, 24.0);
        let bucket = live_slab(view.window, 24);

        // Time runs from before zero to after it, and a slab's byte is a
        // function of its own KEY — so neighbouring slabs differ, a slab drawn
        // from the wrong slot is the wrong number, and the ramp never repeats
        // inside a run.
        let mut spectrum = crate::AudioSpectrum::default();
        let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
        let mut columns = (0..238).map(|i| {
            let time = -1.5 + i as f64 * interval;
            let key = (time / bucket).floor() as i64;
            flat_column(time, 60 + key.rem_euclid(190) as u8)
        });
        let mut next = columns.next();

        let (mut negative, mut wrapped) = (0u32, 0u32);
        let mut now = -1.2;
        while now < 0.4 {
            // As a live stream delivers them: every column up to the frame.
            while let Some((time, power)) = next.take_if(|(time, _)| *time <= now) {
                spectrum.push_history(time, &power);
                next = columns.next();
            }
            let plan = Plan::new(&view, now - view.window, None);
            run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).expect("a run to draw");
            let (grid, _) = frame_data(&mut surfaces, 0, &cfg).expect("a grid to draw");
            let slabs = grid.run.len() as u32;
            let first_slot = grid.first_key.rem_euclid(i64::from(grid.capacity)) as u32;
            negative += u32::from(grid.first_key < 0);
            wrapped += u32::from(first_slot + slabs > grid.capacity);

            let read = probe_read(&view, plan.rows);
            let frame = headless.frame(
                0,
                SIZE,
                slab_blocks(slabs, SIZE),
                grid.clone(),
                read,
                probe_shades(),
            );
            for j in 0..slabs {
                let want = grid.run[j as usize][0];
                let got = pixel(&frame, SIZE, block_centre(j, slabs, SIZE), SIZE[1] / 2)[0];
                assert_eq!(
                    got,
                    want,
                    "slab {j} of the run at key {} (slot {}, ring {}) drew slot {:?}",
                    grid.first_key,
                    (grid.first_key + i64::from(j)).rem_euclid(i64::from(grid.capacity)),
                    grid.capacity,
                    (0..slabs).find(|&k| grid.run[k as usize][0] == got),
                );
                if j > 0 {
                    assert_ne!(
                        want,
                        grid.run[(j - 1) as usize][0],
                        "slabs {} and {j} draw alike, so a swap between them is invisible",
                        j - 1,
                    );
                }
            }
            now += 2.0 * bucket;
        }
        assert!(negative > 0, "no frame's run started before zero");
        assert!(wrapped > 0, "no frame's run crossed the ring's end");
    }

    /// Energy at one bucket of one slab lights that slab's column at that
    /// bucket's row, and nothing else.
    ///
    /// The row geometry and the slot arithmetic each place a texel, in
    /// different units, and a picture with everything in the right place except
    /// its energy is the failure neither of them reports: the frame is bright
    /// somewhere, so nothing looks broken, and it is a transposition or an
    /// off-by-one row away from the sound it claims to draw.
    #[test]
    fn energy_at_one_bucket_of_one_slab_lights_that_texel_and_no_other() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        // Rows far coarser than the bucket grid, so a row is a RUN and the
        // bucket has to be found inside one rather than landed on.
        const ROWS: u32 = 32;
        const SLAB: u32 = 17;
        const ROW: u32 = 11;
        let view = view_of(whole_axis(), SpectrumConfig::default());
        let read = probe_read(&view, ROWS as usize);
        // Mid-run, because adjacent rows SHARE the bucket their boundary falls
        // in — a bucket at the edge would be read by two rows and the claim
        // below would be false of the picture rather than of the code.
        let run = run_of(&read, ROW);
        let bucket = run.start + run.len() / 2;
        let reached: Vec<u32> = (0..ROWS).filter(|&r| run_of(&read, r).contains(&bucket)).collect();
        assert_eq!(reached, [ROW], "bucket {bucket} is read by rows {reached:?}");

        let mut bytes = vec![0u8; W as usize * SPECTRUM_BINS];
        bytes[SLAB as usize * SPECTRUM_BINS + bucket] = q(1.0);
        let size = [W, ROWS];
        let frame = headless.frame(
            0,
            size,
            texel_quad(&read, W, size),
            grid_of(bytes),
            read.clone(),
            probe_shades(),
        );
        let lit = pixel(&frame, size, SLAB, ROW)[0];
        assert!(lit > 0, "the loud texel drew nothing");
        for py in 0..ROWS {
            for px in 0..W {
                if (px, py) == (SLAB, ROW) {
                    continue;
                }
                assert_eq!(
                    pixel(&frame, size, px, py)[0],
                    0,
                    "slab {px} row {py} carries energy that belongs at slab {SLAB} row {ROW}",
                );
            }
        }
    }

    /// The picture's noise floor HOLDS STILL as the pitch axis zooms out,
    /// instead of climbing with the number of buckets a pixel happens to
    /// span.
    ///
    /// Read by MAX, a pixel asks "how large was the largest of N draws",
    /// whose answer grows like the log of N and so has no limit: the floor
    /// between the partials reads brighter the further out the zoom, which
    /// is a statement about the layout rather than about the sound. The
    /// resample answers with the floor's own mean instead, at every N — see
    /// [`footprint_mean_db`](crate::panes::spectral::spectrogram::footprint_mean_db).
    ///
    /// Both widths are in the MINIFYING arm — 8 buckets to a row and 64 —
    /// because that is the arm at issue: a row narrower than a bucket reads
    /// between two of them and has no run whose width could lift it. The
    /// mean of dB has no step between the two widths at all, where a max
    /// has one that never stops.
    ///
    /// The MAX control is what gives the first assertion its teeth: it fails if
    /// the noise is too flat, or the runs too short, for either rule to be
    /// tested. It is arithmetic on the same bytes rather than a second frame,
    /// because the shader has no max arm to render.
    #[test]
    fn the_noise_floor_settles_as_the_pitch_axis_zooms_out() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        // Exponentially distributed power around -60 dB: the distribution a
        // noise floor's buckets have, and the one whose maximum keeps growing.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut power = [0.0f32; SPECTRUM_BINS];
        for p in power.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let u = ((seed >> 11) as f64 / (1u64 << 53) as f64).clamp(1e-12, 1.0);
            *p = (-u.ln() * 1e-6) as f32;
        }
        let column: Vec<u8> = power.iter().map(|&p| q(p)).collect();
        let view = view_of(whole_axis(), SpectrumConfig::default());

        // Every slab the same column, so the pane mean is the row read's own
        // and the time axis contributes nothing to it.
        let bytes: Vec<u8> = (0..W).flat_map(|_| column.iter().copied()).collect();
        let floor = |per_row: usize, headless: &mut SpectrogramHeadless| -> (f64, f64) {
            let rows = (SPECTRUM_BINS / per_row) as u32;
            let read = probe_read(&view, rows as usize);
            let runs: Vec<std::ops::Range<usize>> = (0..rows).map(|r| run_of(&read, r)).collect();
            let widest = runs.iter().map(|r| r.len()).max().expect("a row");
            assert!(
                runs.iter().all(|r| r.len() >= 2) && widest <= per_row + 2,
                "rows of {per_row} buckets came out {}..{widest} wide",
                runs.iter().map(|r| r.len()).min().expect("a row"),
            );
            let size = [W, rows];
            let frame = headless.frame(
                0,
                size,
                texel_quad(&read, W, size),
                grid_of(bytes.clone()),
                read.clone(),
                probe_shades(),
            );
            let mean = frame.as_chunks::<4>().0.iter().map(|p| f64::from(p[0])).sum::<f64>()
                / (W * rows) as f64;
            // The same rows read by MAX, which is what the mean is being
            // measured against.
            let by_max = runs
                .iter()
                .map(|r| f64::from(column[r.clone()].iter().copied().max().expect("a bucket")))
                .sum::<f64>()
                / rows as f64;
            (mean, by_max)
        };
        let step = f64::from(harmonigraph_core::spectrogram::DB_STEP);
        let (mean_8, max_8) = floor(8, &mut headless);
        let (mean_64, max_64) = floor(64, &mut headless);
        let climb = (mean_64 - mean_8) * step;
        assert!(climb < 0.1, "the floor moved {climb:.2} dB across a three-octave zoom");
        let by_max = (max_64 - max_8) * step;
        assert!(by_max > 2.2, "a plain MAX climbed only {by_max:.2} dB, so this proves nothing");
    }

    /// A ramp of stored bytes renders dark to bright.
    ///
    /// Every step between the store and the screen is monotone on its own — the
    /// read, the level affine, the table — and the picture's whole grammar is
    /// that brighter means louder, so a sign or an index flipped anywhere in
    /// the chain is a heatmap that reads backwards while every part of it looks
    /// right.
    ///
    /// Lightness in `L*`, which is the units the gradient is authored in: a
    /// channel sum weights blue like green and would call a violet and a yellow
    /// of one sum equally bright. Opacity is not measurable here and stays with
    /// `cells_are_opaque_and_run_dark_to_bright` — the pane's bed is opaque and
    /// egui's blend leaves the target's alpha at 1 whatever a fragment writes.
    #[test]
    fn a_ramp_of_stored_bytes_renders_dark_to_bright() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        // One row over the whole axis and a constant column per slab, so a
        // slab's texel is that slab's byte and the ramp is the frame.
        let size = [W, 1];
        let bytes: Vec<u8> = (0..W).flat_map(|j| vec![(j * 4) as u8; SPECTRUM_BINS]).collect();
        for preset in crate::SpectrogramPreset::ALL {
            let cfg =
                SpectrumConfig { spectrogram_gradient: preset.gradient(), ..Default::default() };
            let view = view_of(whole_axis(), cfg);
            let read = read_of(&view, 1);
            let frame = headless.frame(
                0,
                size,
                texel_quad(&read, W, size),
                grid_of(bytes.clone()),
                read,
                FoldedGrid::default().shades(&cfg),
            );
            let lightness = |px: u32| {
                let c = pixel(&frame, size, px, 0);
                let v = |b: u8| f64::from(b) / 255.0;
                harmonigraph_scene::color::lightness_of_encoded(v(c[0]), v(c[1]), v(c[2]))
            };
            for px in 1..W {
                assert!(
                    lightness(px) >= lightness(px - 1) - 1e-9,
                    "{preset:?}: byte {} draws darker than byte {}",
                    px * 4,
                    (px - 1) * 4,
                );
            }
            assert!(lightness(0) < 0.5, "{preset:?}: silence must sit on the ramp's black end");
            assert!(lightness(W - 1) > lightness(0) + 0.2, "{preset:?}: the ramp barely moved");
        }
    }

    /// The quiet end of the ramp FADES toward black rather than falling off a
    /// cliff into it.
    ///
    /// A shortcut answering everything under some dB as silence is invisible
    /// while the Level window bottoms out above it, and becomes a hard edge —
    /// faintest colour straight to black — the moment the window can be dragged
    /// below. The control is the same bucket at the default window, where it
    /// really is under the floor: without it, a picture pinned to the darkest
    /// LUT slice everywhere would pass the first half.
    #[test]
    fn a_bucket_above_a_dragged_down_floor_still_draws_a_colour() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        let size = [W, 1];
        let quiet = q(1e-9); // -90 dB
        let bytes = vec![quiet; W as usize * SPECTRUM_BINS];
        let drawn = |cfg: SpectrumConfig, headless: &mut SpectrogramHeadless| {
            let view = view_of(whole_axis(), cfg);
            let read = read_of(&view, 1);
            let frame = headless.frame(
                0,
                size,
                texel_quad(&read, W, size),
                grid_of(bytes.clone()),
                read,
                FoldedGrid::default().shades(&cfg),
            );
            pixel(&frame, size, 0, 0)
        };
        // Hold the palette and pitch tilt fixed while probing the
        // level-window mapping.
        let default = SpectrumConfig {
            spectrogram_gradient: crate::SpectrogramPreset::Aurora.gradient(),
            volume_ceiling_db: 0.0,
            tilt: 0.0,
            ..SpectrumConfig::default()
        };
        let darkest = crate::panes::spectral::spectrogram::cell_color(
            default.spectrogram_gradient,
            0.5 / SHADES as f32,
        )
        .to_array();
        assert_eq!(
            drawn(default, &mut headless)[..3],
            darkest[..3],
            "a -90 dB bucket under the default floor missed the darkest LUT slice",
        );
        let dragged = SpectrumConfig { volume_floor_db: -120.0, ..default };
        let lit = drawn(dragged, &mut headless);
        assert_ne!(
            lit[..3],
            darkest[..3],
            "a -90 dB bucket 30 dB above a -120 dB floor was cut off instead of faded",
        );
    }

    /// The curve and the heatmap read one footprint of buckets the same way.
    ///
    /// They hold their buckets differently — the curve as floats of power in
    /// Rust, the heatmap as bytes of dB in WGSL — so the operator is written
    /// twice, and two forms of one definition drift. This is the only thing
    /// holding the shader to the Rust, and what the drift costs is visible
    /// rather than subtle: a pixel of the curve and a pixel of the heatmap
    /// cover the SAME buckets, and the pane draws both from one gradient
    /// through one loudness mapping precisely so equal levels read equal, so
    /// a disagreement puts a ridge and the curve over it at different
    /// heights on one tone.
    ///
    /// The tolerance is the store's own, not a fudge: the heatmap's side
    /// quantizes each bucket to
    /// [`DB_STEP`](harmonigraph_core::spectrogram::DB_STEP) where the
    /// curve's holds the power, and the table it then lands in is sampled at
    /// [`SHADES`] levels.
    ///
    /// The dB window is dialled wide enough to hold the whole fixture,
    /// which is what makes the two forms one claim. The heatmap resamples
    /// the CLAMPED level image — a bucket off either end of the ramp
    /// contributes the end it is off, which is what keeps a partial
    /// brighter than the window's ceiling from dragging its pixel dark —
    /// and the curve answers in powers, where there is no ramp to be off.
    /// Inside the window the clamp is the identity and the two are the same
    /// mean under one affine map, which is the claim being made.
    #[test]
    fn the_curve_and_the_heatmap_read_a_run_of_buckets_alike() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        let cfg = SpectrumConfig {
            volume_floor_db: -140.0,
            volume_ceiling_db: 20.0,
            ..SpectrumConfig::default()
        };
        let lut = FoldedGrid::default().shades(&cfg).lut;
        let step = harmonigraph_core::spectrogram::DB_STEP;
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };

        // One row over the whole picture, so the frame IS that row's read,
        // once per slab. At one row the footprint IS the visible range, so
        // the range alone decides how many buckets it covers: a span of `k`
        // buckets straddles `k + 1` of them, and the half bucket keeps both
        // ends off a boundary, where a tie would decide the count instead.
        // That reaches every run length from a pair to a full 32, which the
        // assertion at the end holds this to rather than trusting it.
        let spans: Vec<f32> =
            (1..=31).map(|k| (k as f32 + 0.5) / BINS_PER_SEMITONE as f32).collect();
        let size = [W, 1];
        let mut lengths = std::collections::BTreeSet::new();
        for span in spans {
            let view = view_of(
                PitchScale { min_midi: SPECTRUM_MIN_MIDI + 20.0, max_midi: 0.0, span },
                cfg,
            );
            let read = read_of(&view, 1);
            let run = run_of(&read, 0);
            lengths.insert(run.len());
            let foot = foot_of(&read, 0);
            let midi = foot.midi;
            // The footprint in BUCKET coordinates, which is the argument the
            // curve's form of the operator takes.
            let x = |t: f32| {
                (read.min_midi + t * read.span - read.spectrum_min_midi) * read.bins_per_semitone
            };
            let (x0, x1) = (x(foot.lo_t), x(foot.hi_t));
            for _ in 0..4 {
                // A case per slab: 90 dB of spread, which is where two forms
                // of one mean have the most room to disagree — a flat run
                // agrees trivially.
                let powers: Vec<Vec<f32>> = (0..W)
                    .map(|_| (0..run.len()).map(|_| 10f64.powf(-9.0 * next()) as f32).collect())
                    .collect();
                let mut bytes = vec![0u8; W as usize * SPECTRUM_BINS];
                let mut spectra = vec![vec![0.0f32; SPECTRUM_BINS]; W as usize];
                for (j, ps) in powers.iter().enumerate() {
                    for (b, &p) in run.clone().zip(ps.iter()) {
                        bytes[j * SPECTRUM_BINS + b] = q(p);
                        spectra[j][b] = p;
                    }
                }
                let frame = headless.frame(
                    0,
                    size,
                    texel_quad(&read, W, size),
                    grid_of(bytes),
                    read.clone(),
                    SpectrogramShades { lut: lut.clone() },
                );
                for (j, spectrum) in spectra.iter().enumerate() {
                    let curve = footprint_mean_db(spectrum, x0, x1);
                    // The band the store's own rounding leaves the heatmap
                    // free to land in, widened by the one table index a level
                    // sitting on a slice boundary can fall either side of.
                    let index = |db: f32| {
                        let level = spectrogram_level_db(&cfg, db, midi);
                        ((level * SHADES as f32) as usize).min(SHADES - 1)
                    };
                    let lo = index(curve - 2.0 * step).saturating_sub(1);
                    let hi = (index(curve + 2.0 * step) + 1).min(SHADES - 1);
                    let got = pixel(&frame, size, j as u32, 0);
                    assert!(
                        (lo..=hi).any(|i| lut[i][..3] == got[..3]),
                        "a run of {} at {curve:.3} dB drew {got:?}, outside the table's \
                         {lo}..={hi} ({:?} to {:?})",
                        run.len(),
                        lut[lo],
                        lut[hi],
                    );
                }
            }
        }
        // The sweep really did reach every run length it claims to.
        assert_eq!(
            lengths.iter().copied().collect::<Vec<_>>(),
            (2..=32).collect::<Vec<_>>(),
            "the spans reached run lengths {lengths:?}",
        );
    }

    /// A pane resize re-sends the whole heatmap a handful of times, not
    /// once per frame.
    ///
    /// The ring's slot for a key is `key mod capacity`, so every change of
    /// capacity moves the whole mapping: the buffer is reallocated and
    /// every slab in it re-sent. Sized straight off the pane's pixels that
    /// is megabytes a frame for as long as a horizontal drag lasts, since
    /// the depth axis moves by a pixel or more per frame of one. The ring is
    /// one size for every pane ([`RING_SLOTS`]), so what re-sends the grid
    /// is [`live_slab`]'s rung stepping, once per rung the drag crosses.
    ///
    /// Count actual renderer writes; preparing CPU data alone cannot report them.
    #[test]
    fn a_pane_resize_does_not_re_send_the_grid_every_frame() {
        let Some(mut headless) = SpectrogramHeadless::new() else {
            return;
        };
        const SIZE: [u32; 2] = [64, 64];
        let interval = crate::spectrogram::FFT_INTERVAL;
        let cfg = SpectrumConfig::default();
        // A Span deep enough to sit well up the ladder. Both ladders step
        // on the travel below — `live_slab` reads `target_cols` too — and
        // the budget at the bottom counts the crossings of each.
        let window = 180.0;
        let mut spectrum = crate::AudioSpectrum::default();
        let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
        let mut clock = 0.0f64;
        let mut n = 0usize;
        let push = |spectrum: &mut crate::AudioSpectrum, clock: &mut f64, n: &mut usize| {
            let mut bins = [0.02f32; SPECTRUM_BINS];
            bins[400 + (*n * 13) % 3000] = 1.0;
            spectrum.push_history(*clock, &bins);
            *clock += interval;
            *n += 1;
        };
        while clock < window + 5.0 {
            push(&mut spectrum, &mut clock, &mut n);
        }

        // One frame at a pane this wide, with the columns a live stream
        // delivers between two frames.
        let frame = |spectrum: &mut crate::AudioSpectrum,
                     surfaces: &mut crate::spectrogram::SpectrogramSurfaces,
                     headless: &mut SpectrogramHeadless,
                     clock: &mut f64,
                     n: &mut usize,
                     depth_len: f32| {
            for _ in 0..2 {
                push(spectrum, clock, n);
            }
            let view = PaneView {
                ppp: 2.0,
                pitch_len: 400.0,
                depth_len,
                window,
                scale: whole_axis(),
                cfg,
            };
            let plan = Plan::new(&view, *clock - window, None);
            run_for(spectrum.pyramid(), surfaces, 0, &plan).expect("a run to draw");
            let (grid, shades) = frame_data(surfaces, 0, &cfg).expect("a grid to draw");
            let slabs = grid.run.len() as u32;
            let read = read_of(&view, plan.rows);
            headless.frame(0, SIZE, run_quad(slabs, SIZE), grid, read, shades);
        };

        // Settle, so the run the drag starts from is one the GPU has
        // uploaded rather than the first fold of all.
        for _ in 0..4 {
            frame(&mut spectrum, &mut surfaces, &mut headless, &mut clock, &mut n, 100.0);
        }
        let was = surfaces.at(0).folded.full_uploads();
        // A point a frame, which is the rate a hand delivers a drag at, over
        // travel that takes the depth axis from a sliver to past the cap.
        const TRAVEL: u32 = 400;
        for step in 0..TRAVEL {
            let depth_len = 100.0 + step as f32;
            frame(&mut spectrum, &mut surfaces, &mut headless, &mut clock, &mut n, depth_len);
        }
        let sent = surfaces.at(0).folded.full_uploads() - was;
        // The ring is one size, so what re-sends is `live_slab` stepping: this
        // travel crosses two of its rungs.
        assert!(sent <= 2, "a {TRAVEL}-frame drag re-sent the whole grid {sent} times");
    }
}

/// A stream that stops draws its columns on as they scroll away, then nothing
/// once the window has passed the last of them, until columns resume.
#[test]
fn a_stopped_stream_draws_nothing_until_columns_resume() {
    let mut spectrum = crate::AudioSpectrum::default();
    let mut surfaces = crate::spectrogram::SpectrogramSurfaces::default();
    let bins = [0.5; SPECTRUM_BINS];
    let view = PaneView {
        ppp: 1.0,
        pitch_len: 64.0,
        depth_len: 256.0,
        window: 4.0,
        scale: SWEEP_SCALE,
        cfg: SpectrumConfig::default(),
    };
    let mut draws = |spectrum: &mut crate::AudioSpectrum, now: f64| {
        let plan = Plan::new(&view, now - view.window, None);
        run_for(spectrum.pyramid(), &mut surfaces, 0, &plan).is_some()
    };
    for i in 0..500 {
        spectrum.push_history(100.0 + i as f64 * 0.008, &bins);
    }
    assert!(draws(&mut spectrum, 104.0), "a streaming history drew nothing");
    assert!(draws(&mut spectrum, 106.0), "the stopped stream's columns did not scroll on");
    assert!(!draws(&mut spectrum, 108.1), "the window passed the last column and still drew");
    for i in 0..3 {
        spectrum.push_history(110.0 + i as f64 * 0.008, &bins);
    }
    assert!(draws(&mut spectrum, 110.03), "resumed columns drew nothing");
}
