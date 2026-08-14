//! The Spectral pane's spectrogram: a frequency-vs-time heatmap of the
//! analyzed audio, drawn in the roll's depth region on the roll's own time
//! axis. A column of spectral energy therefore lines up with the note
//! ribbons that made it — the same pitch axis across, the same `now`-anchored
//! time along, so what you hear and what you played read against each other.
//!
//! It's a layer under the roll, not a pane. The heatmap is built into a small
//! image — one pixel per (time slab, pitch pixel) — uploaded as a texture and
//! sampled with bilinear filtering, so it reads as one smooth, filled image
//! rather than a mesh of flat cells (which looked blocky) or interpolated
//! triangles (which floated and creased). Geometry still comes from
//! [`Axes`], so it turns and flips with the pane, and
//! its dB intensity scale is shared with the spectrum curve via
//! [`loudness`](super::axes::loudness) so "loud" means the same in both.

use egui::Color32;

use super::axes::{Axes, PitchScale, TimeAxis};
use crate::spectrogram::{bins_for, build, hold_time, u_drawn, Columns, PaneView, Plan, TexLayout};
use crate::SharedState;
use harmonigraph_scene::Gradient;

/// Order of the power mean a row takes over the run of buckets under it — how
/// hard that read leans toward the loudest of them. See
/// [`RowRead::Mean`](crate::spectrogram::RowRead).
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
/// form of the read [`RowRead::Mean`](crate::spectrogram::RowRead) performs on
/// the heatmap's stored dB bytes.
///
/// Two implementations of one definition rather than one shared function,
/// because the two callers hold their buckets differently and each form is the
/// cheap one where it lives: the heatmap's are bytes of dB, where the mean is a
/// table lookup and a sum, and the curve's are floats of power, where it is
/// this. `the_curve_and_the_heatmap_read_a_run_of_buckets_alike` is what keeps
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

/// The scrolling quads over the uploaded texture, from `d_near` to `d_far`.
///
/// Pure geometry, which is what lets the UV rule below be checked at all: these
/// are VERTEX UVs, so the scale every fragment samples at is interpolated
/// between a quad's corners.
///
/// Map a screen depth to the texture's time axis CONTINUOUSLY, through the
/// roll's own `now`-anchored depth<->time relation (unclamped, the inverse of
/// `depth_of`), so `u` slides with `now` frame to frame exactly as a note ribbon
/// at the same depth does. Pinning it to the slab endpoints instead jumps a
/// whole slab at a time, which is both the image losing the notes it is meant to
/// register with and a per-slab stutter. `v` maps pitch to the bin rows.
///
/// Slabs occupy texels `x0 .. x0 + visible` of a `tex_w`-wide texture; for a
/// full-width build that is the whole thing and this collapses to the plain
/// `(t - t_origin) / tex_span`.
fn heatmap_mesh(
    tex: egui::TextureId,
    axes: &Axes,
    time: &TimeAxis,
    layout: &TexLayout,
    d_near: f32,
    d_far: f32,
) -> egui::Mesh {
    // Straight in time out to the newest column the texture holds, then HELD
    // there — see [`u_drawn`], and the mesh is split at that corner so no
    // fragment ever interpolates across it.
    let u_at = |d: f32| u_drawn(layout, time.time_at(d));
    let v_at = |p: f32| (p - layout.t0) / (layout.tn - layout.t0);
    // Untinted: the texels carry the whole of the colour.
    //
    // A tint here is how an Opacity setting fades the heatmap so it can sit
    // under the notes, and what that costs is the SHARED SCHEME. The spectrum
    // curve is drawn from the same [`cell_color`] ramp against the same
    // `loudness_db`, and it takes no tint — so a faded heatmap means equal
    // levels stop looking equal across the two halves of one pane, which is the
    // whole reason they share a mapping. A heatmap worth less than solid is one
    // to turn off.
    //
    // What a tint does NOT cost is the ramp's dark end: `Color32` is
    // premultiplied, so a black texel over the black bed composites to black at
    // any alpha (`spectral_pane` lays that bed and leans on the same fact).
    let tint = Color32::WHITE;
    let vert = |p: f32, d: f32| egui::epaint::Vertex {
        pos: axes.at(p, d),
        uv: egui::pos2(u_at(d), v_at(p)),
        color: tint,
    };

    // Quads over pitch [0,1] x a depth span; the GPU bilinear-samples them.
    let mut mesh = egui::Mesh::with_texture(tex);
    let quad = |mesh: &mut egui::Mesh, near: f32, far: f32| {
        let i = mesh.vertices.len() as u32;
        mesh.vertices.push(vert(0.0, far)); // far, low
        mesh.vertices.push(vert(1.0, far)); // far, high
        mesh.vertices.push(vert(1.0, near)); // near, high
        mesh.vertices.push(vert(0.0, near)); // near, low
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };

    // Split at the corner in `u_drawn`: one quad whose UVs are straight in time
    // (the data) and one whose UVs are CONSTANT (the sliver past the newest
    // column — leading for the live window, trailing for the whole-song build,
    // which is the only reason `d_hold` is clamped into the pair rather than
    // assumed to sit inside it).
    //
    // Letting one quad span the corner instead is what the vertex-UV rule
    // forbids: a vertex sitting mid-bend rescales the whole image, and the bend
    // crosses each slab, so it would rescale it once per slab — the jitter.
    // Split here, the data quad's two ends are both straight in time, so it
    // holds exactly one texel per slab forever; all that changes over a slab is
    // how long the flat sliver is, and its colour matches the data at the seam
    // (both sample the newest texel's centre), so the join is invisible.
    let d_hold = time.depth_of(hold_time(layout)).max(d_near).min(d_far);
    if d_hold > d_near {
        quad(&mut mesh, d_near, d_hold);
    }
    quad(&mut mesh, d_hold, d_far);
    mesh
}

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// Builds the heatmap into a `[time slab x pitch bin]` image, (re)uploads it
/// to the surface's texture, then stretches it over the region as a
/// single bilinear-filtered quad — smooth in both axes, and opaque (silence is
/// the ramp's dark end, not transparent) so the plane is a filled image rather
/// than bright patches floating on the background.
pub(crate) fn draw_spectrogram(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut SharedState,
    split: f32,
    now: f64,
    // Which texture slot to build into (0 the docked pane / offline render, 1
    // the Render preview) — two live spectrograms in a frame need their own.
    surface: usize,
) {
    // A small copy, so `state.spectrum` is then free to take mutably (its
    // texture handle) without fighting the config reads.
    let cfg = state.spectrum_config;
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

    let mut view = PaneView {
        ppp: painter.ctx().pixels_per_point().max(1.0),
        max_rows: painter.ctx().input(|i| i.max_texture_side).max(64),
        pitch_len: axes.pitch_len(),
        depth_len: time.region_depth_len(axes),
        window: time.window(),
        scale: *scale,
        cfg,
        whole: whole.is_some(),
        coarse: false,
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
    let mut plan = Plan::new(&view, &columns);

    // While the style is moving — a pitch wheel, a Level drag, a palette bar,
    // the Span crossing a slab rung — every frame restarts the ring, so the
    // frame builds the COARSE image instead: rows capped and stretched over
    // the same pane, and wide rows reading a plain max (see
    // [`crate::spectrogram::GESTURE_ROWS`] and
    // [`RowRead::Max`](crate::spectrogram::RowRead)). One build at full
    // quality sharpens it once the style has held still — a frame nothing
    // else schedules when no audio is flowing and the pointer has let go, so
    // it is requested here. Whole-song is out: its one style change per
    // config edit is already cached after a frame, and an offline render must
    // never trade resolution away.
    if !view.whole
        && crate::spectrogram::StyleMotion::observe(
            &mut spectrum.spectrogram[surface].motion,
            plan.key.style(),
            painter.ctx().input(|i| i.time),
        )
    {
        painter.ctx().request_repaint_after(std::time::Duration::from_secs_f64(
            crate::spectrogram::STYLE_SETTLE,
        ));
        view.coarse = true;
        view.max_rows = view.max_rows.min(crate::spectrogram::GESTURE_ROWS);
        plan = Plan::new(&view, &columns);
    }

    // Fast path: the built image is still valid — reuse the uploaded texture and
    // its geometry; only the scrolling quad below is recomputed (with `now`).
    let surf = &spectrum.spectrogram[surface];
    let reused = match &surf.cache {
        Some(c) if c.matches(&plan.key) && surf.tex.is_some() => Some(c.geometry()),
        _ => None,
    };
    let layout = match reused {
        Some(layout) => layout,
        None => {
            let bins = bins_for(plan.rows, scale, view.coarse);
            match build(painter.ctx(), spectrum, whole, surface, &plan, &view, &bins) {
                Some(layout) => layout,
                None => return,
            }
        }
    };

    let Some(tex) = &spectrum.spectrogram[surface].tex else { return };

    // The quad only spans the depths the data actually reaches, so the drawn
    // strip GROWS from the now-line as history accumulates rather than being
    // stretched to fill the whole region. Without the far cap, clearing the
    // spectrogram (or startup) left a handful of fresh columns ClampToEdge-
    // smeared across everything as trails.
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

    let mesh = heatmap_mesh(tex.id(), axes, &time, &layout, d_near, d_far);
    painter.add(egui::Shape::mesh(mesh));
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
