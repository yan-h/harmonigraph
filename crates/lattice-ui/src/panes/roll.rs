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
pub(super) fn draw_roll(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
) {
    let cfg = &state.spectrum_config;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let oldest = time.oldest();

    let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
    let ribbon_px = 2.0 * half * axes.pitch_len();
    let opacity = cfg.roll_opacity.clamp(0.0, 1.0);

    // Draw in a stable order (the live notes come out of a HashMap, whose
    // iteration order varies per run): with translucent glows the paint order
    // of overlapping notes is visible, and the offline render must be
    // byte-identical between runs.
    // Whole-song (offline playhead): the render lays the whole take out at once
    // from a full roll built up front. Live: the causal tracker's rolling
    // window, filling in as notes arrive.
    let roll = match state.whole_song.as_ref() {
        Some(ws) => &ws.roll,
        None => state.tracker.roll(),
    };
    let mut notes: Vec<&RollNote> = roll.notes().collect();
    notes.sort_unstable_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.channel.cmp(&b.channel))
            .then(a.note.cmp(&b.note))
    });
    for note in notes {
        // Entirely past the window's far end, or entirely off the octave
        // zoom (both endpoints outside and on the same side, so a note
        // that merely crosses the edge still draws its visible part).
        if note.stop(now) < oldest {
            continue;
        }
        let (lo, hi) = {
            let (a, b) = (note.start_pitch(), note.end_pitch());
            (a.min(b), a.max(b))
        };
        if hi < scale.min_midi - cfg.roll_thickness || lo > scale.max_midi + cfg.roll_thickness {
            continue;
        }

        for ((t0, p0), (t1, p1)) in note.segments(now) {
            let (t0, t1) = (t0.max(oldest), t1.max(oldest));
            if t1 < oldest {
                continue;
            }
            let (d0, d1) = (time.depth_of(t0), time.depth_of(t1));
            let (a0, a1) = (scale.t_of(p0), scale.t_of(p1));

            let mut alpha = opacity;
            if cfg.roll_velocity_alpha {
                alpha *= super::visibility_floor(note.velocity);
            }
            if alpha <= 0.004 {
                continue;
            }
            let pitch = (p0 + p1) * 0.5;
            let width = cfg.roll_outline_width.clamp(0.5, 8.0);
            let body = |a: f32| note_color(note, cfg, state, pitch, a);

            // Everything around the outline is drawn as a band standing
            // OUTSIDE it, never as a wider stroke of the same path.
            //
            // A centered stroke grows inward exactly as much as outward, and a
            // note is only a few pixels thick at the pitch ranges this pane is
            // actually used at. The keyline was a stroke of `width + 2px`, so
            // it reached ~1.75px inward from BOTH long edges — they met in the
            // middle, painted the hollow interior white, and painted it twice
            // where they overlapped. The note stopped reading as a ribbon with
            // a light edge and read as a translucent white box instead.
            //
            // Each band is (standoff from the outline's outer edge, thickness,
            // color), outermost first. Bands don't overlap, so for the shapes
            // that can be expanded the order is cosmetic; it matters only in
            // the hairline branch, where there is no interior to stand outside
            // of and the bands go under the spine widest-first.
            let mut bands: Vec<(f32, f32, Color32)> = Vec::with_capacity(4);
            let rim = glow(cfg, alpha).zip(border(cfg, alpha));
            // Where the rim ends, so the bloom can start outside it.
            let rim_px = if rim.is_some() { BORDER_PX + KEYLINE_PX } else { 0.0 };
            // Bloom: a soft halo around the note, driven by the SAME setting as
            // the lattice's bloom so the two panes share the look. egui has no
            // post-process pass like the lattice's wgpu bloom, so approximate
            // one — two faint bands outside the rim, the brighter one nested
            // inside the dimmer, which reads as a falloff. Outside the rim
            // rather than under it: a halo spills from the whole of what you
            // can see of the note.
            let g = state.view.bloom_strength.clamp(0.0, 2.0);
            if g > 0.0 {
                bands.push((rim_px, g * 1.5, brighten(body(alpha * 0.12 * g))));
                bands.push((rim_px, g * 0.75, brighten(body(alpha * 0.20 * g))));
            }
            if let Some((light, dark)) = rim {
                // Reading outward: the note's color, a solid black outline
                // hugging it, then the bright white glow riding the black's
                // outer edge, then whatever the spectrogram is doing. The black
                // gives the note a crisp separation; the glow is the highlight.
                bands.push((BORDER_PX, KEYLINE_PX, light));
                bands.push((0.0, BORDER_PX, dark));
            }
            // The crisp outline is the note's TRUE color, so it matches the
            // same note on the lattice. It goes on top of everything.
            let core = body(alpha);

            if ribbon_px < MIN_RIBBON_PX {
                // Too thin to bound: the note IS its spine. A line has no
                // interior to flood, so here a band really is just a wider
                // stroke underneath — widest first.
                let spine = [axes.at(a0, d0), axes.at(a1, d1)];
                let core_w = width.max(MIN_RIBBON_PX);
                for &(out, thick, color) in &bands {
                    painter.line_segment(
                        spine,
                        egui::Stroke::new(core_w + 2.0 * (out + thick), color),
                    );
                }
                painter.line_segment(spine, egui::Stroke::new(core_w, core));
            } else if p0 == p1 {
                // Unbent: a hollow axis-aligned rectangle (the only shape egui
                // will round the corners of).
                let rect = egui::Rect::from_two_pos(axes.at(a0 - half, d0), axes.at(a1 + half, d1));
                let radius = cfg.roll_rounding.clamp(0.0, 1.0) * ribbon_px * 0.5;
                // NOT snapped to whole pixels, which egui does to rects by
                // default (TessellationOptions::round_rects_to_pixels) to keep
                // static chrome crisp. These rects scroll: snapping holds a
                // note still until it has drifted a whole pixel and then jumps
                // it, so the roll advanced in steps while the spectrogram — a
                // mesh, never snapped — slid smoothly underneath, and the notes
                // read as jittering against it. Sub-pixel placement costs a
                // little edge softness and buys motion that matches.
                let stroked = |rect: egui::Rect, radius: f32, thick: f32, color| {
                    egui::epaint::RectShape::stroke(
                        rect,
                        egui::CornerRadius::same(radius.min(127.0) as u8),
                        egui::Stroke::new(thick, color),
                        egui::StrokeKind::Middle,
                    )
                    .with_round_to_pixels(false)
                };
                for &(out, thick, color) in &bands {
                    // Grow the RECT by the band's distance and keep the stroke
                    // thin, rather than growing the stroke on the same rect:
                    // the shape moves outward, and nothing reaches back inside.
                    // The corner radius grows with it, or the rim would round
                    // tighter than the note it wraps.
                    let e = width * 0.5 + out + thick * 0.5;
                    painter.add(stroked(rect.expand(e), radius + e, thick, color));
                }
                painter.add(stroked(rect, radius, width, core));
            } else {
                // Bent: a hollow quad following the glide. Wound consistently so
                // egui's convex-polygon stays valid whichever way the axes run.
                //
                // Grown in the (pitch, depth) plane, where the ribbon's own
                // half-width already lives: `Axes` maps those two onto
                // perpendicular screen axes, so `e` pixels of standoff is
                // exactly `e / pitch_len` and `e / depth_len` there, whichever
                // way round the pane is.
                let quad = |e: f32| {
                    let ep = e / axes.pitch_len().max(1.0);
                    // Away from the other end, so the ends grow outward rather
                    // than the segment sliding along itself.
                    let ed = e / axes.depth_len().max(1.0) * (d1 - d0).signum();
                    vec![
                        axes.at(a0 - half - ep, d0 - ed),
                        axes.at(a0 + half + ep, d0 - ed),
                        axes.at(a1 + half + ep, d1 + ed),
                        axes.at(a1 - half - ep, d1 + ed),
                    ]
                };
                for &(out, thick, color) in &bands {
                    let e = width * 0.5 + out + thick * 0.5;
                    painter.add(egui::Shape::convex_polygon(
                        quad(e),
                        Color32::TRANSPARENT,
                        egui::Stroke::new(thick, color),
                    ));
                }
                painter.add(egui::Shape::convex_polygon(
                    quad(0.0),
                    Color32::TRANSPARENT,
                    egui::Stroke::new(width, core),
                ));
            }
        }
    }

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

/// The edge/onset variant of a note color: the same hue, lifted toward
/// white so it reads against the ribbon it sits on.
fn brighten(color: Color32) -> Color32 {
    let lift = |v: u8| v.saturating_add((255 - v) / 2);
    Color32::from_rgba_unmultiplied(lift(color.r()), lift(color.g()), lift(color.b()), color.a())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SharedState, SpectralOrientation};
    use lattice_core::{NoteEvent, NoteEventKind};

    /// One rect the roll emitted: its stroke width and color, and the
    /// rectangle the stroke is centered on.
    struct Ribbon {
        width: f32,
        color: Color32,
        rect: egui::Rect,
    }

    /// Paint one held note and report every rect the roll emitted. `range` is
    /// the pitch span in semitones — the pane is 100px across the pitch axis,
    /// so a wide range makes a thin ribbon, which is where the rim geometry is
    /// under the most pressure.
    fn ribbon_with_range(keyline: f32, range: f32) -> Vec<Ribbon> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 60.0 - range * 0.5;
        state.spectrum_config.high_midi = 60.0 + range * 0.5;
        state.spectrum_config.roll_opacity = 1.0;
        state.spectrum_config.roll_velocity_alpha = false;
        state.spectrum_config.roll_outline_width = 2.0;
        state.spectrum_config.roll_thickness = 2.0;
        state.spectrum_config.keyline = keyline;
        state.view.bloom_strength = 0.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let rect = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                crate::panes::spectral::spectral_pane(&mut child, &mut state, 0.05, 1.0, 0);
            },
        );
        out.shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Rect(r) => {
                    Some(Ribbon { width: r.stroke.width, color: r.stroke.color, rect: r.rect })
                }
                _ => None,
            })
            .collect()
    }

    /// The note is a rect 12 semitones tall (a thick ribbon), so the rim has
    /// room and the geometry is easy to read.
    fn ribbon(keyline: f32) -> Vec<Ribbon> {
        ribbon_with_range(keyline, 12.0)
    }

    /// Note rects opt OUT of egui's pixel snapping, which is on by default
    /// for rects so that static chrome stays crisp. These scroll: snapping
    /// holds a note still until it has drifted a whole pixel and then jumps
    /// it, while the spectrogram — a mesh, never snapped — slides smoothly
    /// underneath. The notes read as jittering against it.
    #[test]
    fn note_rects_are_not_snapped_to_whole_pixels() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        // Zoomed in enough that the ribbon is a rect rather than a bare
        // spine — the thin branch has no rect to snap.
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let rect = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                crate::panes::spectral::spectral_pane(&mut child, &mut state, 0.05, 1.0, 0);
            },
        );
        let stroked: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.round_to_pixels),
                _ => None,
            })
            .collect();
        assert!(!stroked.is_empty(), "the note drew no ribbon to check");
        assert!(
            stroked.iter().all(|&r| r == Some(false)),
            "a scrolling note rect is still pixel-snapped: {stroked:?}",
        );
    }

    /// The crisp outline: the widest stroke, in the note's own color.
    fn core_of(rects: &[Ribbon]) -> &Ribbon {
        rects.iter().max_by(|a, b| a.width.total_cmp(&b.width)).expect("no ribbon drawn")
    }

    /// The white glow and the solid black outline, told apart by brightness
    /// among the note's rects — the two rim bands are neutral grey (r==g==b),
    /// bright for the glow and dark for the outline. Identified this way rather
    /// than by an exact color so tuning the glow's intensity leaves the
    /// geometry tests alone. The core is excluded by width: it is the widest
    /// stroke, and carries the note's own (hued) color anyway.
    fn rim_of(rects: &[Ribbon]) -> (Option<&Ribbon>, Option<&Ribbon>) {
        let core_w = core_of(rects).width;
        let grey = |r: &&Ribbon| {
            let c = r.color;
            c.a() > 0 && c.r() == c.g() && c.g() == c.b() && r.width < core_w - 0.01
        };
        let white = rects.iter().filter(grey).find(|r| r.color.r() >= 128);
        let black = rects.iter().filter(grey).find(|r| r.color.r() < 128);
        (white, black)
    }

    /// The filled span of a stroked rect: the rectangle it is drawn on, grown
    /// and shrunk by half the (centered) stroke width. Painted pixels lie
    /// between `inner` and `outer`.
    fn outer(r: &Ribbon) -> egui::Rect {
        r.rect.expand(r.width * 0.5)
    }
    fn inner(r: &Ribbon) -> egui::Rect {
        r.rect.shrink(r.width * 0.5)
    }

    /// A tolerant `contains`, since the geometry is built from floats.
    fn encloses(outer: egui::Rect, inner: egui::Rect) -> bool {
        outer.min.x <= inner.min.x + 0.01
            && outer.min.y <= inner.min.y + 0.01
            && outer.max.x >= inner.max.x - 0.01
            && outer.max.y >= inner.max.y - 0.01
    }

    /// The Edge rim is a solid black outline hugging the note with a white glow
    /// riding its outer edge, and BOTH stand entirely outside the note's own
    /// outline. This is the flood fix: they used to be wider strokes of the same
    /// rectangle, so on a thin note they grew inward and painted the hollow
    /// interior — the note read as a translucent box instead of an edged ribbon.
    /// The invariant is that each rim band's INNER (filled) edge still encloses
    /// the outline's OUTER edge, so no rim pixel lands on the note's interior.
    #[test]
    fn the_edge_rim_stands_outside_the_note_never_inside_it() {
        let rects = ribbon(0.5);
        let core_outer = outer(core_of(&rects));
        let (Some(glow), Some(black)) = rim_of(&rects) else {
            panic!("expected a glow and a black outline, got {} rects", rects.len());
        };
        for (name, band) in [("glow", glow), ("black outline", black)] {
            assert!(
                encloses(inner(band), core_outer),
                "the {name} reaches inside the note rather than sitting outside it",
            );
        }
        // The white glow rides the OUTSIDE of the black outline.
        assert!(
            encloses(inner(glow), outer(black)),
            "the white glow should sit outside the black outline it rides",
        );

        // Off is off: no rim at all, not a hairline that can't be cleared.
        let none = ribbon(0.0);
        let (g, b) = rim_of(&none);
        assert!(g.is_none() && b.is_none(), "Edge 0 still drew a rim");
    }

    /// The same invariant at the pitch range where the bug actually bit: the
    /// whole analyzer axis, where a note is a couple of pixels thick. A centered
    /// stroke would have painted straight across the interior here — the band's
    /// inner edge would collapse or invert and stop enclosing the outline.
    /// Thickness-independent by construction, so it holds regardless.
    #[test]
    fn the_rim_does_not_flood_a_thin_note() {
        // ~120 semitones over 100px: the ribbon is under 2px thick.
        let rects = ribbon_with_range(0.5, 120.0);
        let core_outer = outer(core_of(&rects));
        let (Some(glow), Some(black)) = rim_of(&rects) else {
            panic!("a lit thin note drew no rim to check");
        };
        for band in [glow, black] {
            assert!(
                encloses(inner(band), core_outer),
                "the rim floods the thin note's interior instead of edging it",
            );
        }
    }

    /// The rim is a fixed pixel thickness whatever the note's own width — an
    /// outline should not thin out just because the ribbon it wraps did. The
    /// black outline and the white glow each draw the same width on a thin note
    /// as on a thick one.
    #[test]
    fn the_rim_is_the_same_thickness_at_any_note_width() {
        // Same Edge, very different ribbon thickness (wide vs narrow pitch span).
        let thick = ribbon_with_range(0.5, 12.0);
        let thin = ribbon_with_range(0.5, 120.0);
        let (Some(gw_t), Some(bk_t)) = rim_of(&thick) else { panic!("no rim on the thick note") };
        let (Some(gw_n), Some(bk_n)) = rim_of(&thin) else { panic!("no rim on the thin note") };
        assert!(
            (gw_t.width - gw_n.width).abs() < 0.001,
            "the glow thinned with the note: {} vs {}",
            gw_t.width,
            gw_n.width,
        );
        assert!(
            (bk_t.width - bk_n.width).abs() < 0.001,
            "the black outline thinned with the note: {} vs {}",
            bk_t.width,
            bk_n.width,
        );
    }

    /// The black outline is solid (opaque at the note's opacity) and the white
    /// glow is bright and grid-stable: reading outward, color, a crisp black
    /// line, then a punchy highlight. The glow reads stronger than the raw Edge
    /// fraction — that boost is the "more intense" ask — and it is a full
    /// logical pixel wide, so a bright line does not shimmer as the note
    /// scrolls across the pixel grid.
    #[test]
    fn the_black_outline_is_solid_and_the_glow_is_bright_and_grid_stable() {
        // A modest Edge, below the point where the boosted glow clips to full,
        // so "brighter than the fraction" is a real comparison.
        let edge = 0.3;
        let rects = ribbon(edge);
        let (Some(glow), Some(black)) = rim_of(&rects) else {
            panic!("no rim drawn");
        };
        // Solid: opaque at the note's own opacity (1.0 here).
        assert_eq!(black.color.a(), 255, "the black outline is not solid");
        // Bright: the glow's alpha clears the raw Edge fraction by a margin.
        assert!(
            f32::from(glow.color.a()) / 255.0 > edge + 0.05,
            "the glow ({}) is no brighter than the Edge fraction {edge}",
            glow.color.a(),
        );
        // Grid-stable: a full logical pixel, so a bright moving line holds its
        // peak instead of twinkling. It never reads bolder than the outline.
        assert!(
            glow.width >= 1.0 - 1e-3,
            "the glow ({}) is sub-pixel and will shimmer when scrolling",
            glow.width,
        );
        assert!(
            glow.width <= black.width + 1e-3,
            "the glow ({}) is bolder than the black outline ({})",
            glow.width,
            black.width,
        );
    }
}
