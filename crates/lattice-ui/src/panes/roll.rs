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
pub(super) const KEYLINE_PX: f32 = 1.0;

/// The light edge drawn around a note's outline and along the spectrum's
/// profile, at `cfg.keyline` strength — `None` when the setting is off.
///
/// Both sit over the spectrogram, whose ramps run from black to near-white,
/// so either can end up almost exactly the brightness of what is behind it and
/// lose its shape entirely. A light rim gives them an edge to be seen by
/// whatever they cross. It is a setting because how much is right depends
/// entirely on the palette and opacity in play.
pub(super) fn keyline(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    let strength = cfg.keyline.clamp(0.0, 1.0) * alpha;
    (strength > 0.004).then(|| Color32::WHITE.gamma_multiply(strength))
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
            let width = cfg.roll_outline_width.clamp(0.5, 8.0);
            // Bloom: a soft glow around the outline, driven by the SAME setting
            // as the lattice's bloom so the two panes share the look. egui has
            // no post-process pass like the lattice's wgpu bloom, so approximate
            // one — a couple of wider, fainter passes of the stroke under the
            // crisp one; more bloom widens and brightens the halo.
            let g = state.view.bloom_strength.clamp(0.0, 2.0);
            let body = |a: f32| note_color(note, cfg, state, pitch, a);
            // (stroke width, color) per pass, painted in order; the crisp
            // outline is last and on top.
            let mut passes: Vec<(f32, Color32)> = Vec::with_capacity(4);
            if g > 0.0 {
                // The glow passes brighten, toward the bloom halo.
                passes.push((width + g * 3.0, brighten(body(alpha * 0.12 * g))));
                passes.push((width + g * 1.5, brighten(body(alpha * 0.20 * g))));
            }
            // A thin light keyline just outside the outline, at the strength
            // the Edge setting asks for. See `keyline`.
            if let Some(edge) = keyline(cfg, alpha) {
                passes.push((width + 2.0 * KEYLINE_PX, edge));
            }
            // The crisp outline is the note's TRUE color, so it matches the
            // same note on the lattice.
            passes.push((width, body(alpha)));

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
                for &(w, color) in &passes {
                    painter.add(egui::Shape::convex_polygon(
                        quad.clone(),
                        Color32::TRANSPARENT,
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

    /// Paint one held note at the given Edge strength and report every rect
    /// the roll emitted, as (stroke width, stroke color, fill).
    fn ribbon(keyline: f32) -> Vec<(f32, Color32, Color32)> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Horizontal;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
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
                egui::Shape::Rect(r) => Some((r.stroke.width, r.stroke.color, r.fill)),
                _ => None,
            })
            .collect()
    }

    /// The ribbons lie over the spectrogram, whose cells run the whole ramp
    /// from black to near-white, so a note's own color is sometimes almost
    /// exactly what is behind it. The Edge setting puts a light keyline under
    /// each outline, wider on both sides, to be seen by.
    #[test]
    fn the_edge_setting_puts_a_keyline_under_each_outline() {
        let outline = 2.0;
        let rects = ribbon(0.5);
        let key = rects.iter().position(|&(w, c, _)| {
            (w - (outline + 2.0 * KEYLINE_PX)).abs() < 0.01
                && c == Color32::WHITE.gamma_multiply(0.5)
        });
        let crisp = rects.iter().position(|&(w, _, _)| (w - outline).abs() < 0.01);
        let (Some(key), Some(crisp)) = (key, crisp) else {
            panic!("expected a keyline and a crisp outline, got {rects:?}");
        };
        assert!(key < crisp, "the keyline paints over the outline it should sit under");

        // And nothing at all at zero, rather than a hairline that can't be
        // turned off.
        let none = ribbon(0.0);
        assert!(
            !none.iter().any(|&(w, _, _)| (w - (outline + 2.0 * KEYLINE_PX)).abs() < 0.01),
            "Edge 0 still drew a keyline: {none:?}",
        );
    }
}
