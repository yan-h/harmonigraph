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

/// How far the keyline stands proud of the note's outline on each side.
const KEYLINE_PX: f32 = 1.0;
/// The keyline's color: white, dimmed enough to read as an edge rather than
/// as a second outline competing with the note's own.
const KEYLINE: Color32 = Color32::from_rgba_premultiplied(150, 150, 150, 150);

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
    let window = time.window();
    let oldest = time.oldest();

    // Time gridlines first, so notes sit on top of them.
    if cfg.roll_grid_seconds > 0.0 {
        let step = cfg.roll_grid_seconds as f64;
        let mut t = step;
        // Cap the count as well as the span: a tiny step on a long window
        // would otherwise ask for thousands of hairlines.
        let mut drawn = 0;
        while t <= window && drawn < 400 {
            // Absolute-time lines across the whole take, or `now`-relative
            // lines receding into the past, per mode.
            let d = if time.whole_song() { time.depth_of(oldest + t) } else { time.depth_of(now - t) };
            painter.line_segment(axes.across_pitch(d), egui::Stroke::new(1.0, theme::surface_faint()));
            t += step;
            drawn += 1;
        }
    }

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
            // Sounding at a pitch the visible lattice has no node for. Flagged
            // the way the Notes pane flags it — the color you read the note by
            // goes to `warning_text`, over a `warning_bg` band — rather than
            // with a mark of its own, so the two panes say it the same way.
            // Same match the lattice uses, so all three agree on what
            // "off-lattice" means.
            let off_lattice = super::nearest_visible_node(
                &state.view,
                &state.tuning,
                lattice_core::PitchClass::from_cents(pitch.rem_euclid(12.0) * 100.0),
            )
            .is_none();
            let width = cfg.roll_outline_width.clamp(0.5, 8.0);
            // Bloom: a soft glow around the outline, driven by the SAME setting
            // as the lattice's bloom so the two panes share the look. egui has
            // no post-process pass like the lattice's wgpu bloom, so approximate
            // one — a couple of wider, fainter passes of the stroke under the
            // crisp one; more bloom widens and brightens the halo.
            let g = state.view.bloom_strength.clamp(0.0, 2.0);
            let body = |a: f32| {
                if off_lattice {
                    theme::warning_text().gamma_multiply(a)
                } else {
                    note_color(note, cfg, state, pitch, a)
                }
            };
            // (stroke width, color) per pass, painted in order; the crisp
            // outline is last and on top.
            let mut passes: Vec<(f32, Color32)> = Vec::with_capacity(4);
            if g > 0.0 {
                // The glow passes brighten, toward the bloom halo.
                passes.push((width + g * 3.0, brighten(body(alpha * 0.12 * g))));
                passes.push((width + g * 1.5, brighten(body(alpha * 0.20 * g))));
            }
            // A thin light keyline just outside the outline. The ribbons sit
            // over the spectrogram, whose cells run the whole ramp from black
            // to near-white, so a note's own color is sometimes almost exactly
            // what is behind it; this gives every note an edge to be seen by
            // whatever it crosses. Under the crisp outline and wider by a
            // pixel each side, so it reads as a rim rather than a thickening.
            passes.push((width + 2.0 * KEYLINE_PX, KEYLINE.gamma_multiply(alpha)));
            // The crisp outline is the note's TRUE color, so it matches the
            // same note on the lattice.
            passes.push((width, body(alpha)));
            // Off-lattice notes get the band behind them too — the ribbon is
            // an outline, so its inside is where the Notes pane's row
            // background belongs.
            let fill = if off_lattice {
                theme::warning_bg().gamma_multiply(alpha)
            } else {
                Color32::TRANSPARENT
            };

            if ribbon_px < MIN_RIBBON_PX {
                // Too thin to bound: the note IS its spine.
                for &(w, color) in &passes {
                    painter.line_segment(
                        [axes.at(a0, d0), axes.at(a1, d1)],
                        egui::Stroke::new(w.max(MIN_RIBBON_PX), color),
                    );
                }
            } else if p0 == p1 {
                // Unbent: a hollow axis-aligned rectangle (the only shape egui
                // will round the corners of).
                let rect = egui::Rect::from_two_pos(axes.at(a0 - half, d0), axes.at(a1 + half, d1));
                let radius = cfg.roll_rounding.clamp(0.0, 1.0) * ribbon_px * 0.5;
                let rounding = egui::CornerRadius::same(radius.min(127.0) as u8);
                if fill != Color32::TRANSPARENT {
                    painter.rect_filled(rect, rounding, fill);
                }
                for &(w, color) in &passes {
                    painter.rect_stroke(
                        rect,
                        rounding,
                        egui::Stroke::new(w, color),
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
                for (i, &(w, color)) in passes.iter().enumerate() {
                    painter.add(egui::Shape::convex_polygon(
                        quad.clone(),
                        if i == 0 { fill } else { Color32::TRANSPARENT },
                        egui::Stroke::new(w, color),
                    ));
                }
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

    /// Paint one held note and report every rect the roll emitted, as
    /// (stroke width, stroke color, fill).
    fn ribbon(tuning_offset: f32) -> Vec<(f32, Color32, Color32)> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.spectrum_config.roll_opacity = 1.0;
        state.spectrum_config.roll_velocity_alpha = false;
        state.spectrum_config.roll_outline_width = 2.0;
        state.spectrum_config.roll_thickness = 2.0;
        state.view.bloom_strength = 0.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        if tuning_offset != 0.0 {
            state.tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: tuning_offset },
            });
        }
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
                egui::Shape::Rect(r) => Some((r.stroke.width, r.stroke.color, r.fill)),
                _ => None,
            })
            .collect()
    }

    /// The ribbons lie over the spectrogram, whose cells run the whole ramp
    /// from black to near-white, so a note's own color is sometimes almost
    /// exactly what is behind it. Every note carries a light keyline under its
    /// outline, wider on both sides, to be seen by.
    #[test]
    fn every_note_carries_a_keyline_under_its_outline() {
        let rects = ribbon(0.0);
        let outline = 2.0;
        let key = rects.iter().position(|&(w, c, _)| {
            (w - (outline + 2.0 * KEYLINE_PX)).abs() < 0.01 && c == KEYLINE
        });
        let crisp = rects.iter().position(|&(w, _, _)| (w - outline).abs() < 0.01);
        let (Some(key), Some(crisp)) = (key, crisp) else {
            panic!("expected a keyline and a crisp outline, got {rects:?}");
        };
        assert!(key < crisp, "the keyline paints over the outline it should sit under");
    }

    /// A pitch the visible lattice has no node for is flagged the way the
    /// Notes pane flags it: the color you read the note by goes to
    /// `warning_text`, over a `warning_bg` band.
    #[test]
    fn an_off_lattice_note_takes_the_notes_pane_warning_colors() {
        let plain = ribbon(0.0);
        assert!(
            !plain.iter().any(|&(_, c, _)| c == theme::warning_text()),
            "a plain C should not be flagged",
        );
        // Half a semitone sharp: no lattice node matches that pitch class.
        let flagged = ribbon(0.5);
        assert!(
            flagged.iter().any(|&(_, c, _)| c == theme::warning_text()),
            "off-lattice note kept its own color: {flagged:?}",
        );
        assert!(
            flagged.iter().any(|&(_, _, fill)| fill == theme::warning_bg()),
            "off-lattice note has no band behind it",
        );
    }
}
