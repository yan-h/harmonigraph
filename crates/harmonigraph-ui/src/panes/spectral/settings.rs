//! The Analyzer settings page's own sections: analyzer layout and the shared
//! audio analysis, then spectrogram appearance. Color tables live on
//! [`super::super::color`].

use harmonigraph_scene::{
    CLOUD_DIRECTION_MAX, CLOUD_DIRECTION_MIN, CLOUD_SPEED_MAX, CLOUD_SPEED_MIN, CONTOURS_MAX,
    CONTOURS_MIN, CONTOUR_SOFTNESS_MAX, CONTOUR_SOFTNESS_MIN, PITCH_SOFTNESS_MAX,
    PITCH_SOFTNESS_MIN, SCALE_REFRACT_MAX, SCALE_REFRACT_MIN, TIME_SOFTNESS_MAX, TIME_SOFTNESS_MIN,
};

use crate::config::BALLISTICS_MAX;
use crate::panes::{block, edge_bar, section, switched_section};
use crate::params::{AnalysisInput, ParamBackend};
use crate::widgets::{button_row, choice_row, RangeBar, ValueBar};
use crate::PictureState;

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
pub(crate) fn span_readout(seconds: f32) -> String {
    format!("{:.1} s", seconds.max(0.0))
}

/// The spectrogram's look: how soft it is, whether its levels are terraced,
/// and the texture over it. First on the Analyzer page, being what gets
/// dialled per song. Its switch is in the heading, because off there is
/// nothing here that draws; the history it shares with the ribbons is under
/// View, the axis it is.
pub(crate) fn spectrogram_section(ui: &mut egui::Ui, cfg: &mut crate::SpectrumConfig) {
    let atmosphere = &mut cfg.atmosphere;
    switched_section(
        ui,
        "Spectrogram",
        (
            &mut cfg.show_spectrogram,
            "Show audio levels as a frequency-versus-time heatmap. \
             Uses the shared History duration under View and the Audio level colors on Colors.",
        ),
        |ui| {
            block(ui, "Softness");
            // No style selector. Plain, Blur and Lava were three presets over three
            // independent effects — the blur, the terraces and the cloud — and each of
            // those now has a dial whose zero is off. The measured picture is all three
            // at zero, and the renderer takes its plain path there, so nothing is paid
            // for an effect that is not drawn. A row whose effect is off is greyed
            // rather than hidden, like every other section of this page, so the page's
            // inventory does not move under a drag.
            ValueBar::new(
                &mut atmosphere.pitch_softness,
                PITCH_SOFTNESS_MIN..=PITCH_SOFTNESS_MAX,
                "Pitch softness",
            )
            .unit(1.0, "¢")
            .show(ui)
            .on_hover_text("Blur width along pitch, in cents; 100 cents is one semitone. 0 leaves pitch unblurred. Applies only to the spectrogram.");
            ValueBar::new(
                &mut atmosphere.time_softness,
                TIME_SOFTNESS_MIN..=TIME_SOFTNESS_MAX,
                "Time softness",
            )
            .unit(1.0, " ms")
            .show(ui)
            .on_hover_text("Blur width along time, in milliseconds. 0 leaves time unblurred. Applies only to the spectrogram.");
            let soft = atmosphere.pitch_softness > 0.0 || atmosphere.time_softness > 0.0;
            ui.add_enabled_ui(soft, |ui| {
                ValueBar::new(&mut atmosphere.spread, 0.0..=1.0, "Wide blur mix").percent().show(ui)
                    .on_hover_text("Blend the close blur with a blur five times wider. 0% uses the close blur only; 100% uses the wider field. Pitch and Time softness set their base widths.");
            });
            block(ui, "Level contours");
            ValueBar::new(&mut atmosphere.contour_strength, 0.0..=1.0, "Contour strength")
                .percent()
                .show(ui)
                .on_hover_text(
                    "How far the levels are gathered into smooth terraces. 0% leaves the measured \
                     levels alone and costs nothing. Applies after texture refraction, so the \
                     same controls set the stepping of the refracted picture.",
                );
            ui.add_enabled_ui(atmosphere.contour_strength > 0.0, |ui| {
                ValueBar::new(&mut atmosphere.contours, CONTOURS_MIN..=CONTOURS_MAX, "Contour levels")
                    .integer()
                    .show(ui)
                    .on_hover_text("Number of level bands between the low and high audio-color endpoints. More bands make finer steps.");
                ValueBar::new(
                    &mut atmosphere.contour_softness,
                    CONTOUR_SOFTNESS_MIN..=CONTOUR_SOFTNESS_MAX,
                    "Contour edge softness",
                )
                .percent()
                .show(ui)
                .on_hover_text("Blend across adjacent level bands. 0% makes sharp boundaries; higher values soften the transitions.");
            });
            block(ui, "Texture");
            ValueBar::new(&mut atmosphere.cloud_depth, 0.0..=1.0, "Texture mix")
                .percent()
                .show(ui)
                .on_hover_text(
                    "How strongly the texture replaces the original picture. 0% removes the \
                     texture; 100% uses only the texture. Mosaic and Watercolor displace levels, \
                     then Contours and the palette apply; Stars replaces the picture with a \
                     starfield colored from the palette. Reads whatever the softness above \
                     leaves: with none, the measured picture itself.",
                );
            ui.add_enabled_ui(atmosphere.cloud_depth > 0.0, |ui| {
                // Two constructions rather than two presets of one, so the dials below
                // the shared three are per style: nothing a wash carries means anything
                // to a refracting scale, and the page would otherwise be a list of controls
                // most of which do nothing.
                use harmonigraph_scene::CloudStyle;
                choice_row(
                    ui,
                    "Style",
                    &mut atmosphere.cloud_style,
                    &[
                        (
                            CloudStyle::Mosaic,
                            "Mosaic",
                            "A pile of soft domes refracting the sound through their faces, \
                             then colored by the shared Contour levels and palette controls",
                        ),
                        (
                            CloudStyle::Watercolor,
                            "Watercolor",
                            "A field of overlapping globs, each reading the sound near its own \
                             centre. Fine layer mix blends their levels before Contour levels and the palette",
                        ),
                        (
                            CloudStyle::Stars,
                            "Stars",
                            "Pinpoint stars at several depths, each taking the color and brightness \
                             of the sound under it as it drifts. Nearer stars are fewer, bigger, \
                             brighter and faster. Contour levels do not apply to it",
                        ),
                    ],
                );
                ValueBar::new(
                    &mut atmosphere.cloud_speed,
                    CLOUD_SPEED_MIN..=CLOUD_SPEED_MAX,
                    "Drift speed",
                )
                .unit(1.0, "\u{d7}")
                .show(ui)
                .on_hover_text(
                    "1\u{d7} carries the texture about a pane-height every four minutes, and the \
                         nearest stars a pane-height in about nine seconds. 0 holds it still.",
                );
                ValueBar::new(
                    &mut atmosphere.cloud_direction,
                    CLOUD_DIRECTION_MIN..=CLOUD_DIRECTION_MAX,
                    "Drift direction",
                )
                .integer()
                .unit(1.0, "°")
                .show(ui)
                .on_hover_text(
                    "Constant direction of texture travel: 0° right, 90° down, 180° left and 270° up.",
                );
                // Two constructions, so two sets of dials: nothing a wash carries means
                // anything to a refracting scale, and a page listing both would be mostly
                // controls that do nothing wherever it stands.
                if atmosphere.cloud_style == CloudStyle::Stars {
                    star_bars(ui, atmosphere);
                } else if atmosphere.cloud_style == CloudStyle::Watercolor {
                    wash_bars(ui, atmosphere);
                } else {
                    ValueBar::new(&mut atmosphere.scale_size, cloud_size_range(), "Cell size")
                        .eased(true)
                        .unit(1.0, "\u{d7}")
                        .show(ui)
                        .on_hover_text(
                            "Size of each mosaic cell relative to the pane. 1× is the reference size; larger values make broader cells. Refraction is a fraction of each cell's width, so larger cells also displace the picture farther.",
                        );
                    ValueBar::new(&mut atmosphere.scale_variety, 0.0..=1.0, "Size variation")
                        .percent()
                        .show(ui)
                        .on_hover_text(
                            "Variation in mosaic cell size. 0% makes an even grid; 100% mixes small and large cells, with the largest about four times the smallest. The cells continue to cover the whole picture.",
                        );
                    ValueBar::new(
                        &mut atmosphere.scale_refract,
                        SCALE_REFRACT_MIN..=SCALE_REFRACT_MAX,
                        "Refraction",
                    )
                    .unit(100.0, "%")
                    .show(ui)
                    .on_hover_text(
                        "Displacement of the spectrogram within each mosaic cell, as a percentage of cell width. Positive values bend bands outward; negative values pull toward the center. -100% gives each cell one level; 0% leaves the picture unchanged.",
                    );
                }
            });
        },
    );
}

/// Played MIDI notes as ribbons over the history, and their names. Their
/// switch is in the heading.
pub(crate) fn ribbons_section(ui: &mut egui::Ui, cfg: &mut crate::SpectrumConfig) {
    switched_section(
        ui,
        "MIDI ribbons",
        (
            &mut cfg.show_roll,
            "Show played MIDI notes as ribbons over the shared time axis. \
             Their colors come from MIDI note colors on Colors.",
        ),
        |ui| {
            ValueBar::new(
                &mut cfg.roll_thickness,
                crate::config::ROLL_THICKNESS_RANGE,
                "Ribbon width",
            )
            .unit(1.0, " st")
            .show(ui)
            .on_hover_text(
                "Ribbon width in semitones (st), measured on the frequency axis. \
                     1 st is the width of one semitone at any zoom.",
            );
            ValueBar::new(&mut cfg.roll_opacity, 0.0..=1.0, "Ribbon opacity")
                .percent()
                .show(ui)
                .on_hover_text(
                    "Opacity of MIDI ribbon colors over the spectrogram. \
                     Their dark surrounds keep their full strength.",
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
                "Time for a released extension to fade where it detached from the history boundary. 0 ms removes it immediately.",
            );
            crate::widgets::checkbox(ui, &mut cfg.note_names, "Show note names").on_hover_text(
                "Label MIDI ribbons using the lattice tuning and spelling. Crowded labels wait for space.",
            );
            ui.add_enabled_ui(cfg.note_names, |ui| {
                // The box moves a name to its ribbon's other END, which is the
                // onset only where time runs the screen's way (`Anchor::of`).
                let (label, hover) = if cfg.orientation.is_time_reversed() {
                    (
                        "Labels wait at the now-line",
                        "Place labels at the newest end of each note, so a held note's name waits at the now-line. \
                         Turn off to have labels travel with each note's onset.",
                    )
                } else {
                    (
                        "Labels follow note onset",
                        "Place labels at the start of each note so they travel with its onset. \
                         Turn off to keep labels at the newest edge.",
                    )
                };
                crate::widgets::checkbox(ui, &mut cfg.note_names_travel, label).on_hover_text(hover);
                ValueBar::new(&mut cfg.note_name_scale, crate::SCALE_BAR_RANGE, "Label scale")
                    .unit(1.0, "×")
                    .show(ui)
                    .on_hover_text(
                        "Text size relative to each MIDI ribbon. \
                         1× is the reference size; labels also grow when you zoom in on frequency.",
                    );
            });
            ValueBar::new(&mut cfg.atmosphere.note_glow, 0.0..=2.0, "Ribbon bloom")
                .unit(1.0, "×")
                .show(ui)
                .on_hover_text(
                    "Soft halos around MIDI ribbons in the spectrogram. \
                     0 turns bloom off; \
                     1× is the reference strength.",
                );
        },
    );
}

/// The analyzer's layout and its two axes: where it docks, which edge the
/// spectrum is on, how much pitch and how much time it shows. Then the
/// spectrum's own look.
pub(crate) fn view_section(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    dock: &mut crate::workspace::Position,
) {
    use crate::workspace::Position;
    use crate::SpectralOrientation;

    section(ui, "View", |ui| {
        // Beside the side the spectrum sits on, because the two together decide
        // the analyzer's shape. The dock is the editor window's arrangement only;
        // a video's layout is set on the Video tab.
        choice_row(
            ui,
            "Dock",
            dock,
            &[
                (Position::Right, "Right", "Analyzer to the right of the lattice"),
                (Position::Below, "Below", "Analyzer below the lattice"),
            ],
        );
        let cfg = &mut state.appearance.spectrum;
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
        choice_row(ui, "Spectrum edge", &mut cfg.orientation, &sides);
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
        ValueBar::new(
            &mut cfg.roll_seconds,
            crate::ROLL_SECONDS_MIN..=crate::ROLL_SECONDS_MAX,
            "History duration",
        )
        .eased(true)
        .decimals(1)
        .unit(1.0, " s")
        .display(span_readout)
        .show(ui)
        .on_hover_text(
            "Time shown by both MIDI ribbons and the spectrogram, up to 600 seconds. \
             Drag the picture along its time axis to zoom, or double-click this bar to type seconds.",
        );
        button_row(ui, |ui| {
            if ui
                .button("Clear history")
                .on_hover_text(
                    "Clear MIDI ribbons and spectrogram history. A held note reappears in the roll only when played again.",
                )
                .clicked()
            {
                state.runtime.tracker.clear_roll();
                state.runtime.spectrum.clear_history();
            }
        });
        block(ui, "Spectrum");
        ValueBar::new(&mut cfg.keyline_lift, 0.0..=1.0, "Spectrum outline intensity").percent().show(ui).on_hover_text(
            "How bright the line along the top of the spectrum is kept. The line takes the color of the spectrum under it, \
                 and dark colors are brightened toward white until they reach this level; brighter ones keep their own color. \
                 100% is a white line, 0% is exactly the color under it.",
        );
        ValueBar::new(&mut cfg.backdrop_strength, 0.0..=1.0, "Backdrop strength").percent().show(ui).on_hover_text(
            "How bright the grey backdrop above the spectrum is at the floor; it fades out upward and stops at the spectrum's edge. \
                 0% is no backdrop. Faint backdrops read as a flat wash; about 70% and up keep dark fills visible.",
        );
        // Below the strength, so hiding them does not move the bar just dragged.
        if cfg.backdrop_strength > 0.0 {
            ValueBar::new(&mut cfg.backdrop_height, crate::config::BACKDROP_HEIGHT_RANGE, "Backdrop height")
                .percent()
                .show(ui)
                .on_hover_text("How far up the level axis the backdrop reaches before it has faded out: 100% is the ceiling.");
            ValueBar::new(&mut cfg.backdrop_gap, crate::config::BACKDROP_GAP_RANGE, "Stripe spacing")
                .integer()
                .unit(1.0, " px")
                .display(|gap| if gap < 0.5 { "Gradient".into() } else { format!("{gap:.0} px") })
                .show(ui)
                .on_hover_text(
                    "Dark pixels between the backdrop's stripes. The dark gaps are what let even a fill as dark as the background \
                         show as a break in the stripes; wider spacing is calmer but can miss a partial one pixel wide. \
                         0 is a smooth gradient: softer, but a fill as bright as the glow at its own height blends into it.",
                );
        }
    });
}

/// Measurement controls shared by every audio view: the input and its
/// resolution, how levels map to height, and how fast the live picture moves.
/// Set once and left, in every saved project, so the section is one place to
/// find all of it rather than three.
pub(crate) fn analysis_section(
    ui: &mut egui::Ui,
    cfg: &mut crate::SpectrumConfig,
    params: &dyn ParamBackend,
) {
    use crate::{SpectrumTapers, SpectrumWindow};

    section(ui, "Analysis", |ui| {
        crate::widgets::weak(
            ui,
            "Shared by the Analyzer, Spiral, spectrogram and lattice audio rings. These settings do not change pass-through audio.",
        );
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
        let windows = [
            (SpectrumWindow::Fast, "Fast", "snappy response, coarse bass pitch"),
            (SpectrumWindow::Balanced, "Balanced", "the default tradeoff"),
            (SpectrumWindow::Precise, "Precise", "sharp bass pitch, slower response"),
        ];
        let hints: Vec<_> = windows
            .iter()
            .map(|(window, _, hint)| format!("{} samples: {hint}", window.samples()))
            .collect();
        let options: Vec<_> = windows
            .iter()
            .zip(&hints)
            .map(|(&(window, label, _), hint)| (window, label, hint.as_str()))
            .collect();
        choice_row(ui, "Frequency resolution", &mut cfg.window, &options);
        choice_row(ui, "Spectrum averaging", &mut cfg.tapers, &[
            (SpectrumTapers::One, "1", "Sharpest frequency detail, with the most flicker and speckle. Lowest processing cost."),
            (SpectrumTapers::Three, "3", "Average three tapers of the same audio for less speckle, with softer frequency detail and higher processing cost."),
            (SpectrumTapers::Five, "5", "Average five tapers for the steadiest levels, with the softest frequency detail and highest processing cost."),
        ]);
        block(ui, "Level mapping");
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
            "Levels mapped to zero and full height in the Analyzer and Spiral, also used by lattice audio rings. \
                     Lower the upper end to enlarge quiet signals. \
                     Audio colors have their own Level color range on Colors.",
        );
        let labels: Vec<_> = crate::TILT_STEPS.iter().map(|step| format!("{step:.1}")).collect();
        let options: Vec<_> = crate::TILT_STEPS.iter().zip(&labels).map(|(&step, label)|
            (step, label.as_str(), "Reference slope in dB/oct. 0 shows raw power; -3 makes pink noise appear flat; more negative values lift high frequencies. Affects every audio view.")
        ).collect();
        choice_row(ui, "Tilt (dB/oct)", &mut cfg.tilt, &options);
        block(ui, "Live response");
        // Two bars and not one, because a spectrum's two directions are different
        // events: a partial arriving is worth seeing when it happens, and the same
        // partial's noise wobbling down is not worth drawing at all.
        ValueBar::new(&mut cfg.attack, 0.0..=BALLISTICS_MAX, "Live attack")
            .unit(1000.0, " ms").decimals(0)
            .show(ui)
            .on_hover_text("Response time for levels to rise in the live Analyzer, Spiral and lattice audio rings. Spectrogram history keeps the unsmoothed measurements. 0 ms responds immediately.");
        ValueBar::new(&mut cfg.release, 0.0..=BALLISTICS_MAX, "Live release")
            .unit(1000.0, " ms")
            .decimals(0)
            .show(ui)
            .on_hover_text(
                "Response time for levels to fall in the live Analyzer, Spiral and lattice audio rings. Spectrogram history keeps the unsmoothed measurements. \
                     Increase for a steadier curve. \
                     0 ms responds immediately.",
            );
    });
}

/// The band both texture size bars run over, taken from the same constants the
/// load door clamps to rather than written out here.
///
/// Two bars for two constructions, but one range: they mean the same thing
/// about their own texture, and a size the bar can reach but the blob cannot
/// keep is the silent break the persistence rule is about.
fn cloud_size_range() -> std::ops::RangeInclusive<f32> {
    harmonigraph_scene::CLOUD_SIZE_MIN..=harmonigraph_scene::CLOUD_SIZE_MAX
}

/// The watercolour wash: a field of translucent globs, and no light anywhere in
/// it.
///
/// Its own function because the two textures share only the three dials above,
/// and because describing this look in words has failed repeatedly — the
/// prototype's three keepers (J1, J2, J5) are one construction at three settings
/// of these bars, so the settings are what shipped rather than a choice made
/// here.
fn wash_bars(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::SpectralAtmosphere) {
    ValueBar::new(&mut atmosphere.wash_size, cloud_size_range(), "Glob size")
        .eased(true)
        .unit(1.0, "\u{d7}")
        .show(ui)
        .on_hover_text(
            "Size of watercolor globs relative to the pane. 1× is about one twentieth of the pane's height; smaller values make finer grain and larger values make broader patches. The apparent pitch width depends on the frequency range.",
        );
    ValueBar::new(&mut atmosphere.wash_fuzz, 0.0..=1.0, "Edge feathering").percent().show(ui).on_hover_text(
        "Blend between neighboring watercolor patches. 0% makes hard-edged patches; 100% dissolves their edges. Does not change where each patch samples the audio.",
    );
    ValueBar::new(&mut atmosphere.wash_lobe, 0.0..=1.0, "Shape warp")
        .percent()
        .show(ui)
        .on_hover_text(
            "Distort round watercolor patches into lobes and streaks. 0% keeps them round; higher values stretch and bend their shapes.",
        );
    ValueBar::new(&mut atmosphere.wash_refract, 0.0..=1.0, "Refraction")
        .percent()
        .show(ui)
        .on_hover_text(
            "Pull the sampled audio toward each glob's center. 0% keeps the original picture; 100% gives each glob the level at its center.",
        );
    ValueBar::new(&mut atmosphere.wash_layers, 0.0..=1.0, "Fine layer mix")
        .percent()
        .show(ui)
        .on_hover_text(
            "Mix a second layer of smaller watercolor patches over the broad layer. 0% uses the broad layer alone; 100% gives the fine layer its full strength.",
        );
}

/// The starfield: pinpoints in depth drifting with parallax, and the one texture
/// that is light rather than a displaced reading of it.
///
/// Every quality that differed between the prototype's four motion variants is
/// a bar here rather than a choice made in the shader, as are the two levers on
/// how heavy the field reads (`Glow` and `Depth balance`), because Yan's pick was a
/// starting point "with sliders exposed". The fresh values are that pick, V3,
/// with round 8's YB3 for how a star is coloured and shaped.
fn star_bars(ui: &mut egui::Ui, atmosphere: &mut harmonigraph_scene::SpectralAtmosphere) {
    use harmonigraph_scene::{
        STAR_DEFOCUS_MAX, STAR_DENSITY_MAX, STAR_DENSITY_MIN, STAR_FRINGE_MAX, STAR_GLOW_MAX,
        STAR_LIFETIME_MAX, STAR_LIFETIME_MIN, STAR_SIZE_CURVE_MAX, STAR_SIZE_CURVE_MIN,
        STAR_SIZE_MAX, STAR_SIZE_MIN, STAR_SPEED_CURVE_MAX, STAR_SPEED_CURVE_MIN, STAR_WANDER_MAX,
    };
    ValueBar::new(&mut atmosphere.star_density, STAR_DENSITY_MIN..=STAR_DENSITY_MAX, "Star density")
        .unit(1.0, "\u{d7}")
        .show(ui)
        .on_hover_text(
            "How many stars at every depth. Higher values pack them closer; past about 3\u{d7} the faintest dust is finer than a pixel and merges into texture.",
        );
    // Dragged in octaves, so the small end has room on the track: the sizes run
    // over two orders of magnitude and a linear track would crush 0.5 to 4
    // into its first few percent.
    let (mut small, mut big) = (atmosphere.star_size_min.log2(), atmosphere.star_size_max.log2());
    let response =
        RangeBar::new(&mut small, &mut big, STAR_SIZE_MIN.log2()..=STAR_SIZE_MAX.log2(), "Star size")
            .display(|octaves| format!("{:.1} px", octaves.exp2()))
            .show(ui)
            .on_hover_text(
                "The smallest and biggest stars: the farthest dust at the low end, the nearest stars at the high end, with Size curve deciding how the depths between share it out. Bigger stars are also farther apart.",
            );
    // Only on a change: the round trip through octaves is not exact, and
    // writing it back every frame would move the stored sizes by an ulp at a
    // time — and every star with them, since the life clocks key on them.
    if response.changed() {
        atmosphere.star_size_min = small.exp2();
        atmosphere.star_size_max = big.exp2();
    }
    ValueBar::new(
        &mut atmosphere.star_size_curve,
        STAR_SIZE_CURVE_MIN..=STAR_SIZE_CURVE_MAX,
        "Size curve",
    )
    .curve(|curve, p| p.powf(curve))
    .show(ui)
    .on_hover_text(
        "How the star spacing grows from the farthest depth to the nearest. 1 grows it evenly; higher values keep most depths fine dust and save the big stars for the nearest. The line previews it.",
    );
    ValueBar::new(&mut atmosphere.star_randomness, 0.0..=1.0, "Randomness")
        .percent()
        .show(ui)
        .on_hover_text(
            "How much stars differ from each other in brightness and size. A star's brightness is a position on the palette: dim stars take the palette's lower colors, bright ones its higher colors. 0% colors every star from the sound behind it; 100% makes a few bright stars among many faint ones. The field's average brightness stays the same at every setting.",
        );
    ValueBar::new(&mut atmosphere.star_glow, 0.0..=STAR_GLOW_MAX, "Glow")
        .percent()
        .show(ui)
        .on_hover_text(
            "A wide, soft light of the sound behind the stars. 0% leaves the palette's darkest color between them; higher values fill the gaps between harmonics, and stars dimmer than the glow disappear into it.",
        );
    ValueBar::new(&mut atmosphere.star_balance, -1.0..=1.0, "Depth balance")
        .show(ui)
        .on_hover_text(
            "Which depths keep all their stars. Left keeps all the far dust and thins the near stars; right keeps all the near stars and thins the dust; the middle keeps every depth full, so how many stars a depth has follows Star size alone.",
        );
    ValueBar::new(&mut atmosphere.star_fringe, 0.0..=STAR_FRINGE_MAX, "Fringe")
        .percent()
        .show(ui)
        .on_hover_text(
            "A faint, wider fringe around every star in the star's own color. 0% draws bare soft points.",
        );
    ValueBar::new(&mut atmosphere.star_wander, 0.0..=STAR_WANDER_MAX, "Wander")
        .percent()
        .show(ui)
        .on_hover_text(
            "How far each star strays from the shared drift on its own slow path, as a share of the spacing between stars at its depth. 0% moves every depth as one sheet.",
        );
    ValueBar::new(&mut atmosphere.star_far_speed, 0.0..=1.0, "Far star speed")
        .percent()
        .show(ui)
        .on_hover_text(
            "How fast the farthest stars drift, as a share of the nearest stars' speed. Lower values deepen the parallax; 100% moves every depth together.",
        );
    ValueBar::new(
        &mut atmosphere.star_speed_curve,
        STAR_SPEED_CURVE_MIN..=STAR_SPEED_CURVE_MAX,
        "Speed curve",
    )
    .curve(|curve, p| p.powf(curve))
    .show(ui)
    .on_hover_text(
        "How the drift speed grows from the farthest depth to the nearest. 1 steps it evenly; higher values keep most depths slow and the nearest fast, lower ones the reverse. The line previews it.",
    );
    ValueBar::new(&mut atmosphere.star_speed_spread, 0.0..=1.0, "Speed spread")
        .percent()
        .show(ui)
        .on_hover_text(
            "How much each star's speed differs from the others at its depth. 0% moves each depth as one sheet; 100% fills the gaps between the depths' speeds, so the parallax is a continuum rather than steps.",
        );
    ValueBar::new(
        &mut atmosphere.star_lifetime,
        STAR_LIFETIME_MIN..=STAR_LIFETIME_MAX,
        "Star lifetime",
    )
    .eased(true)
    .unit(1.0, " s")
    .show(ui)
    .on_hover_text(
        "How long each star lives before a new one takes its place, fading in and out. With Speed spread, the smallest stars live shorter so they stay near their places, and twinkle faster.",
    );
    ValueBar::new(&mut atmosphere.star_defocus, 0.0..=STAR_DEFOCUS_MAX, "Near star softness")
        .percent()
        .show(ui)
        .on_hover_text("How much the nearest stars are softened, as if out of focus. 0% keeps every star sharp.");
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
        state: &mut PictureState,
        backend: &dyn ParamBackend,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 1600.0));
        ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), events, ..Default::default() },
            |ui| analysis_section(ui, &mut state.appearance.spectrum, backend),
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
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
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
