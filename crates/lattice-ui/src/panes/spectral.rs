//! The Spectral pane — the audio FFT curve and the sounding voices, over
//! a shared MIDI-pitch axis — and its settings pane.

use super::visibility_floor;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::{theme, SharedState};
use super::{nearest_visible_node, KEY_NAMES};
use lattice_core::notes::{display_octave_of, octave_start_midi};
use egui::Sense;
use lattice_scene::channel_color;

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state).
pub(super) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{SpectrumLabels, SpectrumWindow};
    let cfg = &mut state.spectrum_config;

    ui.checkbox(&mut cfg.show_audio, "Audio spectrum").on_hover_text(
        "Analyze and overlay the audio's spectrum, every partial at its \
         actual pitch (plugin: the input bus; standalone: a synth on the \
         held notes)",
    );

    button_row(ui, |ui| {
        ui.label("Window");
        for (window, label) in [
            (SpectrumWindow::Fast, "Fast"),
            (SpectrumWindow::Balanced, "Balanced"),
            (SpectrumWindow::Precise, "Precise"),
        ] {
            ui.selectable_value(&mut cfg.window, window, label).on_hover_text(format!(
                "{} samples: {}",
                window.samples(),
                match window {
                    SpectrumWindow::Fast => "snappy response, coarse bass pitch",
                    SpectrumWindow::Balanced => "the default tradeoff",
                    SpectrumWindow::Precise => "sharp bass pitch, slower response",
                },
            ));
        }
    });

    ValueBar::new(&mut cfg.floor_db, -90.0..=-30.0, "Floor (dB)")
        .show(ui)
        .on_hover_text("Bottom of the height scale; a full-scale sine is 0 dB");
    ValueBar::new(&mut cfg.smoothing, 0.0..=0.9, "Smoothing")
        .show(ui)
        .on_hover_text("Display inertia: 0 reacts instantly, 0.9 glides");
    // Tilt: conventional stepped reference slopes. Snap stray persisted
    // values (e.g. from the short-lived continuous bar) onto a step.
    if !crate::TILT_STEPS.contains(&cfg.tilt) {
        cfg.tilt = crate::TILT_STEPS
            .into_iter()
            .min_by(|a, b| (a - cfg.tilt).abs().total_cmp(&(b - cfg.tilt).abs()))
            .unwrap_or(0.0);
    }
    button_row(ui, |ui| {
        ui.label("Tilt (dB/oct)").on_hover_text(
            "Reference slope (dB/oct) that displays flat: 0 = raw power, \
             -3 flattens pink noise, -4.5 flattens typical material",
        );
        for step in crate::TILT_STEPS {
            ui.selectable_value(&mut cfg.tilt, step, format!("{step:.1}"));
        }
    });

    choice_row(
        ui,
        "Labels",
        &mut cfg.labels,
        &[
            (SpectrumLabels::Notes, "Notes", "A gridline at every C, Bitwig octave numbers"),
            (
                SpectrumLabels::Frequency,
                "Frequency",
                "Gridlines at 20, 50, 100 ... 10k, 20k Hz",
            ),
        ],
    );

    ui.checkbox(&mut cfg.fill, "Fill")
        .on_hover_text("Shade under the spectrum curve");
    ui.checkbox(&mut cfg.peak_hold, "Peak hold")
        .on_hover_text("Keep a decaying outline at each pitch's recent maximum");
    ui.checkbox(&mut cfg.show_voice_bars, "Voice bars")
        .on_hover_text("MIDI-derived bars at each voice's actual pitch");

    // Octave zoom (Bitwig numbering; C-1..C9 is the full analyzer range).
    let mut low = cfg.low_octave as f32;
    if ValueBar::new(&mut low, -1.0..=8.0, "Low octave").integer().show(ui).changed() {
        cfg.low_octave = low as i32;
        cfg.high_octave = cfg.high_octave.max(cfg.low_octave + 1);
    }
    let mut high = cfg.high_octave as f32;
    if ValueBar::new(&mut high, 0.0..=9.0, "High octave").integer().show(ui).changed() {
        cfg.high_octave = high as i32;
        cfg.low_octave = cfg.low_octave.min(cfg.high_octave - 1);
    }
}

/// Height budget for both plots, as a fraction of the pane: a full-scale
/// (0 dB) sine and a full-velocity held voice both top out here. The two
/// MUST agree, or the voice bars stop being comparable with the spectrum
/// curve they are drawn over.
const PLOT_HEIGHT_FRACTION: f32 = 0.85;

/// The 1 kHz pivot of the tilt slope, as a MIDI pitch.
const TILT_PIVOT_MIDI: f32 = 83.213_1;

/// Two views of the same sound over one shared MIDI-pitch axis: the
/// audio spectrum as a curve (FFT of the input bus, every partial at its
/// actual pitch) and the sounding MIDI voices as bars. Both are optional;
/// the settings live in [`spectrum_settings_pane`].
///
/// Hover sync goes both ways: the lattice-hovered pitch class shows as a
/// band here, and hovering a pitch here highlights the matching lattice
/// node (if one is in view).
pub(super) fn spectral_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    use crate::SpectrumLabels;
    use lattice_core::spectrum::{hz_to_midi, midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let cfg = state.spectrum_config;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::well());

    // The axis: absolute pitch, linear in MIDI note = logarithmic in
    // frequency, so every octave gets equal width and every note draws at
    // its actual pitch. The displayed range is the Spectrum tab's octave
    // zoom; the Bitwig octave<->MIDI convention lives in lattice-core.
    let min_midi = octave_start_midi(cfg.low_octave) as f32;
    let max_midi = octave_start_midi(cfg.high_octave) as f32;
    let span = max_midi - min_midi;
    let x_of = |midi: f32| rect.left() + rect.width() * (midi - min_midi) / span;
    // dB height mapping: 0 dB (a full-scale sine) tops out at 85% of the
    // pane; the Spectrum tab's floor sets the bottom. Tilt is the
    // conventional reference slope (negative), so the display SUBTRACTS
    // it per octave above the 1 kHz pivot: -4.5 lifts treble 4.5 dB/oct.
    let h_of = |power: f32, midi: f32| {
        let db = 10.0 * power.max(1e-12).log10()
            - cfg.tilt * (midi - TILT_PIVOT_MIDI) / 12.0;
        ((db - cfg.floor_db) / -cfg.floor_db).clamp(0.0, 1.0)
            * rect.height()
            * PLOT_HEIGHT_FRACTION
    };

    // Axis gridlines: every C (note labels) or the analyzer-standard
    // 1-2-5 frequency series, per the Spectrum tab.
    match cfg.labels {
        SpectrumLabels::Notes => {
            let mut c = min_midi as i32;
            while c <= max_midi as i32 {
                let x = x_of(c as f32);
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, theme::panel()),
                );
                if c < max_midi as i32 {
                    painter.text(
                        egui::pos2(x + 3.0, rect.bottom() - 2.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("C{}", display_octave_of(c)),
                        egui::FontId::monospace(10.0),
                        theme::text_dim(),
                    );
                }
                c += 12;
            }
        }
        SpectrumLabels::Frequency => {
            for hz in
                [20.0f32, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0]
            {
                let midi = hz_to_midi(hz);
                if !(min_midi..=max_midi).contains(&midi) {
                    continue;
                }
                let x = x_of(midi);
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, theme::panel()),
                );
                let label = if hz >= 1_000.0 {
                    format!("{}k", hz / 1_000.0)
                } else {
                    format!("{hz}")
                };
                painter.text(
                    egui::pos2(x + 3.0, rect.bottom() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    egui::FontId::monospace(10.0),
                    theme::text_dim(),
                );
            }
        }
    }

    // Cross-pane highlight: the pitch class hovered in ANY pane shows as
    // a tolerance-wide band in every octave.
    if let Some(pos) = state.hovered {
        let semis = state.tuning.pitch_class(pos).to_cents() / 100.0;
        let half_width = (rect.width() * (state.tuning.tolerance_cents() / 100.0) / span).max(1.5);
        let mut midi = min_midi + semis;
        while midi < max_midi {
            let x = x_of(midi);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x - half_width, rect.top()),
                    egui::pos2(x + half_width, rect.bottom()),
                ),
                0.0,
                theme::accent_fill(),
            );
            midi += 12.0;
        }
    }

    // Audio spectrum: the FFT of the shell's audio source, every partial
    // at its actual pitch. Fundamentals line up under their voice bars;
    // the harmonic series marches up to the right of each note.
    if cfg.show_audio {
        if let Some((levels, peaks)) = state.spectrum.display(now, &cfg) {
            // Only the buckets inside the octave zoom.
            let visible: Vec<(f32, f32, f32, f32)> = (0..levels.len())
                .filter_map(|i| {
                    let midi = SPECTRUM_MIN_MIDI + (i as f32 + 0.5) / BINS_PER_SEMITONE as f32;
                    (min_midi..=max_midi)
                        .contains(&midi)
                        .then(|| (midi, x_of(midi), levels[i], peaks[i]))
                })
                .collect();

            if cfg.fill {
                // One translucent vertical slab per bucket, wide enough to
                // meet its neighbors.
                let slab = (rect.width() / (span * BINS_PER_SEMITONE as f32)) + 0.5;
                let fill_color = theme::accent().gamma_multiply(0.15);
                for &(midi, x, level, _) in &visible {
                    let h = h_of(level, midi);
                    if h > 0.5 {
                        painter.line_segment(
                            [egui::pos2(x, rect.bottom()), egui::pos2(x, rect.bottom() - h)],
                            egui::Stroke::new(slab, fill_color),
                        );
                    }
                }
            }
            if cfg.peak_hold {
                let peak_points: Vec<egui::Pos2> = visible
                    .iter()
                    .map(|&(midi, x, _, peak)| egui::pos2(x, rect.bottom() - h_of(peak, midi)))
                    .collect();
                painter.add(egui::Shape::line(
                    peak_points,
                    egui::Stroke::new(1.0, theme::accent().gamma_multiply(0.35)),
                ));
            }
            let points: Vec<egui::Pos2> = visible
                .iter()
                .map(|&(midi, x, level, _)| egui::pos2(x, rect.bottom() - h_of(level, midi)))
                .collect();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.5, theme::accent().gamma_multiply(0.65)),
            ));
        }
    }

    // Voice bars at the voice's ACTUAL pitch (per-note tuning and MPE
    // bends slide the bar): length follows the same envelope as the
    // lattice glow, weighted by velocity; color matches the node color.
    // They hang from the TOP: the audio spectrum's fundamental rises from
    // the bottom at the same x, so a bottom-up bar sat exactly on the peak
    // it should be compared against. Hanging bars point down at the peak
    // instead of hiding it.
    if cfg.show_voice_bars {
        for voice in state.tracker.voices() {
            let activation = voice.activation(now, state.frame_params.fade_time);
            if activation <= 0.0 || !(min_midi..=max_midi).contains(&voice.pitch) {
                continue;
            }
            let x = x_of(voice.pitch);
            let height =
                rect.height() * PLOT_HEIGHT_FRACTION
                    * activation
                    * visibility_floor(voice.velocity);
            let c = channel_color(
                voice.channel,
                voice.pitch,
                state.frame_params.darkest_pitch,
                state.frame_params.brightest_pitch,
            );
            let color = egui::Color32::from_rgb(
                (c.x * 255.0) as u8,
                (c.y * 255.0) as u8,
                (c.z * 255.0) as u8,
            );
            painter.line_segment(
                [
                    egui::pos2(x, rect.top()),
                    egui::pos2(x, rect.top() + height),
                ],
                egui::Stroke::new(3.0, color),
            );
        }
    }

    // Hovering here highlights the matching lattice node (if in view) and
    // reads out the pitch under the cursor.
    if let Some(pointer) = response.hover_pos() {
        let midi = (min_midi + (pointer.x - rect.left()) / rect.width() * span)
            .clamp(min_midi, max_midi);
        // The axis starts on a C, so cents-from-C is just the fractional
        // octave position.
        let pc_cents = (midi - min_midi).rem_euclid(12.0) * 100.0;
        state.hovered = nearest_visible_node(
            &state.view,
            &state.tuning,
            lattice_core::PitchClass::from_cents(pc_cents),
        );
        let nearest = midi.round();
        painter.text(
            egui::pos2(pointer.x + 6.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{}{} {:+.0}\u{a2} \u{b7} {:.1} Hz",
                KEY_NAMES[nearest as usize % 12],
                display_octave_of(nearest as i32),
                (midi - nearest) * 100.0,
                midi_to_hz(midi),
            ),
            egui::FontId::monospace(10.5),
            theme::text(),
        );
    }
}
