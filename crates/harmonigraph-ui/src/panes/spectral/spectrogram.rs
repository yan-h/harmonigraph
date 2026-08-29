//! The Spectral pane's spectrogram: a frequency-vs-time heatmap of the
//! analyzed audio, drawn in the roll's depth region on the roll's own time
//! axis. A column of spectral energy therefore lines up with the note
//! ribbons that made it — the same pitch axis across, the same `now`-anchored
//! time along, so what you hear and what you played read against each other.
//!
//! It's a layer under the roll, not a pane. The heatmap is a handful of quads
//! whose fragments read the aggregator's slab grid themselves (see
//! [`harmonigraph_render::spectrogram_paint_callback`]), filtered across both
//! axes so it reads as one smooth, filled image rather than a mesh of flat
//! cells (which looked blocky) or interpolated triangles (which floated and
//! creased). Geometry still comes from [`Axes`], so it turns and flips with the
//! pane, and its dB intensity scale is shared with the spectrum curve via the
//! volume-color mapping, so "loud" means the same color in both while their
//! analyzer geometry remains independent.

use egui::Color32;
use harmonigraph_render::SpectrogramVertex;

use super::axes::{Axes, PitchScale, TimeAxis};
use crate::spectrogram::{
    frame_data, hold_time, read_of, run_for, slab_drawn, Columns, PaneView, Plan, TexLayout,
};
use crate::SharedState;
use harmonigraph_scene::Gradient;

/// Order of the power mean a row takes over the run of buckets under it — how
/// hard that read leans toward the loudest of them. Uploaded as
/// [`SpectrogramRead::mean_steps`](harmonigraph_render::SpectrogramRead), which
/// is the form the fragment shader reads it in.
///
/// A plain MAX is the obvious read, and it makes the picture's NOISE FLOOR a
/// function of the ZOOM. Zoomed out a row covers a dozen-odd buckets, and the
/// largest of a dozen samples of a fluctuating floor sits well above their
/// typical value — 4.7 dB for the exponentially distributed power a noise floor
/// has, against 0 dB for a row reading a single bucket. So the same passage
/// reads brighter between its partials the further out it is zoomed, and the
/// brightness is a statement about how wide the pane is rather than about the
/// sound. It climbs without limit, too: the excess goes as the log of the run,
/// so there is no zoom at which it settles.
///
/// A power mean estimates a fixed property of the distribution instead, so it
/// CONVERGES as the run widens rather than climbing with it. The order sets
/// where it sits between the plain mean (1) and the max (infinity). Four is high
/// enough to keep a partial reading as a partial — the analysis lobe is many
/// buckets wide wherever the axis outruns the FFT, so a lobe fills its row
/// rather than being diluted in it — and low enough to settle quickly, landing
/// 3.4 dB over a noise floor's mean and staying there.
///
/// What it costs is absolute level on anything NARROWER than its row, which up
/// near the top of the axis a lobe can be: alone in a run of ten buckets, it
/// reads 2.5 dB down. That is a trade rather than a win. For a lobe WIDER than
/// its row — which is most of the axis, since the analysis lobe spans many
/// buckets wherever the axis outruns the FFT — the lobe reads unchanged and the
/// floor drops away beneath it, so contrast improves by 1.3 dB over a run of
/// four buckets and 1.8 over eight. For a narrow one both come down together.
/// What does not vary either way is the zoom.
///
/// Read by the CURVE as well as by the heatmap, through [`power_mean`] — the two
/// halves of the pane draw one measurement two ways, and a pixel of each covers
/// the same run of buckets, so a run that read differently between them would
/// put a ridge and the curve over it at different heights.
pub(crate) const ROW_MEAN_ORDER: i32 = 4;

/// The power mean of order [`ROW_MEAN_ORDER`] over a run of POWERS — the curve's
/// form of the read the heatmap's fragment shader performs on stored dB bytes.
///
/// Two implementations of one definition rather than one shared function,
/// because the two callers hold their buckets differently and each form is the
/// cheap one where it lives: the heatmap's are bytes of dB in a storage buffer,
/// where the mean is a table lookup and a sum, and the curve's are floats of
/// power, where it is this. `the_curve_and_the_heatmap_read_a_run_of_buckets_alike` is what keeps
/// the pair honest.
///
/// Denominated against the run's own loudest, exactly as the table is, and for
/// the same reason: a raw fourth power of an absolute power underflows an `f32`
/// long before the axis runs out of quiet buckets.
pub(crate) fn power_mean(run: &[f32]) -> f32 {
    let top = run.iter().fold(0.0f32, |a, &b| a.max(b));
    if run.len() < 2 || top <= 0.0 {
        return top;
    }
    let sum: f32 = run.iter().map(|&p| (p / top).powi(ROW_MEAN_ORDER)).sum();
    top * (sum / run.len() as f32).powf(1.0 / ROW_MEAN_ORDER as f32)
}

/// The scrolling quads the heatmap is read through, from `d_near` to `d_far`.
///
/// Pure geometry, which is what lets the interpolation rule below be checked at
/// all: these are VERTEX attributes, so the scale every fragment reads at is
/// interpolated between a quad's corners.
///
/// Map a screen depth to the run's slab axis CONTINUOUSLY, through the roll's
/// own `now`-anchored depth<->time relation (unclamped, the inverse of
/// `depth_of`), so a fragment slides with `now` frame to frame exactly as a note
/// ribbon at the same depth does. Pinning it to the slab endpoints instead jumps
/// a whole slab at a time, which is both the picture losing the notes it is
/// meant to register with and a per-slab stutter. `t` is the plain pitch
/// fraction; the row geometry it maps through is the shader's.
pub(super) fn heatmap_vertices(
    axes: &Axes,
    time: &TimeAxis,
    layout: &TexLayout,
    d_near: f32,
    d_far: f32,
) -> Vec<SpectrogramVertex> {
    // Straight in time out to the newest slab the run holds, then HELD there —
    // see [`slab_drawn`], and the mesh is split at that corner so no fragment
    // ever interpolates across it.
    let vert = |p: f32, d: f32| {
        let pos = axes.at(p, d);
        SpectrogramVertex { pos: [pos.x, pos.y], slab: slab_drawn(layout, time.time_at(d)), t: p }
    };

    // Quads over pitch [0,1] x a depth span, as a triangle list.
    let mut vertices = Vec::with_capacity(12);
    let mut quad = |near: f32, far: f32| {
        let (low_far, high_far) = (vert(0.0, far), vert(1.0, far));
        let (high_near, low_near) = (vert(1.0, near), vert(0.0, near));
        vertices.extend([low_far, high_far, high_near, low_far, high_near, low_near]);
    };

    // Split at the corner in `slab_drawn`: one quad whose slab coordinate is
    // straight in time (the data) and one whose coordinate is CONSTANT (the
    // sliver past the newest slab — leading for the live window, trailing for
    // the whole-song build, which is the only reason `d_hold` is clamped into
    // the pair rather than assumed to sit inside it).
    //
    // Letting one quad span the corner instead is what the vertex rule forbids:
    // a vertex sitting mid-bend rescales the whole picture, and the bend crosses
    // each slab, so it would rescale it once per slab — the jitter. Split here,
    // the data quad's two ends are both straight in time, so it holds exactly
    // one slab per slab forever; all that changes over a slab is how long the
    // flat sliver is, and its colour matches the data at the seam (both read the
    // newest slab's centre), so the join is invisible.
    let d_hold = time.depth_of(hold_time(layout)).max(d_near).min(d_far);
    if d_hold > d_near {
        quad(d_near, d_hold);
    }
    quad(d_hold, d_far);
    vertices
}

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// Folds the window into a run of slabs, hands that run and the scalars a row is
/// read through to the GPU, and stretches a pair of quads over the region —
/// smooth in both axes, and opaque so the plane is a filled image rather than
/// bright patches floating on the background.
///
/// Opaque and untinted, which is a decision about the SHARED SCHEME rather than
/// an implementation detail: an Opacity setting would fade the heatmap so it can
/// sit under the notes, and the spectrum curve is drawn from the same
/// [`cell_color`] ramp against the same `loudness_db` and takes no tint — so a
/// faded heatmap means equal levels stop looking equal across the two halves of
/// one pane. A heatmap worth less than solid is one to turn off.
pub(crate) fn draw_spectrogram(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut SharedState,
    split: f32,
    now: f64,
    // Which surface this is (0 the docked pane / offline render, 1 the Render
    // preview) — two live spectrograms in a frame need their own grid.
    surface: usize,
) {
    // Small copies, so `state.spectrum` is then free to take mutably without
    // fighting the config reads.
    let cfg = state.spectrum_config;
    let target_format = state.target_format;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let whole = state.whole_song.as_ref();
    let spectrum = &mut state.spectrum;
    // Columns come from the precomputed whole-take set (playhead mode) or the
    // live store.
    let enough = match whole {
        Some(ws) => ws.columns.len() >= 2,
        None => spectrum.history().len() >= 2,
    };
    if !enough {
        return;
    }

    let view = PaneView {
        ppp: painter.ctx().pixels_per_point().max(1.0),
        pitch_len: axes.pitch_len(),
        depth_len: time.region_depth_len(axes),
        window: time.window(),
        scale: *scale,
        cfg,
        whole: whole.is_some(),
    };
    let columns = match whole {
        Some(ws) => Columns {
            first: 0,
            len: ws.columns.len(),
            newest: ws.columns.last().map_or(now, |c| c.time),
        },
        None => {
            let hist = spectrum.history();
            Columns {
                first: hist.partition_point(|c| c.time < time.oldest()).saturating_sub(1),
                len: hist.len(),
                newest: hist.back().map_or(now, |c| c.time),
            }
        }
    };
    let plan = Plan::new(&view, &columns);

    // The run on the GPU when the plan's key still names it, and a fresh fold
    // otherwise. The pitch axis, the rows and the colours are uniforms, so a
    // zoom, a resize or a palette drag reaches neither.
    let Some(layout) = run_for(spectrum, whole, surface, &plan, &view) else {
        return;
    };

    // The quad only spans the depths the data actually reaches, so the drawn
    // strip GROWS from the now-line as history accumulates rather than being
    // stretched to fill the whole region. Without the far cap, clearing the
    // spectrogram (or startup) leaves a handful of fresh columns smeared across
    // everything as trails, by the clamp that holds the run's outermost slab.
    //
    // Near edge: to the split while fresh columns keep arriving, but stopping
    // at the newest data once it goes stale — most visibly when switching the
    // window algorithm, which empties the ring for a window's worth of samples
    // and would otherwise smear the last pre-switch slice over the growing gap.
    // The grace keeps the ordinary ~one-FFT lag from opening a flickering
    // sliver. Far edge: the oldest slab's depth, which is 1 once history spans
    // the window (depth_of clamps there) and nearer while it is still filling.
    let (d_near, d_far) = if time.whole_song() {
        // The whole take is present from the first frame, so the strip fills the
        // region edge to edge; only the playhead moves.
        (time.depth_of(layout.t_origin), time.depth_of(layout.t_origin + layout.tex_span))
    } else {
        // Plus the lag every healthy column has by construction: it is stamped
        // at the middle of the window it measured, so the newest one is always
        // half a window old. Without that term the strip stops short of the
        // now-line whenever the window is long, and the gap changes size with
        // the window setting.
        const FRESH: f64 = 0.12;
        let stale_after = FRESH + spectrum.column_lag();
        let near =
            if now - columns.newest <= stale_after { split } else { time.depth_of(columns.newest) };
        (near, time.depth_of(layout.t_origin))
    };

    let vertices = heatmap_vertices(axes, &time, &layout, d_near, d_far);
    let Some((grid, shades)) = frame_data(spectrum, surface, &cfg) else {
        return;
    };
    // The painter's own clip is what bounds the heatmap: the quads reach past
    // the pane wherever the strip does (the whole-song build's oldest slab
    // starts before the region), and the callback draws against the whole
    // surface and leaves the scissor egui set from this rect alone.
    painter.add(harmonigraph_render::spectrogram_paint_callback(
        painter.clip_rect(),
        vertices,
        grid,
        read_of(&view, plan.rows),
        shades,
        target_format,
        crate::panes::lattice::pane_id(surface),
    ));
}

/// A cell's opaque color: `level` (0..1 loudness) mapped through the heatmap's
/// gradient. The gradient's dark end is black at every preset, matching the
/// region's black bed (laid down in `spectral_pane`), so silence recedes while
/// energy stands out — and the quad is drawn untinted, so what a texel says is
/// what lands. Shared with the spectrum curve so the two read in the same
/// scheme.
///
/// Through the same table the lattice's own colors come off
/// ([`harmonigraph_scene::gradient_color`]), which is what lets the heatmap be
/// a gradient at all: a cell's color is otherwise a gamut bisection and a
/// Newton solve, and there are [`SHADES`](crate::spectrogram::SHADES) of them to
/// build per repaint and a curve's worth per frame.
pub(crate) fn cell_color(gradient: Gradient, level: f32) -> Color32 {
    let c = harmonigraph_scene::gradient_color(level, gradient);
    // The table is linear-interpolated between entries and the encode is
    // already done, so this is a straight quantization. ROUND and not truncate:
    // `as u8` floors, which drops up to a whole level of every interpolated
    // colour — a systematic darkening, on a ramp whose whole job is to read as
    // smooth.
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(byte(c.x), byte(c.y), byte(c.z))
}
