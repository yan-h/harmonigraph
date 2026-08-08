//! The Spectral pane — the audio FFT curve, the sounding voices, and the
//! piano roll of what has been played, all over a shared MIDI-pitch axis —
//! and its settings pane.
//!
//! Everything is drawn in an abstract *(pitch, depth)* plane and mapped to
//! the screen by [`Axes`] at the last moment, so the whole pane
//! turns together when its orientation changes and no element has to know
//! which way is up.
//!
//! A directory rather than a file because this is four drawn layers over one
//! plane, not one pane: [`roll`], [`spectrogram`] and [`names`] each fill a
//! share of the same depth axis, and all three read the same [`axes`]. Held
//! in one file, that shared coordinate system was reachable only as a pane's
//! insides, and the pane was the most-edited file in the crate.

pub(crate) mod axes;
mod gestures;
pub(crate) mod names;
pub(crate) mod roll;
pub(crate) mod spectrogram;
mod settings;

pub(super) use settings::spectrum_settings_pane;

use crate::{theme, SharedState};
use crate::panes::nearest_visible_node;
use axes::{
    loudness, spectrum_share, text_scales, Axes, PitchScale, TimeAxis,
    MARKING_PT, PLOT_HEIGHT_FRACTION,
};
use gestures::{drag_split, drag_zoom};
use egui::Sense;

/// Three views of the same music over one shared MIDI-pitch axis: the
/// audio spectrum as a curve (FFT of the input bus, every partial at its
/// actual pitch), the sounding MIDI voices as bars, and the piano roll of
/// what has been played. All are optional; the settings live in
/// [`spectrum_settings_pane`].
///
/// The depth axis is shared out between the roll (the far end) and the
/// spectrum (the baseline end) at `split`, which is also where the voice
/// bars hang from: a note crosses that one line out of the roll and into
/// the spectrum peak it is making.
///
/// A pointer over this pane changes nothing on it and nothing on the
/// lattice, in either direction. Hovering here to light the matching lattice
/// node is the version that shipped and was withdrawn: the pitch axis is
/// continuous and about fourteen cents wide per point, so a pointer on it
/// does not aim at a node so much as land near one, and the highlight read
/// as a label flickering at unrelated nodes rather than as an answer.
/// Lighting a band here for the lattice-hovered pitch class is the same
/// trade the other way round — in every octave, it puts a stripe across the
/// whole picture, too loud an answer to a pointer resting somewhere else.
pub(crate) fn spectral_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    now: f64,
    // Spectrogram texture slot: 0 the docked pane / offline render, 1 the
    // Render preview, so two live copies don't clobber one shared texture.
    surface: usize,
) {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let cfg = state.spectrum_config;
    // Drag-sensing, so the pitch range can be panned and the Span or the Level
    // zoomed by grabbing the picture (see `drag_zoom`). Registered BEFORE the
    // divider's own band, which
    // is what leaves the divider on top where the two overlap: egui hands a
    // drag to the last widget registered over the pointer.
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::drag());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::well());

    let axes = Axes::new(rect, &cfg);

    // Offline playhead render: the whole take laid out statically with a
    // sweeping playhead. It takes the whole pane (split = 0), which also drops
    // the live curve and voice bars via their `split > 0` guards — leaving the
    // spectrogram, roll, and playhead.
    let whole_song = state.whole_song.is_some();
    // The divider is grabbable whenever the far region is turned ON, even
    // where it has been dragged shut (`roll_fraction` 0 or 1) — otherwise
    // shutting it would be one-way. Whole-song has no divider: the spectrum
    // isn't drawn at all there.
    let divider = (!whole_song && (cfg.show_roll || cfg.show_spectrogram))
        .then(|| drag_split(ui, &axes, state, surface));
    drag_zoom(ui, &axes, &response, state, surface);
    // Re-snapshot: the two drags above just wrote `roll_fraction`, the pitch
    // range, the Span or the level ceiling, and everything below has to be this
    // frame's values, not the ones from before the drag. The ceiling is the
    // sharpest case — it is what `loudness` maps through, so a stale copy would
    // draw the curve and the heatmap a frame behind the hand dragging them.
    let cfg = state.spectrum_config;
    let split = if whole_song { 0.0 } else { spectrum_share(&cfg) };

    // The axis: absolute pitch, linear in MIDI note = logarithmic in
    // frequency, so every octave gets equal room and every note draws at
    // its actual pitch. The displayed range is the Analyzer tab's pitch
    // range, which is free to start anywhere — it is not snapped to C.
    let min_midi = cfg.low_midi;
    // Never trust the pair to be ordered. A zero or negative span divides by
    // zero in PitchScale and paints NaN geometry, which egui panics on — and
    // a panic here takes the plugin's editor down inside the host. The range
    // bar can't produce one; a hand-edited state blob can.
    let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
    let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
    // Everything this pane sets text at, decided once from the range it just
    // settled on: the markings hold their size, the names follow the zoom.
    let text = text_scales(&cfg, &axes, scale.span, painter.ctx().pixels_per_point());
    // dB depth mapping: 0 dB (a full-scale sine) tops out at 85% of the
    // spectrum's share; the Analyzer tab's floor sets the bottom. Tilt is
    // the conventional reference slope (negative), so the display
    // SUBTRACTS it per octave above the 1 kHz pivot: -4.5 lifts treble
    // 4.5 dB/oct.
    let d_of = |power: f32, midi: f32| loudness(&cfg, power, midi) * split * PLOT_HEIGHT_FRACTION;
    // The spectrum joins the spectrogram: its region mirrors so the baseline
    // sits on the now-line (against the spectrogram's newest column) and the
    // peaks point outward. With no roll/spectrogram (split == 1) there's
    // nothing to join, so it stands up from the outer edge as usual.
    let joined = split < 1.0;
    let sd = |d: f32| if joined { split - d } else { d };
    // Labels ride the baseline: the now-line when joined (offsetting into the
    // spectrum, whichever way that runs), else the outer edge. Whole-song has
    // no spectrum to join, so its labels ride the near edge like the latter.
    let (label_d, label_into) =
        if joined && !whole_song { (split, -2.0) } else { (0.0, 2.0) };

    // A uniform dark bed under the whole spectrogram region, so it reads as one
    // surface. The heatmap mesh only covers the depths that actually have
    // columns, and its silence is black; without this bed the un-covered depths
    // (before history fills the window, or past its oldest column) show the
    // lighter pane `well` in jarring patches. Black is the heatmap's own silence
    // color, so covered and un-covered silence match whatever the quad is tinted
    // with: `Color32` is premultiplied, so a black texel over this bed
    // composites to black at every alpha.
    if cfg.show_spectrogram && split < 1.0 {
        let bed = egui::Rect::from_two_pos(axes.at(0.0, split), axes.at(1.0, 1.0));
        painter.rect_filled(bed, 0.0, egui::Color32::BLACK);
    }

    // Axis markings: the analyzer-standard 1-2-5 frequency series, and only
    // that. The alternative is a mark at every C with Bitwig octave numbers,
    // which answers a question the pane answers better elsewhere — every ribbon
    // carries its note NAME, spelled the lattice's way and placed at the pitch
    // that is sounding. What an axis is for is the other reading: where in the
    // spectrum a band sits, which is a frequency. Numbers only, no gridline: a
    // line run the full depth would outrun the spectrogram's heatmap (which
    // only grows out from the now-line as history accumulates) and sit bare on
    // the bed ahead of it, and a line stopped at the data would still cross the
    // live spectrum curve and the roll's ribbons for no reading a number
    // doesn't already give.
    let mut axis_labels: Vec<(f32, String)> = Vec::new();
    for hz in [20.0f32, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0] {
        let midi = hz_to_midi(hz);
        if !scale.contains(midi) {
            continue;
        }
        let t = scale.t_of(midi);
        let label = if hz >= 1_000.0 { format!("{}k", hz / 1_000.0) } else { format!("{hz}") };
        axis_labels.push((t, label));
    }

    // Nothing to pump here: the analyzer runs off the samples the shell pushes
    // (see `AudioSpectrum::push_samples`), so the spectrogram's columns arrive
    // whether or not the curve is drawn — and whether or not this pane is.
    // The far share of the depth axis: a spectrogram heatmap of the audio
    // and/or the piano roll of what has been played, both on the same
    // `now`-anchored time axis. The spectrogram lays down first (it's a
    // background) and the roll's ribbons sit over it. The spectrum curve is
    // drawn between the two and does not come into that order at all: it has
    // the near share of the axis to itself, so the only place anything it
    // paints can meet anything the roll paints is ON the line dividing them —
    // which is why the marks for `now` go after all three. Turning the ribbons
    // off (`show_roll`) with the spectrogram on leaves the heatmap alone.
    if split < 1.0 && cfg.show_spectrogram {
        spectrogram::draw_spectrogram(&painter, &axes, &scale, state, split, now, surface);
    }

    // Audio spectrum: the FFT of the shell's audio source, every partial
    // at its actual pitch. Fundamentals line up under their voice bars;
    // the harmonic series marches up the axis from each note.
    if split > 0.0 {
        if let Some(levels) = state.spectrum.display(now) {
            // Only the buckets inside the pitch range.
            // One slab per pitch PIXEL, each taking the loudest bucket that
            // falls in it — not one slab per bucket. The axis holds thousands
            // of buckets and the pane a few hundred pixels, so per-bucket
            // meant thousands of shapes a frame stacked on top of each other,
            // which was survivable only while most buckets were zero. MAX
            // rather than an average so a thin partial still reads full
            // height instead of being diluted by its quiet neighbours.
            let bucket_at = |midi: f32| {
                (((midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as isize)
                    .clamp(0, levels.len() as isize - 1) as usize
            };
            let cols = (axes.pitch_len().round() as usize).clamp(2, 4096);
            let visible: Vec<(f32, f32, f32)> = (0..cols)
                .map(|c| {
                    let edge = |i: usize| scale.min_midi + scale.span * i as f32 / cols as f32;
                    let (b0, b1) = (bucket_at(edge(c)), bucket_at(edge(c + 1)));
                    let level =
                        levels[b0..=b1.max(b0)].iter().fold(0.0f32, |a, &b| a.max(b));
                    let t = (c as f32 + 0.5) / cols as f32;
                    (scale.min_midi + t * scale.span, t, level)
                })
                .collect();

            // Color from the SAME palette as the spectrogram, keyed by the same
            // loudness, so the curve reads in the heatmap's scheme rather than a
            // flat accent. `tint` keeps the palette's hue/brightness and only
            // sets opacity (gamma_multiply would darken it toward black).
            let hue = |power: f32, midi: f32| {
                spectrogram::cell_color(cfg.spectrogram_color, loudness(&cfg, power, midi))
            };
            let tint = |c: egui::Color32, a: u8| {
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
            };

            // The spectrum is a filled shape, like the spectrogram — no outline
            // curve. Each slab is one bucket in its own palette color, opaque
            // enough to read as a solid fill; densely packed, their tops make
            // the shape's edge (no separate line to fray).
            let slab = axes.pitch_len() / cols as f32 + 0.5;
            for &(midi, t, level) in &visible {
                let d = d_of(level, midi);
                if d * axes.depth_len() > 0.5 {
                    painter.line_segment(
                        [axes.at(t, sd(0.0)), axes.at(t, sd(d))],
                        egui::Stroke::new(slab, tint(hue(level, midi), 210)),
                    );
                }
            }

            // ...and a light rim along their tops, the same edge the note
            // ribbons carry. The spectrum's own colors come from the
            // spectrogram's palette, so where the curve is quiet it is drawn
            // in that palette's dark end — against the pane's dark background,
            // with no edge, the shape simply stops existing. Follows the
            // profile the slabs make rather than being a separate curve.
            if let Some(edge) = roll::keyline(&cfg, 1.0) {
                let top: Vec<egui::Pos2> = visible
                    .iter()
                    .map(|&(midi, t, level)| axes.at(t, sd(d_of(level, midi))))
                    .collect();
                painter.add(egui::Shape::line(top, egui::Stroke::new(1.0, edge)));
            }
        }
    }

    // Notes sounding at a pitch the visible lattice has no node for, flagged
    // as a red band down the spectrum at that pitch. The lattice shows nothing
    // for such a note by definition, so this pane is where you would otherwise
    // never learn one was playing — and the band says it in the spectrum's own
    // territory, where there is room for it, instead of recoloring the note
    // and costing you the one thing the ribbon's color is for. Same
    // `nearest_visible_node` match the Notes pane and the lattice use.
    if split > 0.0 && !whole_song {
        let mut voices: Vec<&harmonigraph_core::Voice> = state
            .tracker
            .voices()
            .filter(|v| {
                nearest_visible_node(&state.view, &state.tuning, v.pitch_class).is_none()
            })
            .collect();
        // Translucent bands accumulate where they overlap, so the paint
        // order is part of the picture. The tracker's own order is stable —
        // held voices by (channel, note), then the release tail — but it is
        // not this one: pitch decides which band is on top here.
        voices.sort_unstable_by(|a, b| {
            a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
        });
        let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
        // One envelope for the whole roll, as every other caller takes it: it
        // is a property of the view and the frame, and rebuilding it per voice
        // would read as if it could vary between them.
        let env = state.view.envelope(&state.frame_params);
        for voice in voices {
            let strength = voice.activation(now, &env);
            if strength <= 0.0 || !scale.contains(voice.pitch) {
                continue;
            }
            let t = scale.t_of(voice.pitch);
            let band = egui::Rect::from_two_pos(
                axes.at(t - half, 0.0),
                axes.at(t + half, split),
            );
            painter.rect_filled(band, 0.0, theme::warning_text().gamma_multiply(0.3 * strength));
        }
    }

    // The roll. Its ribbons occupy the far side of the split and the spectrum
    // the near side, so the only thing this shares with the line below is what
    // happens ON the line: a sounding note's ribbon ends square on it.
    if split < 1.0 && cfg.show_roll {
        roll::draw_roll(&painter, &axes, &scale, state, split, now, surface);
    }

    // The now-line, where the roll hands over to the spectrum — drawn after
    // all three things it divides rather than at the end of the roll. It marks
    // the boundary between two pictures, so it has to sit ON them, and each of
    // the three arrives at it from its own side: the spectrum curve's fill
    // paints down to it, the spectrogram's quad reaches it from the far side,
    // and a sounding note's ribbon stops on it. Any of them eating into the
    // line leaves a divider that thins and flickers with the music instead of
    // holding still — and the roll is the worst of the three, taking half the
    // line's width under every ribbon that is sounding, which is exactly where
    // the picture is busiest and the boundary hardest to follow.
    //
    // The cost is the ribbon end, covered by the line rather than painting
    // across it. What a sounding note has to show is that it reaches the
    // boundary and comes out the other side as the spectrum peak it is making,
    // and it still shows that; an unbroken line is what makes the two sides one
    // picture to read across, and a line chewed away wherever a note crosses
    // reads as a drawing error rather than as the note.
    //
    // Always drawn (there is no setting): the handover is what the pane is
    // built around, so the boundary is marked whether or not anything is
    // sounding on it.
    if !whole_song && split < 1.0 && split > 0.0 {
        painter.line_segment(axes.across_pitch(split), egui::Stroke::new(1.0, theme::hairline()));
    }

    // Whole-song mode marks `now` with a playhead instead — the one moving
    // thing sweeping across a static spectrogram and roll — and it goes here,
    // beside the line it replaces, for the identical reason. This mode gives
    // the roll the WHOLE depth axis (`split` is 0), so the playhead crosses
    // every ribbon on the pane rather than meeting them end-on: drawn under
    // the roll it comes out dashed, notched by each note it passes, and this
    // is the mark the mode is built around. It is also the render behind
    // `--playhead` video export, where a notched sweep is baked into a file.
    if whole_song {
        let time = TimeAxis::new(state, split, now);
        painter.line_segment(
            axes.across_pitch(time.playhead_depth()),
            egui::Stroke::new(1.5, theme::accent()),
        );
    }

    // Axis labels last, riding on top of the spectrogram, spectrum, and
    // voice bars: a label only earns its place if you can read which
    // frequency it marks, and a loud slab would otherwise bury it.
    // Haloed exactly like the lattice's node labels, and for the same reason:
    // whatever is behind them is a picture, not a background. A pitch label
    // over a bright spectrogram slab, or over the spectrum's own fill, has no
    // contrast to rely on at all.
    let mut labels = crate::text::TextBatch::default();
    for (p, label) in axis_labels {
        let (pos, align) = axes.text_anchor(p, label_d, 3.0, label_into);
        labels.text(
            &painter,
            pos,
            align,
            label,
            egui::FontId::monospace(MARKING_PT * text.markings),
            theme::text_dim(),
            theme::well(),
        );
    }
    // Each note's own name, over the ribbon it belongs to. In the same batch
    // as the axis labels, and so over the same pictures: a name that could be
    // buried by a loud slab — or by the ribbon it is naming — names nothing.
    let note_names = names::plan(state, &axes, &scale, split, now, text.names);
    names::draw(&painter, &note_names, text.names, &mut labels);
    // Flushed before the divider: a batch is drawn where it is flushed, and
    // the divider belongs over the plots, not under the names.
    labels.flush(&painter, rect, state, crate::text::spectral_labels(surface));

    // The divider, over the plots so it stays findable against a loud
    // spectrogram. Nothing at rest — the roll's now-line already marks where
    // it is, and the offline render (which has no pointer) must keep emitting
    // exactly the shapes it always did.
    if let Some(divider) = &divider {
        let lit = if divider.dragged() {
            Some(theme::accent())
        } else if divider.hovered() {
            Some(theme::accent_edge())
        } else {
            None
        };
        if let Some(color) = lit {
            painter.line_segment(axes.across_pitch(split), egui::Stroke::new(2.0, color));
        }
    }
}

#[cfg(test)]
mod tests;
