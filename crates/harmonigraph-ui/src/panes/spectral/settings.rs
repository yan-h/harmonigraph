//! The Display tab's Analyzer page: every setting the Spectral display
//! carries, and the readouts its bars are dragged against. The heatmap's
//! colors are the one thing dialled elsewhere — they are a color table, and
//! both of those are on the Colors page ([`super::super::color`]).

use crate::config::BALLISTICS_MAX;
use crate::panes::{edge_bar, section};
use crate::params::{AnalysisInput, ParamBackend};
use crate::widgets::{button_row, choice_row, option_label, RangeBar, ValueBar};
use crate::SharedState;

/// A MIDI note as the frequency an analyzer would label it: whole hertz down
/// low, kHz to one decimal above 1000, each carrying its unit so the number
/// says what it is. Three or four significant figures is all a range readout
/// can use — "16744 Hz" is noise where "16.7 kHz" is a number you can read at
/// a glance while dragging.
pub(super) fn hz_readout(midi: f32) -> String {
    let hz = harmonigraph_core::spectrum::midi_to_hz(midi);
    if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

/// A level as the range bar reads it out. Whole dB: the scale spans a
/// hundred of them and is dragged by eye, so a decimal place is a digit that
/// only ever moves.
fn db_readout(db: f32) -> String {
    format!("{db:.0} dB")
}

/// History is displayed and entered in seconds across the whole range.
pub(super) fn span_readout(seconds: f32) -> String {
    format!("{:.1} s", seconds.max(0.0))
}

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state). The Display tab's Analyzer page.
pub(crate) fn spectrum_settings_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
) {
    use crate::{SpectralOrientation, SpectrumTapers, SpectrumWindow};

    // What the page reaches, said once at the top rather than repeated on the
    // sections: every setting here is the analyzer's, and the Spiral is the
    // same analyzed frame wound onto a disc rather than a second one.
    ui.weak(
        "Audio analysis is shared by the Analyzer, Spiral and lattice audio rings. \
                 View and history controls arrange the Analyzer picture.",
    );

    ui.heading("View");
    let cfg = &mut state.spectrum_config;
    // Named for the side the now-line is on, which is where the spectrum sits
    // and where a note arrives — so the setting says where to LOOK rather than
    // which way the picture travels. There is no Auto: it followed the pane's
    // shape, and a pane that turns itself over when a window is dragged past
    // square is one you cannot dial a video's look in on.
    // Off `ALL` with an exhaustive match, not a hand-written list of four: both
    // are built from the enum, so a fifth side cannot reach the pane without a
    // name and a hint of its own.
    let sides = SpectralOrientation::ALL.map(|side| {
        let (label, hint) = match side {
            SpectralOrientation::Left => {
                ("Left", "Spectrum on the left; time scrolls rightward, pitch climbs")
            }
            SpectralOrientation::Right => {
                ("Right", "Spectrum on the right; time scrolls leftward, pitch climbs")
            }
            SpectralOrientation::Top => {
                ("Top", "Spectrum on top; time scrolls downward, pitch runs left to right")
            }
            SpectralOrientation::Bottom => (
                "Bottom",
                "Spectrum along the bottom; time scrolls upward, pitch runs left to right",
            ),
        };
        (side, label, hint)
    });
    choice_row(ui, "Spectrum position", &mut cfg.orientation, &sides);
    // One control for both ends, because the two ends are one thing: the
    // window onto the analyzer's axis. Dragged in MIDI note (which is what
    // makes it a log-frequency zoom) and read out in Hz.
    RangeBar::new(
        &mut cfg.low_midi,
        &mut cfg.high_midi,
        harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
            ..=harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI,
        "Frequency range",
    )
    .min_span(crate::PITCH_RANGE_MIN_SPAN)
    .display(hz_readout)
    .show(ui)
    .on_hover_text(
        "Visible frequency range, also used by the Spiral. \
                 Each octave has equal width. \
                 Drag the ends or the middle; double-click shows the full range.",
    );
    // No choice of what the markings say. They are the analyzer-standard
    // 1-2-5 frequency series, and were switchable to one at every C with
    // Bitwig octave numbers — which is what the note NAMES on the ribbons
    // already say, in the lattice's own spelling, at the pitch they are
    // sounding rather than at the nearest C below it.
    ValueBar::new(&mut cfg.marking_scale, crate::SCALE_BAR_RANGE, "Axis label scale")
        .unit(1.0, "×")
        .show(ui)
        .on_hover_text(
            "Size of frequency labels and the pointer readout. \
                 1× is the reference size; labels stay the same size when you zoom.",
        );

    // ---- Audio spectrum -------------------------------------------------
    // Always analyzed: the pane IS the analyzer, the spectrogram reads the
    // same buckets, and giving the whole depth axis to the roll is what the
    // divider is for.
    section(ui, "Audio analysis");
    if let Some(mut input) = params.analysis_input() {
        let before = input;
        choice_row(
            ui,
            "Audio input",
            &mut input,
            &[
                (
                    AnalysisInput::Main,
                    "Main",
                    "Analyze the plug-in's main input without changing its pass-through audio",
                ),
                (
                    AnalysisInput::Sidechain,
                    "Sidechain",
                    "Analyze the sidechain routed by the host. An unrouted sidechain is silence",
                ),
            ],
        );
        if input != before {
            params.set_analysis_input(input);
        }
    }
    button_row(ui, |ui| {
        ui.label("Resolution");
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
    // Beside the window rather than folded into it: the window trades time
    // against pitch, this trades cost and contrast against the estimate's own
    // noise, and no single row of buttons can name both.
    button_row(ui, |ui| {
        ui.label("Smoothing passes");
        for (tapers, label) in
            [(SpectrumTapers::One, "1"), (SpectrumTapers::Three, "3"), (SpectrumTapers::Five, "5")]
        {
            ui.selectable_value(&mut cfg.tapers, tapers, label).on_hover_text(match tapers {
                SpectrumTapers::One => "Sharpest frequency detail, with the most flicker and speckle. Lowest processing cost.",
                SpectrumTapers::Three => {
                    "Average three tapers of the same audio for less speckle, with softer frequency detail and higher processing cost."
                }
                SpectrumTapers::Five => {
                    "Average five tapers for the steadiest levels, with the softest frequency detail and highest processing cost."
                }
            });
        }
    });

    // Both ends of the height scale on one control, like the pitch range: the
    // window on the spectrum's dynamics rather than just where it bottoms out.
    RangeBar::new(
        &mut cfg.floor_db,
        &mut cfg.ceiling_db,
        crate::LEVEL_MIN_DB..=crate::LEVEL_MAX_DB,
        "Spectrum level range",
    )
    .min_span(crate::LEVEL_RANGE_MIN_SPAN)
    .display(db_readout)
    .show(ui)
    .on_hover_text(
        "Levels mapped to zero and full spectrum height, also used by lattice audio rings. \
                 Lower the upper end to enlarge quiet signals. \
                 Audio colors have their own Level color range on Colors.",
    );
    // Two bars and not one, because a spectrum's two directions are different
    // events: a partial arriving is worth seeing when it happens, and the same
    // partial's noise wobbling down is not worth drawing at all.
    ValueBar::new(&mut cfg.attack, 0.0..=BALLISTICS_MAX, "Spectrum attack")
        .unit(1000.0, " ms").decimals(0)
        .show(ui)
        .on_hover_text("Response time for the spectrum curve to rise when audio gets louder. 0 ms responds immediately.");
    ValueBar::new(&mut cfg.release, 0.0..=BALLISTICS_MAX, "Spectrum release")
        .unit(1000.0, " ms")
        .decimals(0)
        .show(ui)
        .on_hover_text(
            "Response time for the spectrum curve to fall when audio gets quieter. \
                 Increase for a steadier curve. \
                 0 ms responds immediately.",
        );
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
            "Reference slope in decibels per octave. \
                 0 shows raw power; -3 makes pink noise appear flat; more negative values lift high frequencies further. \
                 Affects every audio view.",
        );
        // Five signed numbers side by side, so `option_label` sets them in
        // monospace: a proportional face gives "0.0" and "-1.5" different
        // widths and leaves the row visibly uneven, where digits of one width
        // make it a scale.
        for step in crate::TILT_STEPS {
            ui.selectable_value(&mut cfg.tilt, step, option_label(&format!("{step:.1}")));
        }
    });

    ValueBar::new(&mut cfg.keyline, 0.0..=1.0, "Outline opacity")
        .percent().show(ui).on_hover_text(
        "Opacity of the outline along the spectrum curve. 0% hides the outline; 100% makes it fully visible.",
    );

    section(ui, "History");
    ui.checkbox(&mut cfg.show_roll, "Show MIDI notes").on_hover_text(
        "Show played MIDI notes as ribbons over the shared time axis. \
                 Their colors come from MIDI note colors on Colors.",
    );
    ui.checkbox(&mut cfg.show_spectrogram, "Show spectrogram").on_hover_text(
        "Show audio levels as a frequency-versus-time heatmap. \
                 Uses the shared History duration and the Audio level colors on Colors.",
    );
    ValueBar::new(&mut cfg.roll_seconds, crate::ROLL_SECONDS_MIN..=crate::ROLL_SECONDS_MAX, "History duration")
        .eased(true)
        .decimals(1)
        .unit(1.0, " s").display(span_readout)
        .show(ui)
        .on_hover_text(
            "Time shown by both MIDI ribbons and the spectrogram, up to 600 seconds. \
                 Drag the picture along its time axis to zoom, or double-click this bar to type seconds.",
        );
    button_row(ui, |ui| {
        if ui
            .button("Clear analyzer history")
            .on_hover_text(
                "Clear MIDI ribbons and spectrogram history. A held note reappears in the roll only when played again.",
            )
            .clicked()
        {
            state.tracker.clear_roll();
            state.spectrum.clear_history();
        }
    });
    section(ui, "MIDI ribbons");
    ui.add_enabled_ui(cfg.show_roll, |ui| {
        ValueBar::new(&mut cfg.roll_thickness, 0.2..=2.0, "Ribbon width")
            .unit(1.0, " st")
            .show(ui)
            .on_hover_text(
                "Ribbon width in semitones (st), measured on the frequency axis. \
                 1 st is the width of one semitone at any zoom.",
            );
        edge_bar(
            ui,
            (&mut cfg.roll_lead, &mut cfg.roll_lead_fade),
            crate::ROLL_LEAD_MAX,
            "Held-note extension",
            {
                let fresh = crate::SpectrumConfig::default();
                (fresh.roll_lead, fresh.roll_lead_fade)
            },
            |v| format!("{:.1}%", v * 100.0),
        )
        .on_hover_text(
            "Distance held notes extend into the spectrum, as a percentage of its depth. \
                 Solid to the inner handle, faded out by the outer. \
                 0% stops notes at the history boundary.",
        );
        ValueBar::new(
            &mut cfg.roll_lead_release,
            0.0..=crate::ROLL_LEAD_RELEASE_MAX,
            "Extension release",
        )
        .unit(1000.0, " ms")
        .decimals(0)
        .show(ui)
        .on_hover_text(
            "Time for the held-note extension to fade after release. 0 ms removes it immediately.",
        );
        ui.checkbox(&mut cfg.note_names, "Show note names").on_hover_text(
        "Label MIDI ribbons using the lattice tuning and spelling. Crowded labels wait for space.",
    );
        ui.add_enabled_ui(cfg.note_names && cfg.show_roll, |ui| {
            ui.checkbox(&mut cfg.note_names_travel, "Labels follow note onset").on_hover_text(
                "Place labels at the start of each note so they travel with its onset. \
                 Turn off to keep labels at the newest edge.",
            );
            ValueBar::new(&mut cfg.note_name_scale, crate::SCALE_BAR_RANGE, "Label scale")
                .unit(1.0, "×")
                .show(ui)
                .on_hover_text(
                    "Text size relative to each MIDI ribbon. \
                 1× is the reference size; labels also grow when you zoom in on frequency.",
                );
        });
    });
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::params::ParamKey;

    struct SourceBackend {
        source: Cell<AnalysisInput>,
        sets: RefCell<Vec<AnalysisInput>>,
    }

    impl Default for SourceBackend {
        fn default() -> Self {
            Self { source: Cell::new(AnalysisInput::Main), sets: RefCell::new(Vec::new()) }
        }
    }

    impl ParamBackend for SourceBackend {
        fn get(&self, key: ParamKey) -> f32 {
            key.default_value()
        }

        fn set(&self, _key: ParamKey, _value: f32) {}

        fn analysis_input(&self) -> Option<AnalysisInput> {
            Some(self.source.get())
        }

        fn set_analysis_input(&self, input: AnalysisInput) {
            self.source.set(input);
            self.sets.borrow_mut().push(input);
        }
    }

    struct NoSource;

    impl ParamBackend for NoSource {
        fn get(&self, key: ParamKey) -> f32 {
            key.default_value()
        }

        fn set(&self, _key: ParamKey, _value: f32) {}
    }

    fn frame(
        ctx: &egui::Context,
        state: &mut SharedState,
        backend: &dyn ParamBackend,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 1600.0));
        ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), events, ..Default::default() },
            |ui| spectrum_settings_pane(ui, state, backend),
        )
    }

    fn text_center(output: &egui::FullOutput, wanted: &str) -> Option<egui::Pos2> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Only the plugin has a host-routable auxiliary bus, so only its backend
    /// offers the row. The click goes through the real choice widget and back
    /// through the capability seam exactly once.
    #[test]
    fn analysis_input_row_is_plugin_only_and_selects_sidechain() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        let absent = frame(&ctx, &mut state, &NoSource, Vec::new());
        assert!(
            text_center(&absent, "Sidechain").is_none(),
            "a shell with no auxiliary input offered a Sidechain choice",
        );

        let backend = SourceBackend::default();
        let first = frame(&ctx, &mut state, &backend, Vec::new());
        let sidechain = text_center(&first, "Sidechain")
            .expect("a capable backend did not draw its Sidechain choice");
        frame(&ctx, &mut state, &backend, vec![egui::Event::PointerMoved(sidechain)]);
        frame(&ctx, &mut state, &backend, vec![press(sidechain, true)]);
        frame(&ctx, &mut state, &backend, vec![press(sidechain, false)]);

        assert_eq!(backend.source.get(), AnalysisInput::Sidechain);
        assert_eq!(backend.sets.borrow().as_slice(), &[AnalysisInput::Sidechain]);
    }
}
