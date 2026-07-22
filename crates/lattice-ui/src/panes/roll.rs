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

use super::spectral::{Axes, PitchScale};
use super::{scene_color, PITCH_RAMP_CHANNEL};
use crate::{theme, RollColor, SharedState};

/// Below this many pixels a ribbon is too thin to read as a shape, so it
/// is drawn as a bare line instead (which stays visible at hairline
/// width, where a filled polygon disappears).
const MIN_RIBBON_PX: f32 = 1.5;

/// Draw every remembered note that falls inside the pane's time window and
/// octave zoom. `split` is the depth fraction the roll starts at; `now` is
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
    // A degenerate span would put every note on one line; clamp rather
    // than divide by it.
    let window = cfg.roll_seconds.max(0.05) as f64;
    let depth_span = 1.0 - split;

    // Time to depth: `now` sits on the split, the window's far end at 1.
    let depth_of = |t: f64| split + ((now - t) / window).clamp(0.0, 1.0) as f32 * depth_span;

    // Time gridlines first, so notes sit on top of them.
    if cfg.roll_grid_seconds > 0.0 {
        let step = cfg.roll_grid_seconds as f64;
        let mut t = step;
        // Cap the count as well as the span: a tiny step on a long window
        // would otherwise ask for thousands of hairlines.
        let mut drawn = 0;
        while t <= window && drawn < 400 {
            painter.line_segment(
                axes.across_pitch(depth_of(now - t)),
                egui::Stroke::new(1.0, theme::surface_faint()),
            );
            t += step;
            drawn += 1;
        }
    }

    let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
    let ribbon_px = 2.0 * half * axes.pitch_len();
    let opacity = cfg.roll_opacity.clamp(0.0, 1.0);
    let oldest = now - window;

    // Draw in a stable order (the live notes come out of a HashMap, whose
    // iteration order varies per run): with translucent glows the paint order
    // of overlapping notes is visible, and the offline render must be
    // byte-identical between runs.
    let mut notes: Vec<&RollNote> = state.tracker.roll().notes().collect();
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
            let (d0, d1) = (depth_of(t0), depth_of(t1));
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
            // Bloom: a soft glow around the outline, driven by the SAME setting
            // as the lattice's bloom so the two panes share the look. egui has
            // no post-process pass like the lattice's wgpu bloom, so approximate
            // one — a couple of wider, fainter passes of the stroke under the
            // crisp one; more bloom widens and brightens the halo.
            let g = state.view.bloom_strength.clamp(0.0, 2.0);
            // (stroke width, alpha fraction) per pass; the crisp outline is last.
            let mut passes: Vec<(f32, f32)> = Vec::with_capacity(3);
            if g > 0.0 {
                passes.push((width + g * 3.0, 0.12 * g));
                passes.push((width + g * 1.5, 0.20 * g));
            }
            passes.push((width, 1.0));
            // The crisp outline (af == 1) is the note's TRUE color, so it
            // matches the same note on the lattice; only the fainter glow
            // passes brighten, toward the bloom halo.
            let stroke_color = |af: f32| {
                let c = note_color(note, cfg, state, pitch, alpha * af);
                if af >= 1.0 { c } else { brighten(c) }
            };

            if ribbon_px < MIN_RIBBON_PX {
                // Too thin to bound: the note IS its spine.
                for &(w, af) in &passes {
                    painter.line_segment(
                        [axes.at(a0, d0), axes.at(a1, d1)],
                        egui::Stroke::new(w.max(MIN_RIBBON_PX), stroke_color(af)),
                    );
                }
            } else if p0 == p1 {
                // Unbent: a hollow axis-aligned rectangle (the only shape egui
                // will round the corners of).
                let rect = egui::Rect::from_two_pos(axes.at(a0 - half, d0), axes.at(a1 + half, d1));
                let radius = cfg.roll_rounding.clamp(0.0, 1.0) * ribbon_px * 0.5;
                let rounding = egui::CornerRadius::same(radius.min(127.0) as u8);
                for &(w, af) in &passes {
                    painter.rect_stroke(
                        rect,
                        rounding,
                        egui::Stroke::new(w, stroke_color(af)),
                        egui::StrokeKind::Middle,
                    );
                }
            } else {
                // Bent: a hollow quad following the glide. Wound consistently so
                // egui's convex-polygon stays valid whichever way the axes run.
                let quad = vec![
                    axes.at(a0 - half, d0),
                    axes.at(a0 + half, d0),
                    axes.at(a1 + half, d1),
                    axes.at(a1 - half, d1),
                ];
                for &(w, af) in &passes {
                    painter.add(egui::Shape::convex_polygon(
                        quad.clone(),
                        egui::Color32::TRANSPARENT,
                        egui::Stroke::new(w, stroke_color(af)),
                    ));
                }
            }
        }
    }

    // The present moment, where the roll hands over to the spectrum.
    if cfg.roll_now_line {
        painter.line_segment(
            axes.across_pitch(split),
            egui::Stroke::new(1.0, theme::hairline()),
        );
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
