//! The Spectral pane — the audio FFT curve, the sounding voices, and the
//! piano roll of what has been played, all over a shared MIDI-pitch axis —
//! and its settings pane.
//!
//! Everything is drawn in an abstract *(pitch, depth)* plane and mapped to
//! the screen by [`Axes`] at the last moment, so the whole pane turns
//! together when its orientation changes and no element has to know which
//! way is up. The roll's drawing lives next door in [`super::roll`].

use super::visibility_floor;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::{theme, SharedState};
use super::{nearest_visible_node, section, KEY_NAMES};
use lattice_core::notes::{display_octave_of, octave_start_midi};
use egui::Sense;
use lattice_scene::channel_color;

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state).
pub(super) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{
        RollColor, SpectralOrientation, SpectralPlacement, SpectrogramColor, SpectrumLabels,
        SpectrumWindow,
    };

    // ---- Layout ---------------------------------------------------------
    // Placement rebuilds the dock, so it needs the whole state; everything
    // below only touches the config.
    section(ui, "Layout");
    button_row(ui, |ui| {
        ui.label("Place").on_hover_text(
            "Where the Spectral pane sits. Changing this rebuilds the pane \
             arrangement, discarding any hand-docking.",
        );
        for (placement, label, hint) in [
            (
                SpectralPlacement::Below,
                "Below lattice",
                "A wide strip under the lattice: what sounds is directly \
                 under what lights up",
            ),
            (
                SpectralPlacement::Right,
                "Right of lattice",
                "A tall strip beside the lattice — with Auto orientation \
                 this turns the pane into an upright piano roll",
            ),
        ] {
            let selected = state.spectrum_config.placement == placement;
            if ui.selectable_label(selected, label).on_hover_text(hint).clicked() && !selected {
                state.spectrum_config.placement = placement;
                state.reset_dock_layout();
            }
        }
    });

    let cfg = &mut state.spectrum_config;
    choice_row(
        ui,
        "Orientation",
        &mut cfg.orientation,
        &[
            (
                SpectralOrientation::Auto,
                "Auto",
                "Follow the pane's shape: taller than wide reads upright",
            ),
            (SpectralOrientation::Horizontal, "Across", "Pitch left to right"),
            (SpectralOrientation::Vertical, "Upright", "Pitch bottom to top"),
        ],
    );
    button_row(ui, |ui| {
        ui.checkbox(&mut cfg.flip_pitch, "Flip pitch")
            .on_hover_text("Run the pitch axis the other way (high notes first)");
        ui.checkbox(&mut cfg.flip_depth, "Flip depth").on_hover_text(
            "Put the baseline on the opposite edge: the spectrum hangs \
             instead of standing, and the roll flows the other way",
        );
    });

    // ---- Audio spectrum -------------------------------------------------
    section(ui, "Spectrum");
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

    ui.checkbox(&mut cfg.fill, "Fill")
        .on_hover_text("Shade under the spectrum curve");
    ui.checkbox(&mut cfg.peak_hold, "Peak hold")
        .on_hover_text("Keep a decaying outline at each pitch's recent maximum");
    ui.checkbox(&mut cfg.show_voice_bars, "Voice bars")
        .on_hover_text("MIDI-derived bars at each voice's actual pitch");

    // ---- Pitch axis -----------------------------------------------------
    section(ui, "Pitch axis");
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

    // ---- Piano roll -----------------------------------------------------
    section(ui, "Piano roll");
    ui.checkbox(&mut cfg.show_roll, "Note history").on_hover_text(
        "Draw incoming MIDI as a scrolling roll over the same pitch axis. \
         Time runs away from the spectrum, so a note leaving the roll meets \
         the peak it is making.",
    );
    ValueBar::new(&mut cfg.roll_fraction, 0.0..=1.0, "Roll share")
        .show(ui)
        .on_hover_text("How much of the pane's depth the roll takes from the spectrum");
    ValueBar::new(&mut cfg.roll_seconds, 1.0..=120.0, "Span (s)")
        .eased(true)
        .decimals(1)
        .show(ui)
        .on_hover_text("Seconds of history the roll spans end to end");
    ValueBar::new(&mut cfg.roll_thickness, 0.2..=4.0, "Note width")
        .show(ui)
        .on_hover_text("Ribbon width in semitones of the pitch axis");
    ValueBar::new(&mut cfg.roll_rounding, 0.0..=1.0, "Rounding")
        .show(ui)
        .on_hover_text("Corner rounding of an unbent note (bent notes stay angular)");
    ValueBar::new(&mut cfg.roll_opacity, 0.05..=1.0, "Opacity").show(ui);
    ValueBar::new(&mut cfg.roll_age_fade, 0.0..=1.0, "Age fade")
        .show(ui)
        .on_hover_text("How far a note dims as it travels to the far edge");
    ValueBar::new(&mut cfg.roll_grid_seconds, 0.0..=10.0, "Grid (s)")
        .decimals(1)
        .show(ui)
        .on_hover_text("Seconds between time gridlines; 0 draws none");
    choice_row(
        ui,
        "Color",
        &mut cfg.roll_color,
        &[
            (RollColor::Channel, "Channel", "The lattice's own per-channel colors"),
            (RollColor::Pitch, "Pitch", "The low-to-high gradient, on every channel"),
            (RollColor::Accent, "Accent", "One flat color; the lattice leads"),
        ],
    );
    ui.checkbox(&mut cfg.roll_velocity_alpha, "Velocity opacity")
        .on_hover_text("Quiet notes draw fainter");
    ui.checkbox(&mut cfg.roll_highlight_held, "Highlight held")
        .on_hover_text("Notes still sounding keep full brightness");
    ui.checkbox(&mut cfg.roll_onsets, "Onset caps")
        .on_hover_text("A bright cap at each note's attack");
    ui.checkbox(&mut cfg.roll_outline, "Outline")
        .on_hover_text("Trace every note in a brightened edge");
    ui.checkbox(&mut cfg.roll_now_line, "Now line")
        .on_hover_text("Mark where the roll meets the spectrum");
    button_row(ui, |ui| {
        if ui
            .button("Clear roll")
            .on_hover_text("Forget the played-note timeline (held notes included)")
            .clicked()
        {
            state.tracker.clear_roll();
        }
    });

    // ---- Spectrogram ----------------------------------------------------
    section(ui, "Spectrogram");
    ui.checkbox(&mut cfg.show_spectrogram, "Spectrogram").on_hover_text(
        "A frequency-vs-time heatmap of the audio, drawn in the roll's \
         region on the same time axis — so each column of energy lines up \
         with the notes that made it. Shares the Spectrum's Floor and Tilt \
         for intensity. Turn Note history off to see the heatmap alone.",
    );
    choice_row(
        ui,
        "Palette",
        &mut cfg.spectrogram_color,
        &[
            (SpectrogramColor::Mono, "Mono", "Grayscale; the most neutral over the roll"),
            (SpectrogramColor::Heat, "Heat", "Black-red-orange-yellow-white"),
            (SpectrogramColor::Ice, "Ice", "Black-blue-cyan-white"),
            (SpectrogramColor::Aurora, "Aurora", "Violet-teal-green-yellow (even ramp)"),
            (SpectrogramColor::Pitch, "Pitch", "The lattice's own low-to-high pitch colors"),
        ],
    );
    ValueBar::new(&mut cfg.spectrogram_opacity, 0.05..=1.0, "Opacity")
        .show(ui)
        .on_hover_text("Overall heatmap opacity, so it can sit under the notes");
    button_row(ui, |ui| {
        if ui
            .button("Clear spectrogram")
            .on_hover_text("Forget the accumulated spectral history")
            .clicked()
        {
            state.spectrum.clear_history();
        }
    });
}

/// Depth budget for both plots, as a fraction of the *spectrum's share* of
/// the depth axis: a full-scale (0 dB) sine and a full-velocity held voice
/// both top out here. The two MUST agree, or the voice bars stop being
/// comparable with the spectrum curve they are drawn over.
const PLOT_HEIGHT_FRACTION: f32 = 0.85;

/// The 1 kHz pivot of the tilt slope, as a MIDI pitch.
const TILT_PIVOT_MIDI: f32 = 83.213_1;

/// How loud `power` reads at pitch `midi`, on a 0..1 scale: the configured
/// floor is 0, a full-scale (0 dB) sine is 1, and the tilt lifts treble by
/// its slope above the 1 kHz pivot. The spectrum curve's height and the
/// spectrogram's cell intensity both read from this, so the two always agree
/// on what "loud" means for a given bucket.
pub(super) fn loudness(cfg: &crate::SpectrumConfig, power: f32, midi: f32) -> f32 {
    let db = 10.0 * power.max(1e-12).log10() - cfg.tilt * (midi - TILT_PIVOT_MIDI) / 12.0;
    ((db - cfg.floor_db) / -cfg.floor_db).clamp(0.0, 1.0)
}

/// The pane's abstract drawing plane, and how it lands on screen.
///
/// Two axes, both running `0..1`:
///
/// - **pitch** runs along the pane; 0 is the low end of the octave zoom.
/// - **depth** runs across it; 0 is the *baseline* (where the spectrum
///   stands up from) and 1 the far edge (where the roll's oldest notes
///   are).
///
/// Orientation and the two flips are all handled inside [`at`](Self::at);
/// nothing else in the pane names a screen side.
#[derive(Clone, Copy)]
pub(super) struct Axes {
    pub rect: egui::Rect,
    /// Pitch runs up the pane rather than across it.
    vertical: bool,
    flip_pitch: bool,
    flip_depth: bool,
}

impl Axes {
    fn new(rect: egui::Rect, cfg: &crate::SpectrumConfig) -> Axes {
        Axes {
            rect,
            vertical: cfg.orientation.is_vertical(rect),
            flip_pitch: cfg.flip_pitch,
            flip_depth: cfg.flip_depth,
        }
    }

    /// The screen point at pitch fraction `p` and depth fraction `d`.
    pub fn at(&self, p: f32, d: f32) -> egui::Pos2 {
        let p = if self.flip_pitch { 1.0 - p } else { p };
        let d = if self.flip_depth { 1.0 - d } else { d };
        if self.vertical {
            // Pitch climbs the pane (low notes at the bottom, a keyboard
            // stood on end); depth runs out from the left edge.
            egui::pos2(
                self.rect.left() + self.rect.width() * d,
                self.rect.bottom() - self.rect.height() * p,
            )
        } else {
            egui::pos2(
                self.rect.left() + self.rect.width() * p,
                self.rect.bottom() - self.rect.height() * d,
            )
        }
    }

    /// Pixels spanned by the full pitch axis.
    pub fn pitch_len(&self) -> f32 {
        if self.vertical { self.rect.height() } else { self.rect.width() }
    }

    /// Pixels spanned by the full depth axis.
    pub fn depth_len(&self) -> f32 {
        if self.vertical { self.rect.width() } else { self.rect.height() }
    }

    /// Which way the pitch axis points on screen (unit vector).
    fn dir_pitch(&self) -> egui::Vec2 {
        (self.at(1.0, 0.0) - self.at(0.0, 0.0)).normalized()
    }

    /// Which way the depth axis points on screen (unit vector).
    fn dir_depth(&self) -> egui::Vec2 {
        (self.at(0.0, 1.0) - self.at(0.0, 0.0)).normalized()
    }

    /// A line clean across the depth axis at pitch `p` — the shape of
    /// every pitch gridline.
    pub fn across_depth(&self, p: f32) -> [egui::Pos2; 2] {
        [self.at(p, 0.0), self.at(p, 1.0)]
    }

    /// A line clean across the pitch axis at depth `d` — the shape of
    /// the roll's time gridlines and its "now" line.
    pub fn across_pitch(&self, d: f32) -> [egui::Pos2; 2] {
        [self.at(0.0, d), self.at(1.0, d)]
    }

    /// The rectangle spanning pitch `p0..p1` over the whole depth axis.
    pub fn band(&self, p0: f32, p1: f32) -> egui::Rect {
        egui::Rect::from_two_pos(self.at(p0, 0.0), self.at(p1, 1.0))
    }

    /// The pitch fraction under a screen position — the inverse of the
    /// pitch half of [`at`](Self::at). Unclamped.
    fn pitch_at(&self, pos: egui::Pos2) -> f32 {
        let t = if self.vertical {
            (self.rect.bottom() - pos.y) / self.rect.height().max(1.0)
        } else {
            (pos.x - self.rect.left()) / self.rect.width().max(1.0)
        };
        if self.flip_pitch { 1.0 - t } else { t }
    }

    /// Anchor and alignment for a text label at `(p, d)`, offset `along`
    /// pixels up the pitch axis and `into` pixels up the depth axis, and
    /// growing in those same directions. One helper covers all four
    /// orientations: the growth direction is read off the axes rather
    /// than case-matched per side.
    fn text_anchor(&self, p: f32, d: f32, along: f32, into: f32) -> (egui::Pos2, egui::Align2) {
        let (pu, du) = (self.dir_pitch(), self.dir_depth());
        let pos = self.at(p, d) + pu * along + du * into;
        let grow = pu * along.signum() + du * into.signum();
        let axis = |v: f32| {
            if v > 0.5 {
                egui::Align::Min
            } else if v < -0.5 {
                egui::Align::Max
            } else {
                egui::Align::Center
            }
        };
        (pos, egui::Align2([axis(grow.x), axis(grow.y)]))
    }
}

/// How the depth axis is shared out: the spectrum owns `0..split`, the
/// roll (and/or spectrogram) `split..1`. With that far region off the
/// spectrum owns all of it, which reproduces the pre-roll layout exactly —
/// that equality is what keeps the voice bars and the curve calibrated
/// against each other. The spectrogram shares the roll's region and time
/// axis, so it carves out the same share.
fn spectrum_share(cfg: &crate::SpectrumConfig) -> f32 {
    if cfg.show_roll || cfg.show_spectrogram {
        (1.0 - cfg.roll_fraction).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Where a pitch sits on the pane's axis: the octave zoom, as a mapping.
#[derive(Clone, Copy)]
pub(super) struct PitchScale {
    pub min_midi: f32,
    pub max_midi: f32,
    pub span: f32,
}

impl PitchScale {
    /// MIDI pitch to axis fraction. Outside the zoom this runs past
    /// `0..1`; callers clip or skip.
    pub fn t_of(&self, midi: f32) -> f32 {
        (midi - self.min_midi) / self.span
    }

    pub fn contains(&self, midi: f32) -> bool {
        (self.min_midi..=self.max_midi).contains(&midi)
    }
}

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
/// Hover sync goes both ways: the lattice-hovered pitch class shows as a
/// band here, and hovering a pitch here highlights the matching lattice
/// node (if one is in view).
pub(crate) fn spectral_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    use crate::SpectrumLabels;
    use lattice_core::spectrum::{hz_to_midi, midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let cfg = state.spectrum_config;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::well());

    let axes = Axes::new(rect, &cfg);
    // The axis: absolute pitch, linear in MIDI note = logarithmic in
    // frequency, so every octave gets equal room and every note draws at
    // its actual pitch. The displayed range is the Spectrum tab's octave
    // zoom; the Bitwig octave<->MIDI convention lives in lattice-core.
    let min_midi = octave_start_midi(cfg.low_octave) as f32;
    let max_midi = octave_start_midi(cfg.high_octave) as f32;
    let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };

    let split = spectrum_share(&cfg);
    // dB depth mapping: 0 dB (a full-scale sine) tops out at 85% of the
    // spectrum's share; the Spectrum tab's floor sets the bottom. Tilt is
    // the conventional reference slope (negative), so the display
    // SUBTRACTS it per octave above the 1 kHz pivot: -4.5 lifts treble
    // 4.5 dB/oct.
    let d_of = |power: f32, midi: f32| loudness(&cfg, power, midi) * split * PLOT_HEIGHT_FRACTION;

    // Axis gridlines: every C (note labels) or the analyzer-standard
    // 1-2-5 frequency series, per the Spectrum tab. Both run the full
    // depth, so they double as the roll's pitch lanes.
    let gridline = |p: f32, label: Option<String>| {
        painter.line_segment(axes.across_depth(p), egui::Stroke::new(1.0, theme::panel()));
        if let Some(label) = label {
            let (pos, align) = axes.text_anchor(p, 0.0, 3.0, 2.0);
            painter.text(pos, align, label, egui::FontId::monospace(10.0), theme::text_dim());
        }
    };
    match cfg.labels {
        SpectrumLabels::Notes => {
            let mut c = min_midi as i32;
            while c <= max_midi as i32 {
                let label = (c < max_midi as i32).then(|| format!("C{}", display_octave_of(c)));
                gridline(scale.t_of(c as f32), label);
                c += 12;
            }
        }
        SpectrumLabels::Frequency => {
            for hz in
                [20.0f32, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0]
            {
                let midi = hz_to_midi(hz);
                if !scale.contains(midi) {
                    continue;
                }
                let label =
                    if hz >= 1_000.0 { format!("{}k", hz / 1_000.0) } else { format!("{hz}") };
                gridline(scale.t_of(midi), Some(label));
            }
        }
    }

    // Cross-pane highlight: the pitch class hovered in ANY pane shows as
    // a tolerance-wide band in every octave.
    if let Some(pos) = state.hovered {
        let semis = state.tuning.pitch_class(pos).to_cents() / 100.0;
        // At least 1.5 px wide however tight the tolerance is.
        let half = (state.tuning.tolerance_cents() / 100.0 / scale.span)
            .max(1.5 / axes.pitch_len().max(1.0));
        let mut midi = min_midi + semis;
        while midi < max_midi {
            let t = scale.t_of(midi);
            painter.rect_filled(axes.band(t - half, t + half), 0.0, theme::accent_fill());
            midi += 12.0;
        }
    }

    // Advance the analyzer up front (it throttles the FFT internally) so the
    // spectrogram accumulates this frame's column even when the curve is
    // hidden. The curve below calls display() again and gets the same result
    // without re-running the FFT.
    if cfg.show_audio || cfg.show_spectrogram {
        let _ = state.spectrum.display(now, &cfg);
    }

    // The far share of the depth axis: a spectrogram heatmap of the audio
    // and/or the piano roll of what has been played, both on the same
    // `now`-anchored time axis. The spectrogram lays down first (it's a
    // background); the roll's ribbons sit over it, and the live spectrum
    // curve over everything. Turning the ribbons off (`show_roll`) with the
    // spectrogram on leaves the heatmap alone.
    if split < 1.0 {
        if cfg.show_spectrogram {
            super::spectrogram::draw_spectrogram(&painter, &axes, &scale, state, split, now);
        }
        if cfg.show_roll {
            super::roll::draw_roll(&painter, &axes, &scale, state, split, now);
        }
    }

    // Audio spectrum: the FFT of the shell's audio source, every partial
    // at its actual pitch. Fundamentals line up under their voice bars;
    // the harmonic series marches up the axis from each note.
    if cfg.show_audio && split > 0.0 {
        if let Some((levels, peaks)) = state.spectrum.display(now, &cfg) {
            // Only the buckets inside the octave zoom.
            let visible: Vec<(f32, f32, f32, f32)> = (0..levels.len())
                .filter_map(|i| {
                    let midi = SPECTRUM_MIN_MIDI + (i as f32 + 0.5) / BINS_PER_SEMITONE as f32;
                    scale.contains(midi).then(|| (midi, scale.t_of(midi), levels[i], peaks[i]))
                })
                .collect();

            if cfg.fill {
                // One translucent slab per bucket, wide enough to meet its
                // neighbors.
                let slab = (axes.pitch_len() / (scale.span * BINS_PER_SEMITONE as f32)) + 0.5;
                let fill_color = theme::accent().gamma_multiply(0.15);
                for &(midi, t, level, _) in &visible {
                    let d = d_of(level, midi);
                    if d * axes.depth_len() > 0.5 {
                        painter.line_segment(
                            [axes.at(t, 0.0), axes.at(t, d)],
                            egui::Stroke::new(slab, fill_color),
                        );
                    }
                }
            }
            if cfg.peak_hold {
                let peak_points: Vec<egui::Pos2> = visible
                    .iter()
                    .map(|&(midi, t, _, peak)| axes.at(t, d_of(peak, midi)))
                    .collect();
                painter.add(egui::Shape::line(
                    peak_points,
                    egui::Stroke::new(1.0, theme::accent().gamma_multiply(0.35)),
                ));
            }
            let points: Vec<egui::Pos2> = visible
                .iter()
                .map(|&(midi, t, level, _)| axes.at(t, d_of(level, midi)))
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
    // They hang from the roll's "now" edge back INTO the spectrum: the
    // audio spectrum's fundamental rises from the baseline at the same
    // pitch, so a baseline-up bar would sit exactly on the peak it should
    // be compared against. Hanging bars point at the peak instead of
    // hiding it — and they start where the roll's live notes end, so a
    // note's ribbon and its bar read as one continuous mark.
    if cfg.show_voice_bars && split > 0.0 {
        for voice in state.tracker.voices() {
            let activation = voice.activation(now, state.frame_params.fade_time);
            if activation <= 0.0 || !scale.contains(voice.pitch) {
                continue;
            }
            let t = scale.t_of(voice.pitch);
            let depth =
                split * PLOT_HEIGHT_FRACTION * activation * visibility_floor(voice.velocity);
            let c = channel_color(
                voice.channel,
                voice.pitch,
                state.frame_params.darkest_pitch,
                state.frame_params.brightest_pitch,
            );
            let color =
                egui::Color32::from_rgb((c.x * 255.0) as u8, (c.y * 255.0) as u8, (c.z * 255.0) as u8);
            painter.line_segment(
                [axes.at(t, split), axes.at(t, split - depth)],
                egui::Stroke::new(3.0, color),
            );
        }
    }

    // Hovering here highlights the matching lattice node (if in view) and
    // reads out the pitch under the cursor.
    if let Some(pointer) = response.hover_pos() {
        let midi = (min_midi + axes.pitch_at(pointer) * scale.span).clamp(min_midi, max_midi);
        // The axis starts on a C, so cents-from-C is just the fractional
        // octave position.
        let pc_cents = (midi - min_midi).rem_euclid(12.0) * 100.0;
        state.hovered = nearest_visible_node(
            &state.view,
            &state.tuning,
            lattice_core::PitchClass::from_cents(pc_cents),
        );
        let nearest = midi.round();
        let (pos, align) = axes.text_anchor(scale.t_of(midi), 1.0, 6.0, -2.0);
        painter.text(
            pos,
            align,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpectralOrientation, SpectrumConfig};
    use lattice_core::{NoteEvent, NoteEventKind};

    /// A 300x100 pane at an offset origin, so a mistake that assumes the
    /// rect starts at zero shows up.
    const WIDE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };
    const TALL: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(110.0, 320.0) };

    fn axes(rect: egui::Rect, orientation: SpectralOrientation, flips: (bool, bool)) -> Axes {
        let cfg = SpectrumConfig {
            orientation,
            flip_pitch: flips.0,
            flip_depth: flips.1,
            ..Default::default()
        };
        Axes::new(rect, &cfg)
    }

    /// Pitch runs left to right along the bottom; depth stands up from it.
    #[test]
    fn across_puts_the_baseline_on_the_bottom_edge() {
        let a = axes(WIDE, SpectralOrientation::Horizontal, (false, false));
        assert_eq!(a.at(0.0, 0.0), egui::pos2(10.0, 120.0), "low pitch, baseline");
        assert_eq!(a.at(1.0, 0.0), egui::pos2(310.0, 120.0), "high pitch, baseline");
        assert_eq!(a.at(0.0, 1.0), egui::pos2(10.0, 20.0), "low pitch, far edge");
        assert_eq!(a.pitch_len(), 300.0);
        assert_eq!(a.depth_len(), 100.0);
    }

    /// Pitch climbs the left edge (low notes at the bottom, like a
    /// keyboard stood on end); depth runs out to the right.
    #[test]
    fn upright_puts_the_baseline_on_the_left_edge() {
        let a = axes(TALL, SpectralOrientation::Vertical, (false, false));
        assert_eq!(a.at(0.0, 0.0), egui::pos2(10.0, 320.0), "low pitch, baseline");
        assert_eq!(a.at(1.0, 0.0), egui::pos2(10.0, 20.0), "high pitch, baseline");
        assert_eq!(a.at(0.0, 1.0), egui::pos2(110.0, 320.0), "low pitch, far edge");
        assert_eq!(a.pitch_len(), 300.0);
        assert_eq!(a.depth_len(), 100.0);
    }

    #[test]
    fn auto_orientation_follows_the_pane_shape() {
        let wide = axes(WIDE, SpectralOrientation::Auto, (false, false));
        let tall = axes(TALL, SpectralOrientation::Auto, (false, false));
        // Same corner, opposite meanings: on the wide pane the far end of
        // the pitch axis is to the right, on the tall one it is up.
        assert_eq!(wide.at(1.0, 0.0), egui::pos2(310.0, 120.0));
        assert_eq!(tall.at(1.0, 0.0), egui::pos2(10.0, 20.0));
    }

    #[test]
    fn the_flips_reverse_exactly_one_axis_each() {
        let plain = axes(WIDE, SpectralOrientation::Horizontal, (false, false));
        let pitch = axes(WIDE, SpectralOrientation::Horizontal, (true, false));
        let depth = axes(WIDE, SpectralOrientation::Horizontal, (false, true));
        assert_eq!(pitch.at(0.0, 0.0), plain.at(1.0, 0.0));
        assert_eq!(pitch.at(0.0, 1.0), plain.at(1.0, 1.0));
        assert_eq!(depth.at(0.0, 0.0), plain.at(0.0, 1.0));
        assert_eq!(depth.at(1.0, 0.0), plain.at(1.0, 1.0));
    }

    /// Hover has to name the pitch the pointer is actually over, in every
    /// orientation — the readout and the lattice highlight both hang off
    /// this one inverse.
    #[test]
    fn pitch_at_inverts_at_whichever_way_the_axes_run() {
        for rect in [WIDE, TALL] {
            for orientation in [
                SpectralOrientation::Auto,
                SpectralOrientation::Horizontal,
                SpectralOrientation::Vertical,
            ] {
                for flips in [(false, false), (true, false), (false, true), (true, true)] {
                    let a = axes(rect, orientation, flips);
                    for step in 0..=10 {
                        let p = step as f32 / 10.0;
                        // Any depth: the inverse reads the pitch axis only.
                        let back = a.pitch_at(a.at(p, 0.37));
                        assert!(
                            (back - p).abs() < 1e-4,
                            "{orientation:?} {flips:?}: {p} -> {back}"
                        );
                    }
                }
            }
        }
    }

    /// Pin the pre-rotation label placement: gridline labels used to be
    /// hard-coded to `(x + 3, bottom - 2)` / `LEFT_BOTTOM`, and the
    /// orientation-agnostic anchor has to still land there on the layout
    /// that code was written for.
    #[test]
    fn gridline_labels_land_where_they_always_did_on_a_wide_pane() {
        let a = axes(WIDE, SpectralOrientation::Horizontal, (false, false));
        let (pos, align) = a.text_anchor(0.5, 0.0, 3.0, 2.0);
        assert_eq!(pos, egui::pos2(163.0, 118.0));
        assert_eq!(align, egui::Align2::LEFT_BOTTOM);
    }

    /// Whichever way the pane is turned, a label anchored just inside an
    /// edge grows inward rather than off the pane.
    #[test]
    fn label_anchors_grow_into_the_pane() {
        for orientation in [SpectralOrientation::Horizontal, SpectralOrientation::Vertical] {
            for flips in [(false, false), (true, false), (false, true), (true, true)] {
                let a = axes(WIDE, orientation, flips);
                let (pos, align) = a.text_anchor(0.5, 0.0, 3.0, 2.0);
                // A nominal 40x12 label placed by this anchor.
                let box_ = align.anchor_size(pos, egui::vec2(40.0, 12.0));
                assert!(
                    WIDE.contains_rect(box_),
                    "{orientation:?} {flips:?}: {box_:?} escapes {WIDE:?}"
                );
            }
        }
    }

    /// With the roll off, the spectrum gets the whole depth axis — the
    /// layout the voice-bar/curve calibration was set up against.
    #[test]
    fn the_roll_only_takes_depth_when_it_is_shown() {
        let mut cfg = SpectrumConfig { roll_fraction: 0.4, ..Default::default() };
        cfg.show_roll = false;
        assert_eq!(spectrum_share(&cfg), 1.0);
        cfg.show_roll = true;
        assert_eq!(spectrum_share(&cfg), 0.6);
        cfg.roll_fraction = 1.0;
        assert_eq!(spectrum_share(&cfg), 0.0, "the roll may take the whole pane");
    }

    /// The whole pane, painted in every orientation with a roll that has
    /// held notes, bent notes, notes off the octave zoom and notes older
    /// than the window. Geometry this fiddly is easy to make degenerate
    /// (zero-area quads, NaN from a zero span), and egui panics on those.
    #[test]
    fn the_pane_paints_in_every_orientation() {
        for rect in [WIDE, TALL] {
            for orientation in [
                SpectralOrientation::Auto,
                SpectralOrientation::Horizontal,
                SpectralOrientation::Vertical,
            ] {
                for flips in [(false, false), (true, true)] {
                    for roll_fraction in [0.0, 0.55, 1.0] {
                        let shapes = paint(rect, orientation, flips, roll_fraction);
                        assert!(shapes > 0, "{orientation:?} {flips:?} drew nothing");
                    }
                }
            }
        }
    }

    /// Run one frame of the Spectral pane into `rect` and count the shapes
    /// it emitted.
    fn paint(
        rect: egui::Rect,
        orientation: SpectralOrientation,
        flips: (bool, bool),
        roll_fraction: f32,
    ) -> usize {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = orientation;
        state.spectrum_config.flip_pitch = flips.0;
        state.spectrum_config.flip_depth = flips.1;
        state.spectrum_config.roll_fraction = roll_fraction;
        state.spectrum_config.roll_outline = true;
        state.spectrum_config.roll_seconds = 10.0;
        // Exercise the spectrogram's mesh path in every orientation too, with
        // energy at both axis extremes (where cell clamping is most likely to
        // fold a quad to zero area — which egui panics on).
        state.spectrum_config.show_spectrogram = true;
        let mut spectrum_bins = [0.0f32; lattice_core::spectrum::SPECTRUM_BINS];
        spectrum_bins[0] = 1.0;
        spectrum_bins[lattice_core::spectrum::SPECTRUM_BINS / 2] = 0.5;
        spectrum_bins[lattice_core::spectrum::SPECTRUM_BINS - 1] = 0.3;
        for i in 0..80 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.125, spectrum_bins);
        }

        let on = |time, note| NoteEvent {
            time,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 0.7 },
        };
        let off = |time, note| NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off };
        // Long past the window; inside it; bent across it; off the top of
        // the octave zoom; and one still held at `now`.
        state.tracker.handle_event(on(0.0, 60));
        state.tracker.handle_event(off(1.0, 60));
        state.tracker.handle_event(on(95.0, 62));
        state.tracker.handle_event(off(96.0, 62));
        state.tracker.handle_event(on(96.0, 64));
        state.tracker.handle_event(NoteEvent {
            time: 97.0,
            channel: 0,
            note: 64,
            kind: NoteEventKind::Tuning { semitones: 7.5 },
        });
        state.tracker.handle_event(off(99.0, 64));
        state.tracker.handle_event(on(97.0, 127));
        state.tracker.handle_event(on(99.0, 67));
        let now = 100.0;
        state.tracker.prune(now, 1.0);
        // Something for the hover band to draw, too.
        state.hovered = Some(lattice_core::LatticePos { threes: 0, fives: 0, sevens: 0 });

        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                spectral_pane(&mut child, &mut state, now);
            },
        );
        output.shapes.len()
    }
}
