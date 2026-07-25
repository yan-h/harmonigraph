//! The Spectral pane's piano roll: incoming MIDI drawn as ribbons over
//! the same pitch axis the spectrum uses.
//!
//! Not a piano roll in the DAW sense — there are no black/white key lanes
//! and no bar grid, because the pitch axis is the *lattice's* axis: it is
//! continuous in cents, so a bent or microtonally tuned note sits between
//! the keys rather than being quantized onto one. What it keeps from the
//! DAW idea is the shape: pitch across, time along, one ribbon per note.
//!
//! Geometry comes entirely from [`Axes`](super::spectral::Axes), so the
//! roll turns with the rest of the pane and this file never names a screen
//! side. Its share of the depth axis runs from `split` (now) to 1 (the
//! oldest note still on screen), so time flows *away* from the spectrum
//! and a note crossing the split meets the peak it is making.

use egui::Color32;
use lattice_core::RollNote;
use lattice_render::{RollAxes, RollInstance};
use lattice_scene::channel_color;

use super::spectral::{Axes, PitchScale, TimeAxis};
use super::{scene_color, PITCH_RAMP_CHANNEL};
use crate::{theme, RollColor, SharedState};

/// Below this many pixels a ribbon is too thin to read as a shape, so it
/// is drawn as a bare line instead (which stays visible at hairline
/// width, where a filled polygon disappears).
const MIN_RIBBON_PX: f32 = 1.5;

/// How thick the solid black outline is, hugging the note's own outline. This
/// is the structural rim — the crisp dark line that separates the note's color
/// from whatever the spectrogram is doing behind it. A fixed pixel width,
/// independent of the note's own thickness: an outline is an outline whether
/// the ribbon it wraps is fat or a hairline.
pub(super) const BORDER_PX: f32 = 1.0;

/// How thick the white glow line outside the black outline is — a bright
/// highlight riding the note's outer edge. A fixed width, like [`BORDER_PX`].
///
/// A full logical pixel, not less, and that floor is deliberate: the glow is
/// the brightest thing on a note, and a *bright* sub-pixel line shimmers as it
/// scrolls, its peak intensity wobbling with every sub-pixel step across the
/// grid (worst on a Hi-DPI display, where a 0.6px line is barely over one
/// physical pixel). At a full pixel the coverage stays put and the highlight
/// holds still. The wider black outline and the note's own outline are already
/// past this floor, which is why the glow was the one seen to flicker.
pub(super) const KEYLINE_PX: f32 = 1.0;

/// The roll's glow reads this much brighter than the raw Edge fraction, so a
/// modest Edge setting still lands a crisp highlight over a bright spectrogram.
/// Only the roll's glow is boosted; the spectrum profile's edge ([`keyline`])
/// is left at the fraction, since it is one line on a filled slab and does not
/// have a dark backing to be seen against.
const GLOW_INTENSITY: f32 = 2.0;

/// How strong the Edge rim is here: the Edge setting scaled by the note's own
/// opacity, or `None` when there is too little of it to draw. The gate for the
/// whole rim — turn Edge off and the note is drawn with its outline alone.
fn edge_strength(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<f32> {
    let strength = cfg.keyline.clamp(0.0, 1.0) * alpha;
    (strength > 0.004).then_some(strength)
}

/// The light edge drawn along the spectrum's profile, at `cfg.keyline`
/// strength — `None` when the setting is off.
///
/// The curve's own colors come from the spectrogram palette, so where it is
/// quiet it is drawn in that palette's dark end against the pane's dark
/// background, with no edge, and the shape stops existing. A light rim gives
/// it an edge to be seen by. It is a setting because how much is right depends
/// entirely on the palette and opacity in play.
///
/// The roll's notes carry a brighter version of this (see [`glow`]); the
/// profile keeps the plain fraction, having no black backing to compete with.
pub(super) fn keyline(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    edge_strength(cfg, alpha).map(|s| Color32::WHITE.gamma_multiply(s))
}

/// The white glow line a roll note carries on its outer edge, brighter than
/// the raw Edge fraction (see [`GLOW_INTENSITY`]). `None` when Edge is off.
///
/// It rides OUTSIDE the black outline, so it needs to punch — a faint white
/// line lost against a bright spectrogram cell was the old keyline's failing.
pub(super) fn glow(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    edge_strength(cfg, alpha)?;
    let s = (cfg.keyline.clamp(0.0, 1.0) * GLOW_INTENSITY).min(1.0) * alpha;
    Some(Color32::WHITE.gamma_multiply(s))
}

/// The solid black outline a roll note carries just outside its own colored
/// outline — `None` when Edge is off.
///
/// Opaque at the note's opacity, independent of the Edge magnitude: Edge turns
/// the rim on and sets the GLOW's intensity, but the black is structural and
/// always fully drawn, so the note has a crisp dark separation from the picture
/// however faint the glow is set. It fades only with the note's own opacity, so
/// it never reads bolder than the note it edges.
///
/// Roll notes only. The spectrum's profile is a single curve on one side of a
/// filled slab, not a shape to pick out of a background, and a dark line under
/// it just reads as a shadow.
pub(super) fn border(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    edge_strength(cfg, alpha)?;
    Some(Color32::BLACK.gamma_multiply(alpha))
}

/// Draw every remembered note that falls inside the pane's time window and
/// pitch range. `split` is the depth fraction the roll starts at; `now` is
/// the shell clock, the same one the tracker's events are stamped with.
/// `surface` names the roll (0 the docked pane / offline render, 1 the
/// Render preview), so two live copies get their own instance buffer.
///
/// One paint callback for the whole roll, drawing one instanced quad per
/// note segment — NOT a stack of stroked rounded rects through egui's
/// tessellator, which is what this used to be. The roll is a scrolling
/// picture of immutable content, and immediate mode re-uploaded every
/// vertex of it every frame: 100k+ vertices, 4-5 ms of pure upload, the
/// dominant cost of the frame. Four vertices a note and a distance field
/// for the bands costs neither. See `lattice_render::roll` for the
/// measurement and the shape of the fix.
pub(super) fn draw_roll(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
    surface: usize,
) {
    let notes = note_instances(axes, scale, state, split, now);
    if surface == 0 {
        // What the roll now costs, for the performance overlay: this geometry
        // stopped passing through egui's vertex buffer, so the `verts` row
        // can no longer see it. The Render preview is a second roll and does
        // not publish (see `SharedState::roll_notes`).
        state
            .roll_notes
            .store(notes.len() as u32, std::sync::atomic::Ordering::Relaxed);
    }
    if notes.is_empty() {
        return;
    }
    let dir = |v: egui::Vec2| [v.x, v.y];
    painter.add(lattice_render::roll_paint_callback(
        axes.rect,
        notes,
        RollAxes { pitch_dir: dir(axes.dir_pitch()), depth_dir: dir(axes.dir_depth()) },
        state.target_format,
        surface as u64,
    ));
}

/// Every visible note segment as one GPU instance, in paint order.
///
/// The whole geometry of the roll, and the only place it is decided — the
/// shader turns an instance into pixels but invents nothing. Split out from
/// [`draw_roll`] so the geometry can be read back in tests without a GPU.
///
/// Rebuilt from scratch every frame, deliberately: see the note on
/// append-and-evict in `lattice_render::roll`. It also keeps the far-edge
/// truncation below honest — a note crossing the window's oldest edge has
/// its geometry rewritten while it leaves, so it is not the immutable thing
/// a cache would need it to be.
pub(super) fn note_instances(
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
) -> Vec<RollInstance> {
    let cfg = &state.spectrum_config;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let oldest = time.oldest();

    let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
    let ribbon_px = 2.0 * half * axes.pitch_len();

    // Build in a stable order (the live notes come out of a HashMap, whose
    // iteration order varies per run): instances rasterize in buffer order,
    // so where two notes overlap the paint order is visible, and the offline
    // render must be byte-identical between runs.
    // Whole-song (offline playhead): the render lays the whole take out at once
    // from a full roll built up front. Live: the causal tracker's rolling
    // window, filling in as notes arrive.
    let roll = match state.whole_song.as_ref() {
        Some(ws) => &ws.roll,
        None => state.tracker.roll(),
    };
    // Cull to the visible window BEFORE sorting: the roll can remember
    // thousands of notes while only a handful are on screen, and sorting the
    // survivors alone (rather than every remembered note) keeps the same
    // deterministic paint order for far less work.
    //   - Entirely past the window's far end, or
    //   - entirely off the octave zoom (both endpoints outside and on the
    //     same side, so a note that merely crosses an edge still draws its
    //     visible part).
    let mut notes: Vec<&RollNote> = roll
        .notes()
        .filter(|note| {
            if note.stop(now) < oldest {
                return false;
            }
            let (a, b) = (note.start_pitch(), note.end_pitch());
            let (lo, hi) = (a.min(b), a.max(b));
            hi >= scale.min_midi - cfg.roll_thickness && lo <= scale.max_midi + cfg.roll_thickness
        })
        .collect();
    notes.sort_unstable_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.channel.cmp(&b.channel))
            .then(a.note.cmp(&b.note))
    });

    // One segment per note is the common case (a note is bent rarely), so the
    // note count is the right first guess at how many instances this makes.
    let mut instances = Vec::with_capacity(notes.len());
    for note in notes {
        for ((t0, p0), (t1, p1)) in note.segments(now) {
            let (t0, t1) = (t0.max(oldest), t1.max(oldest));
            if t1 < oldest {
                continue;
            }
            let (d0, d1) = (time.depth_of(t0), time.depth_of(t1));
            let (a0, a1) = (scale.t_of(p0), scale.t_of(p1));

            // Notes always draw fully opaque — the ribbon is a hollow outline
            // over the spectrogram, so it does not need to be dimmed to let the
            // heatmap through, and a released note fades on the lattice's fade,
            // not here.
            let alpha = 1.0;
            let pitch = (p0 + p1) * 0.5;
            // The crisp outline is the note's TRUE color, so it matches the
            // same note on the lattice.
            let core = note_color(note, cfg, state, pitch, alpha);
            // Reading outward: the note's color, a solid black outline hugging
            // it, then the bright white glow riding the black's outer edge,
            // then whatever the spectrogram is doing. The black gives the note
            // a crisp separation; the glow is the highlight.
            //
            // Both stand entirely OUTSIDE the note's own outline, never as a
            // wider stroke of the same path — a centered stroke grows inward
            // exactly as much as outward, and a note is only a few pixels thick
            // at the pitch ranges this pane is actually used at, so the two
            // long edges met in the middle and flooded the hollow interior
            // white. Here that is structural: the shader reads the bands off
            // the DISTANCE to the outline, and a band at distance 1..2 cannot
            // reach back inside it whatever the ribbon's thickness.
            //
            // The rim used to carry two more bands approximating the lattice's
            // bloom. They are gone: a note is a hollow outline over the
            // heatmap, and the halo cost two extra stroked rounded rects every
            // note, every frame. Bloom is the lattice's alone, which is where
            // it belongs — there it is a real post-process pass, not four
            // hand-placed bands standing in for one.
            let (rim, dark, light) = match glow(cfg, alpha).zip(border(cfg, alpha)) {
                Some((light, dark)) => ([BORDER_PX, KEYLINE_PX], dark, light),
                // Off is off: no rim at all, not a hairline that can't be
                // cleared.
                None => ([0.0, 0.0], Color32::TRANSPARENT, Color32::TRANSPARENT),
            };

            // Geometry in the pane's own two axes, which is all the shader is
            // told: `Axes` maps those onto perpendicular screen axes, so
            // nothing here names a screen side either.
            //
            // NOT snapped to whole pixels, which egui does to rects by default
            // (TessellationOptions::round_rects_to_pixels) to keep static
            // chrome crisp. These scroll: snapping holds a note still until it
            // has drifted a whole pixel and then jumps it, so the roll advanced
            // in steps while the spectrogram — a mesh, never snapped — slid
            // smoothly underneath, and the notes read as jittering against it.
            // A distance field has no pixel grid to snap to, so the sub-pixel
            // placement is now simply what it does.
            let center = axes.at((a0 + a1) * 0.5, (d0 + d1) * 0.5);
            let depth_px = (d1 - d0) * axes.depth_len();
            // How far the note's center line drifts along the pitch axis per
            // point of depth: 0 for a held note, non-zero for a glide, which
            // shears the box into the parallelogram the ribbon follows. Guarded
            // because a segment can have no duration at all — a note pressed
            // this frame is one — and a slope is meaningless there.
            let slope =
                if depth_px.abs() > 1e-6 { (a1 - a0) * axes.pitch_len() / depth_px } else { 0.0 };
            let width = cfg.roll_outline_width.clamp(0.5, 8.0);
            let (half_pitch, width, radius) = if ribbon_px < MIN_RIBBON_PX {
                // Too thin to bound: the note IS its spine. It has no interior
                // to hollow out, so the outline width alone carries it (at
                // hairline width a filled shape disappears, a line does not),
                // and the rim stands outside that.
                (0.0, width.max(MIN_RIBBON_PX), 0.0)
            } else {
                // Corners round only where the ribbon runs straight. A glide's
                // segments butt end to end, and rounding those ends would pinch
                // the ribbon at every breakpoint.
                let radius = if p0 == p1 {
                    cfg.roll_rounding.clamp(0.0, 1.0) * ribbon_px * 0.5
                } else {
                    0.0
                };
                (half * axes.pitch_len(), width, radius)
            };

            instances.push(RollInstance {
                center: [center.x, center.y],
                half_extent: [half_pitch, depth_px.abs() * 0.5],
                shape: [slope, radius, width, 0.0],
                rim,
                core: core.to_array(),
                border: dark.to_array(),
                glow: light.to_array(),
            });
        }
    }
    instances
}

/// The color of a note at `pitch`, per the Color setting.
fn note_color(
    note: &RollNote,
    cfg: &crate::SpectrumConfig,
    state: &SharedState,
    pitch: f32,
    alpha: f32,
) -> Color32 {
    let (darkest, brightest) =
        (state.frame_params.darkest_pitch, state.frame_params.brightest_pitch);
    match cfg.roll_color {
        RollColor::Channel => {
            scene_color(channel_color(note.channel, pitch, darkest, brightest), alpha)
        }
        RollColor::Pitch => {
            scene_color(channel_color(PITCH_RAMP_CHANNEL, pitch, darkest, brightest), alpha)
        }
        RollColor::Accent => theme::accent().gamma_multiply(alpha),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SharedState, SpectralOrientation};
    use lattice_core::{NoteEvent, NoteEventKind};

    /// The pane the tests paint into: 300 points along the time axis, 100
    /// across pitch.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };

    /// The roll's geometry for `state`, derived exactly the way
    /// [`spectral_pane`](super::super::spectral::spectral_pane) derives it
    /// before handing over — same axes, same pitch scale, same split.
    fn instances(state: &SharedState, now: f64) -> Vec<RollInstance> {
        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::spectral::spectrum_share(cfg);
        note_instances(&axes, &scale, state, split, now)
    }

    /// One held note, and the instances the roll would draw for it. `range`
    /// is the pitch span in semitones — the pane is 100 points across the
    /// pitch axis, so a wide range makes a thin ribbon, which is where the
    /// rim geometry is under the most pressure.
    fn ribbon_with_range(keyline: f32, range: f32) -> Vec<RollInstance> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 60.0 - range * 0.5;
        state.spectrum_config.high_midi = 60.0 + range * 0.5;
        state.spectrum_config.roll_outline_width = 2.0;
        state.spectrum_config.roll_thickness = 2.0;
        state.spectrum_config.keyline = keyline;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        instances(&state, 0.05)
    }

    /// The note is 12 semitones across a 100-point axis (a thick ribbon), so
    /// it is bounded rather than drawn as a bare spine.
    fn ribbon(keyline: f32) -> Vec<RollInstance> {
        ribbon_with_range(keyline, 12.0)
    }

    fn one(rects: &[RollInstance]) -> &RollInstance {
        assert_eq!(rects.len(), 1, "expected one note segment, got {}", rects.len());
        &rects[0]
    }

    /// The rim is a fixed pixel thickness whatever the note's own width — an
    /// outline should not thin out just because the ribbon it wraps did — and
    /// it is expressed as a DISTANCE OUTSIDE the note's outline, never as a
    /// wider stroke of it.
    ///
    /// That distinction is the flood fix, and it is now structural: the
    /// shader reads the black outline off distances `w/2 .. w/2 + border` and
    /// the glow off the band beyond it, so neither can reach back inside the
    /// ribbon however thin it is. What is left to check here is that the two
    /// thicknesses are handed over unscaled.
    #[test]
    fn the_rim_is_the_same_thickness_at_any_note_width() {
        let thick = ribbon_with_range(0.5, 12.0);
        // ~120 semitones over 100 points: the ribbon is under 2 points thick,
        // which is where the old centered strokes met in the middle and
        // painted the hollow interior white.
        let thin = ribbon_with_range(0.5, 120.0);
        assert_eq!(one(&thick).rim, [BORDER_PX, KEYLINE_PX]);
        assert_eq!(one(&thin).rim, one(&thick).rim, "the rim thinned with the note");
        assert!(
            one(&thin).half_extent[0] < one(&thick).half_extent[0],
            "the two notes are the same thickness; the comparison is vacuous",
        );
    }

    /// The black outline is solid (opaque at the note's opacity) and the
    /// white glow is bright and grid-stable: reading outward, color, a crisp
    /// black line, then a punchy highlight. The glow reads stronger than the
    /// raw Edge fraction — that boost is the "more intense" ask — and it is a
    /// full logical pixel wide, so a bright line does not shimmer as the note
    /// scrolls across the pixel grid.
    ///
    /// Edge off is off: no rim at all, not a hairline that can't be cleared.
    #[test]
    fn the_black_outline_is_solid_and_the_glow_is_bright_and_grid_stable() {
        // A modest Edge, below the point where the boosted glow clips to
        // full, so "brighter than the fraction" is a real comparison.
        let edge = 0.3;
        let lit = ribbon(edge);
        let note = one(&lit);
        assert_eq!(note.border, Color32::BLACK.to_array(), "the black outline is not solid");
        let glow_alpha = f32::from(note.glow[3]) / 255.0;
        assert!(
            glow_alpha > edge + 0.05,
            "the glow ({glow_alpha}) is no brighter than the Edge fraction {edge}",
        );
        assert!(
            note.rim[1] >= 1.0,
            "the glow ({}) is sub-pixel and will shimmer when scrolling",
            note.rim[1],
        );
        assert!(
            note.rim[1] <= note.rim[0],
            "the glow ({}) is bolder than the black outline ({})",
            note.rim[1],
            note.rim[0],
        );

        let dark = ribbon(0.0);
        let note = one(&dark);
        assert_eq!(note.rim, [0.0, 0.0], "Edge 0 still drew a rim");
        assert_eq!(note.glow[3], 0, "Edge 0 left a glow color behind");
        assert_eq!(note.border[3], 0, "Edge 0 left an outline color behind");
    }

    /// A ribbon too thin to bound is drawn as a bare spine: no pitch
    /// half-extent at all, and the outline width alone carries it, floored so
    /// it stays visible. A filled shape disappears at hairline width; a line
    /// does not.
    #[test]
    fn a_hairline_ribbon_becomes_a_bare_spine() {
        let thick = ribbon_with_range(0.5, 12.0);
        let note = one(&thick);
        assert!(note.half_extent[0] > 0.0, "a bounded ribbon lost its thickness");
        assert_eq!(note.shape[2], 2.0, "a bounded ribbon should use the outline width as set");

        let thin = ribbon_with_range(0.5, 600.0);
        let note = one(&thin);
        assert_eq!(note.half_extent[0], 0.0, "a hairline ribbon still claims a half-width");
        assert!(
            note.shape[2] >= MIN_RIBBON_PX,
            "a hairline spine ({}) is thinner than it can be seen at",
            note.shape[2],
        );
    }

    /// A glide is the same instance sheared, not a second kind of shape: the
    /// note's center line drifts along pitch as it runs down the depth axis.
    /// Its corners stay square — a glide's segments butt end to end, and
    /// rounding those ends would pinch the ribbon at every breakpoint.
    #[test]
    fn a_glide_shears_the_note_rather_than_needing_another_shape() {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.spectrum_config.roll_rounding = 1.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let held = instances(&state, 1.0);
        assert_eq!(one(&held).shape[0], 0.0, "a held note should not be sheared");
        assert!(one(&held).shape[1] > 0.0, "a held note should round its corners");

        // Bend it a semitone up over the next second.
        state.tracker.handle_event(NoteEvent {
            time: 1.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 1.0 },
        });
        let bent = instances(&state, 2.0);
        let glide = bent
            .iter()
            .find(|n| n.shape[0] != 0.0)
            .expect("the bent segment should be sheared");
        assert_eq!(glide.shape[1], 0.0, "a glide's ends should stay square");
    }

    /// Notes are placed sub-pixel, and that is load-bearing. egui snaps rects
    /// to whole pixels by default (`round_rects_to_pixels`) to keep static
    /// chrome crisp; these scroll, and snapping held a note still until it had
    /// drifted a whole pixel and then jumped it, while the spectrogram — a
    /// mesh, never snapped — slid smoothly underneath, so the notes read as
    /// jittering against it. Four frames a fraction of a pixel apart must
    /// move the note by that same fraction each time, not by 0 then 1.
    #[test]
    fn a_scrolling_note_moves_sub_pixel_rather_than_in_whole_pixel_jumps() {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        // A step that scrolls the roll by well under one point.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let window = f64::from(state.spectrum_config.roll_seconds);
        let step = 0.3 * window / f64::from(axes.depth_len());
        let at = |now: f64| {
            let notes = instances(&state, now);
            let note = one(&notes);
            egui::pos2(note.center[0], note.center[1])
        };
        let moves: Vec<f32> = (0..4)
            .map(|i| at(1.0 + step * (i + 1) as f64) - at(1.0 + step * i as f64))
            .map(|d| d.length())
            .collect();
        for step in &moves {
            assert!(
                *step > 0.0 && *step < 0.5,
                "a sub-pixel scroll moved the note by {step} points: {moves:?}",
            );
        }
        let spread = moves.iter().fold(0.0f32, |a, &b| a.max(b))
            - moves.iter().fold(f32::MAX, |a, &b| a.min(b));
        assert!(spread < 1e-3, "the note scrolled unevenly (snapped?): {moves:?}");
    }
}
