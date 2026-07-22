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

use super::scene_color;
use super::spectral::{Axes, PitchScale};
use crate::{theme, RollColor, SharedState};

/// A channel in the [`PitchGradient`](lattice_core::ChannelRole::PitchGradient)
/// role, used to borrow the lattice's low-to-high color ramp for the
/// roll's Pitch coloring — the ramp has no entry point of its own that
/// takes the display's darkest/brightest bounds.
const PITCH_RAMP_CHANNEL: u8 = 9;

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

    for note in state.tracker.roll().notes() {
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

        let live = note.is_live();
        for ((t0, p0), (t1, p1)) in note.segments(now) {
            let (t0, t1) = (t0.max(oldest), t1.max(oldest));
            if t1 < oldest {
                continue;
            }
            let (d0, d1) = (depth_of(t0), depth_of(t1));
            let (a0, a1) = (scale.t_of(p0), scale.t_of(p1));

            // Age fade is sampled once per segment, at its midpoint: a
            // note bent across the window gets a gradient for free, an
            // unbent one drawn as a single segment reads as one tone.
            let age = ((now - (t0 + t1) * 0.5) / window).clamp(0.0, 1.0) as f32;
            let mut alpha = opacity * (1.0 - cfg.roll_age_fade * age);
            if cfg.roll_velocity_alpha {
                alpha *= super::visibility_floor(note.velocity);
            }
            if live && cfg.roll_highlight_held {
                alpha = opacity;
            }
            if alpha <= 0.004 {
                continue;
            }
            let pitch = (p0 + p1) * 0.5;
            // Full-strength color for the outline (and thin spines); the fill
            // takes a fraction of it so the spectrogram — and the note's own
            // fundamental, which sits right under the ribbon — shows through.
            let color = note_color(note, cfg, state, pitch, alpha);
            let fill = note_color(note, cfg, state, pitch, alpha * cfg.roll_fill.clamp(0.0, 1.0));
            // A translucent fill would read as a vague smear without an edge, so
            // outline it whenever the fill is dialed back, on top of the setting.
            let outline_on = cfg.roll_outline || cfg.roll_fill < 0.999;

            if ribbon_px < MIN_RIBBON_PX {
                // Too thin to fill: a stroke down the note's spine (kept solid;
                // there's no interior to reveal).
                painter.line_segment(
                    [axes.at(a0, d0), axes.at(a1, d1)],
                    egui::Stroke::new(MIN_RIBBON_PX, color),
                );
            } else if p0 == p1 {
                // Unbent: an axis-aligned rectangle, which is the only
                // shape egui will round the corners of.
                let rect = egui::Rect::from_two_pos(axes.at(a0 - half, d0), axes.at(a1 + half, d1));
                let radius = cfg.roll_rounding.clamp(0.0, 1.0) * ribbon_px * 0.5;
                let rounding = egui::CornerRadius::same(radius.min(127.0) as u8);
                painter.rect_filled(rect, rounding, fill);
                if outline_on {
                    painter.rect_stroke(
                        rect,
                        rounding,
                        egui::Stroke::new(1.0, brighten(color)),
                        egui::StrokeKind::Inside,
                    );
                }
            } else {
                // Bent: a quad following the glide. Wound consistently so
                // egui's convex-polygon fill stays valid whichever way the
                // axes run.
                let quad = vec![
                    axes.at(a0 - half, d0),
                    axes.at(a0 + half, d0),
                    axes.at(a1 + half, d1),
                    axes.at(a1 - half, d1),
                ];
                let stroke = if outline_on {
                    egui::Stroke::new(1.0, brighten(color))
                } else {
                    egui::Stroke::NONE
                };
                painter.add(egui::Shape::convex_polygon(quad, fill, stroke));
            }
        }

        // Attack cap: a bright line across the ribbon at the note's start,
        // which is what makes a run of repeated notes countable.
        if cfg.roll_onsets && note.start >= oldest && ribbon_px >= MIN_RIBBON_PX {
            let d = depth_of(note.start);
            let t = scale.t_of(note.start_pitch());
            let color = note_color(note, cfg, state, note.start_pitch(), opacity);
            painter.line_segment(
                [axes.at(t - half, d), axes.at(t + half, d)],
                egui::Stroke::new(1.5, brighten(color)),
            );
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
