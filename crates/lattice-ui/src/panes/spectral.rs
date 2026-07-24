//! The Spectral pane — the audio FFT curve, the sounding voices, and the
//! piano roll of what has been played, all over a shared MIDI-pitch axis —
//! and its settings pane.
//!
//! Everything is drawn in an abstract *(pitch, depth)* plane and mapped to
//! the screen by [`Axes`] at the last moment, so the whole pane turns
//! together when its orientation changes and no element has to know which
//! way is up. The roll's drawing lives next door in [`super::roll`].

use super::visibility_floor;
use crate::widgets::{button_row, choice_row, RangeBar, ValueBar};
use crate::{theme, SharedState};
use super::{nearest_visible_node, scene_color, section, KEY_NAMES};
use lattice_core::notes::display_octave_of;
use egui::Sense;
use lattice_scene::channel_color;

/// The lowest C at or above `midi`, as a MIDI note. Where the octave
/// gridlines start: since the pitch range went continuous it can begin
/// anywhere, and stepping twelves from the range's own start would scatter
/// the "C" lines across whatever pitch the zoom happens to begin on.
fn first_c_at_or_above(midi: f32) -> i32 {
    (midi / 12.0).ceil() as i32 * 12
}

/// A MIDI note as the frequency an analyzer would label it: whole hertz down
/// low, kHz to one decimal above 1000, each carrying its unit so the number
/// says what it is. Three or four significant figures is all a range readout
/// can use — "16744 Hz" is noise where "16.7 kHz" is a number you can read at
/// a glance while dragging.
fn hz_readout(midi: f32) -> String {
    let hz = lattice_core::spectrum::midi_to_hz(midi);
    if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state).
pub(super) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{RollColor, SpectralOrientation, SpectrogramColor, SpectrumLabels, SpectrumWindow};

    // ---- Layout ---------------------------------------------------------
    // Just the orientation now; drag the pane wherever you like (egui_dock
    // docks it freely), and Auto follows the shape it lands in.
    section(ui, "Layout");
    let cfg = &mut state.spectrum_config;
    choice_row(
        ui,
        "Orientation",
        &mut cfg.orientation,
        &[
            (
                SpectralOrientation::Auto,
                "Auto",
                "Follow the pane's shape: the spectrogram scrolls along the long side",
            ),
            (
                SpectralOrientation::Horizontal,
                "Across",
                "Time scrolls sideways (now on the left); pitch is vertical, spectrum on the left",
            ),
            (
                SpectralOrientation::Vertical,
                "Upright",
                "Time scrolls downward (now on top); pitch is horizontal, spectrum on top",
            ),
        ],
    );

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

    ui.checkbox(&mut cfg.peak_hold, "Peak hold")
        .on_hover_text("Keep a decaying outline at each pitch's recent maximum");
    ui.checkbox(&mut cfg.show_voice_bars, "Voice bars")
        .on_hover_text("MIDI-derived bars at each voice's actual pitch");

    // ---- Pitch axis -----------------------------------------------------
    section(ui, "Pitch axis");
    // One control for both ends, because the two ends are one thing: the
    // window onto the analyzer's axis. Dragged in MIDI note (which is what
    // makes it a log-frequency zoom) and read out in Hz.
    RangeBar::new(
        &mut cfg.low_midi,
        &mut cfg.high_midi,
        lattice_core::spectrum::SPECTRUM_MIN_MIDI..=lattice_core::spectrum::SPECTRUM_MAX_MIDI,
    )
    .min_span(crate::PITCH_RANGE_MIN_SPAN)
    .display(hz_readout)
    .show(ui)
    .on_hover_text(
        "The slice of the spectrum on show. Drag either end to move it, drag \
         between them to slide the whole range (it squishes when it meets an \
         end), double-click for the full axis. The scale is logarithmic — \
         equal distances are equal musical intervals — so an octave is the \
         same width wherever it sits.",
    );
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
    ValueBar::new(&mut cfg.roll_outline_width, 0.5..=6.0, "Outline")
        .show(ui)
        .on_hover_text(
            "Stroke width of a note's outline. Notes are hollow, so the \
             spectrogram shows through them; lattice bloom adds a glow.",
        );
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
    ValueBar::new(&mut cfg.spectrogram_smoothing, 0.0..=0.9, "Smoothing")
        .show(ui)
        .on_hover_text(
            "Average each column with its neighbors in time: 0 is off, higher \
             smooths fast beating/chorus/reverb wobble, softening onsets a little",
        );
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
/// - **pitch** runs across the pane's SHORT side; 0 is the low end of the
///   pitch range.
/// - **depth** is the time axis, running along the LONG side; 0 is the
///   spectrum's outer edge, `split` the now-line (where the spectrum joins
///   the spectrogram), and 1 the far edge (the roll's oldest notes).
///
/// Orientation is handled inside [`at`](Self::at); nothing else in the pane
/// names a screen side. Two layouts: **Across** — time left(now)->right(past),
/// pitch bottom->top; and **Upright** — time top(now)->bottom(past), pitch
/// left->right. In both the spectrum sits at the now-line end.
#[derive(Clone, Copy)]
pub(super) struct Axes {
    pub rect: egui::Rect,
    /// Time (the depth axis) runs down the pane rather than along it, with
    /// pitch across. See [`SpectralOrientation`](crate::SpectralOrientation).
    time_vertical: bool,
}

impl Axes {
    fn new(rect: egui::Rect, cfg: &crate::SpectrumConfig) -> Axes {
        Axes { rect, time_vertical: cfg.orientation.is_time_vertical(rect) }
    }

    /// The screen point at pitch fraction `p` and depth (time) fraction `d`.
    pub fn at(&self, p: f32, d: f32) -> egui::Pos2 {
        if self.time_vertical {
            // Upright: time runs down (now/spectrum at the top, past below);
            // pitch runs across (low left, high right).
            egui::pos2(
                self.rect.left() + self.rect.width() * p,
                self.rect.top() + self.rect.height() * d,
            )
        } else {
            // Across: time runs along the pane (now/spectrum at the left, past
            // to the right); pitch climbs (low bottom, high top).
            egui::pos2(
                self.rect.left() + self.rect.width() * d,
                self.rect.bottom() - self.rect.height() * p,
            )
        }
    }

    /// Pixels spanned by the full pitch axis (the short side).
    pub fn pitch_len(&self) -> f32 {
        if self.time_vertical { self.rect.width() } else { self.rect.height() }
    }

    /// Pixels spanned by the full depth/time axis (the long side).
    pub fn depth_len(&self) -> f32 {
        if self.time_vertical { self.rect.height() } else { self.rect.width() }
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
        if self.time_vertical {
            (pos.x - self.rect.left()) / self.rect.width().max(1.0)
        } else {
            (self.rect.bottom() - pos.y) / self.rect.height().max(1.0)
        }
    }

    /// Anchor and alignment for a text label at `(p, d)`, offset `along`
    /// pixels up the pitch axis and `into` pixels up the depth axis, and
    /// growing in those same directions. One helper covers both
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

/// Where a pitch sits on the pane's axis: the pitch range, as a mapping.
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

/// Maps take time to a depth fraction on the shared time axis (the region the
/// roll and spectrogram split between them), and back. One mapping for two
/// modes, so the live and offline paths go through the same place instead of
/// each carrying a copy:
///
/// - **Live**: a `now`-anchored window. `now` sits on the near edge (`split`)
///   and the past scrolls out to the far edge (1), spanning `roll_seconds`.
/// - **Whole-song** (offline playhead): the entire take laid out statically —
///   the near edge is the render's start, the far edge its end — and only the
///   playhead ([`depth_of`](Self::depth_of) of `now`) moves.
#[derive(Clone, Copy)]
pub(super) struct TimeAxis {
    split: f32,
    depth_span: f32,
    /// Live: `roll_seconds`. Whole-song: the take span.
    window: f64,
    /// Take time at the near edge. Live: `now`. Whole-song: the render start.
    origin: f64,
    now: f64,
    whole_song: bool,
}

impl TimeAxis {
    pub(super) fn new(state: &SharedState, split: f32, now: f64) -> Self {
        let depth_span = 1.0 - split;
        match state.whole_song.as_ref() {
            Some(ws) => TimeAxis {
                split,
                depth_span,
                window: ws.span.max(0.05),
                origin: ws.start,
                now,
                whole_song: true,
            },
            None => TimeAxis {
                split,
                depth_span,
                window: state.spectrum_config.roll_seconds.max(0.05) as f64,
                origin: now,
                now,
                whole_song: false,
            },
        }
    }

    /// Fraction from the near edge (0) to the far edge (1) for take time `t`,
    /// unclamped.
    fn frac(&self, t: f64) -> f64 {
        if self.whole_song {
            (t - self.origin) / self.window
        } else {
            (self.origin - t) / self.window
        }
    }

    /// Depth for take time `t`, clamped into the region.
    pub(super) fn depth_of(&self, t: f64) -> f32 {
        self.split + self.frac(t).clamp(0.0, 1.0) as f32 * self.depth_span
    }

    /// Take time at a screen depth — the unclamped inverse of
    /// [`depth_of`](Self::depth_of).
    pub(super) fn time_at(&self, d: f32) -> f64 {
        let f = ((d - self.split) / self.depth_span) as f64;
        if self.whole_song {
            self.origin + f * self.window
        } else {
            self.origin - f * self.window
        }
    }

    /// The oldest take time the region shows — its far-edge cull point.
    pub(super) fn oldest(&self) -> f64 {
        if self.whole_song {
            self.origin
        } else {
            self.now - self.window
        }
    }

    /// Seconds spanned across the region.
    pub(super) fn window(&self) -> f64 {
        self.window
    }

    /// Whether this is the offline whole-song layout.
    pub(super) fn whole_song(&self) -> bool {
        self.whole_song
    }

    /// Depth of the playhead (the present moment).
    pub(super) fn playhead_depth(&self) -> f32 {
        self.depth_of(self.now)
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
pub(crate) fn spectral_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    now: f64,
    label_scale: f32,
    // Spectrogram texture slot: 0 the docked pane / offline render, 1 the
    // Render preview, so two live copies don't clobber one shared texture.
    surface: usize,
) {
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
    // its actual pitch. The displayed range is the Analyzer tab's pitch
    // range, which is free to start anywhere — it is not snapped to C.
    let min_midi = cfg.low_midi;
    // Never trust the pair to be ordered. A zero or negative span divides by
    // zero in PitchScale and paints NaN geometry, which egui panics on — and
    // a panic here takes the plugin's editor down inside the host. The range
    // bar can't produce one; a hand-edited state blob can.
    let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
    let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };

    // Offline playhead render: the whole take laid out statically with a
    // sweeping playhead. It takes the whole pane (split = 0), which also drops
    // the live curve and voice bars via their `split > 0` guards — leaving the
    // spectrogram, roll, and playhead.
    let whole_song = state.whole_song.is_some();
    let split = if whole_song { 0.0 } else { spectrum_share(&cfg) };
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
    // color, so covered and un-covered silence match at any opacity. Drawn under
    // the gridlines, so they still read as pitch lanes across the region.
    if cfg.show_spectrogram && cfg.spectrogram_opacity > 0.0 && split < 1.0 {
        let bed = egui::Rect::from_two_pos(axes.at(0.0, split), axes.at(1.0, 1.0));
        painter.rect_filled(bed, 0.0, egui::Color32::BLACK);
    }

    // Axis gridlines: every C (note labels) or the analyzer-standard
    // 1-2-5 frequency series, per the Analyzer tab. Both run the full
    // depth, so they double as the roll's pitch lanes. The lines lay down
    // here, under the spectrum; their text labels are collected and drawn
    // last (below the voice bars), so a loud spectrum slab never buries
    // which pitch a lane is.
    let gridline = |p: f32| {
        painter.line_segment(axes.across_depth(p), egui::Stroke::new(1.0, theme::panel()));
    };
    let mut axis_labels: Vec<(f32, String)> = Vec::new();
    match cfg.labels {
        SpectrumLabels::Notes => {
            let mut c = first_c_at_or_above(min_midi);
            while c <= max_midi as i32 {
                let t = scale.t_of(c as f32);
                gridline(t);
                if c < max_midi as i32 {
                    axis_labels.push((t, format!("C{}", display_octave_of(c))));
                }
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
                let t = scale.t_of(midi);
                gridline(t);
                let label =
                    if hz >= 1_000.0 { format!("{}k", hz / 1_000.0) } else { format!("{hz}") };
                axis_labels.push((t, label));
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
            super::spectrogram::draw_spectrogram(&painter, &axes, &scale, state, split, now, surface);
        }
        if cfg.show_roll {
            super::roll::draw_roll(&painter, &axes, &scale, state, split, now);
        }
    }

    // The playhead: in whole-song mode, the one moving mark sweeping across the
    // static spectrogram and roll (it replaces the roll's fixed now-line).
    if whole_song {
        let time = TimeAxis::new(state, split, now);
        painter.line_segment(
            axes.across_pitch(time.playhead_depth()),
            egui::Stroke::new(1.5, theme::accent()),
        );
    }

    // Audio spectrum: the FFT of the shell's audio source, every partial
    // at its actual pitch. Fundamentals line up under their voice bars;
    // the harmonic series marches up the axis from each note.
    if cfg.show_audio && split > 0.0 {
        let frame = state.frame_params;
        if let Some((levels, peaks)) = state.spectrum.display(now, &cfg) {
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
            let visible: Vec<(f32, f32, f32, f32)> = (0..cols)
                .map(|c| {
                    let edge = |i: usize| scale.min_midi + scale.span * i as f32 / cols as f32;
                    let (b0, b1) = (bucket_at(edge(c)), bucket_at(edge(c + 1)));
                    let (mut level, mut peak) = (0.0f32, 0.0f32);
                    for b in b0..=b1.max(b0) {
                        level = level.max(levels[b]);
                        peak = peak.max(peaks[b]);
                    }
                    let t = (c as f32 + 0.5) / cols as f32;
                    (scale.min_midi + t * scale.span, t, level, peak)
                })
                .collect();

            // Color from the SAME palette as the spectrogram, keyed by the same
            // loudness, so the curve reads in the heatmap's scheme rather than a
            // flat accent. `tint` keeps the palette's hue/brightness and only
            // sets opacity (gamma_multiply would darken it toward black).
            let hue = |power: f32, midi: f32| {
                super::spectrogram::cell_color(
                    cfg.spectrogram_color,
                    loudness(&cfg, power, midi),
                    midi,
                    &frame,
                )
            };
            let tint = |c: egui::Color32, a: u8| {
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
            };

            // The spectrum is a filled shape, like the spectrogram — no outline
            // curve. Each slab is one bucket in its own palette color, opaque
            // enough to read as a solid fill; densely packed, their tops make
            // the shape's edge (no separate line to fray).
            let slab = axes.pitch_len() / cols as f32 + 0.5;
            for &(midi, t, level, _) in &visible {
                let d = d_of(level, midi);
                if d * axes.depth_len() > 0.5 {
                    painter.line_segment(
                        [axes.at(t, sd(0.0)), axes.at(t, sd(d))],
                        egui::Stroke::new(slab, tint(hue(level, midi), 210)),
                    );
                }
            }
            if cfg.peak_hold {
                // The one remaining line: a decaying trace of recent maxima,
                // in the palette's loud color.
                let loud = super::spectrogram::cell_color(
                    cfg.spectrogram_color,
                    1.0,
                    (min_midi + max_midi) * 0.5,
                    &frame,
                );
                let pts: Vec<egui::Pos2> =
                    visible.iter().map(|&(m, t, _, pk)| axes.at(t, sd(d_of(pk, m)))).collect();
                painter.add(egui::Shape::line(pts, egui::Stroke::new(1.0, tint(loud, 150))));
            }
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
        // Stable order: held voices iterate a HashMap, and overlapping opaque
        // bars paint order-dependently — the offline render must match between
        // runs.
        let mut voices: Vec<&lattice_core::Voice> = state.tracker.voices().collect();
        voices.sort_unstable_by(|a, b| {
            a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
        });
        for voice in voices {
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
            let color = scene_color(c, 1.0);
            let ends = [axes.at(t, split), axes.at(t, split - depth)];
            // A note sounding off the visible lattice lights up no node, so
            // pulse an accent halo behind its bar: the Spectral pane is where
            // you'd otherwise miss that a pitch you can't see is playing. Same
            // match the lattice uses, so "off-lattice" agrees with the view.
            if nearest_visible_node(&state.view, &state.tuning, voice.pitch_class).is_none() {
                let pulse = 0.5 + 0.5 * (now * std::f64::consts::TAU * 1.6).sin() as f32;
                let halo = theme::accent().gamma_multiply(0.35 + 0.5 * pulse);
                painter.line_segment(ends, egui::Stroke::new(3.0 + 4.0 * pulse, halo));
            }
            painter.line_segment(ends, egui::Stroke::new(3.0, color));
        }
    }

    // The now-line, where the roll hands over to the spectrum — drawn here,
    // after both things it divides, rather than at the end of the roll where
    // it used to be. It marks the boundary between two pictures, so it has to
    // sit ON them: from inside the roll it went down before the spectrum curve
    // and the curve's fill painted over it, and the spectrogram's quad reaches
    // the same line from the other side. Either one eating into it left a
    // divider that flickered with the music instead of holding still.
    //
    // Whole-song mode sweeps a playhead across a static layout instead, and
    // draws its own mark above.
    if cfg.roll_now_line && !whole_song && split < 1.0 && split > 0.0 {
        painter.line_segment(axes.across_pitch(split), egui::Stroke::new(1.0, theme::hairline()));
    }

    // Axis labels last, riding on top of the spectrogram, spectrum, and
    // voice bars: a label only earns its place if you can read which pitch a
    // lane is, and a loud slab would otherwise bury it. The gridlines
    // themselves stay underneath (drawn above) as pitch lanes.
    // Haloed exactly like the lattice's node labels, and for the same reason:
    // whatever is behind them is a picture, not a background. A pitch label
    // over a bright spectrogram slab, or over the spectrum's own fill, has no
    // contrast to rely on at all.
    for (p, label) in axis_labels {
        let (pos, align) = axes.text_anchor(p, label_d, 3.0, label_into);
        super::lattice::outlined_text(
            &painter,
            pos,
            align,
            label,
            egui::FontId::monospace(10.0 * label_scale),
            theme::text_dim(),
            theme::well(),
        );
    }

    // Hovering here highlights the matching lattice node (if in view) and
    // reads out the pitch under the cursor.
    if let Some(pointer) = response.hover_pos() {
        let midi = (min_midi + axes.pitch_at(pointer) * scale.span).clamp(min_midi, max_midi);
        // Cents from C, measured from MIDI 0 (which IS a C) rather than from
        // the range's own start: the range used to be a pair of octave numbers
        // and so always began on a C, but it is continuous now and generally
        // does not. Measuring from it offset every hovered pitch class by
        // wherever the zoom happened to start, so hovering the spectrum lit up
        // the wrong lattice node.
        let pc_cents = midi.rem_euclid(12.0) * 100.0;
        state.hovered = nearest_visible_node(
            &state.view,
            &state.tuning,
            lattice_core::PitchClass::from_cents(pc_cents),
        );
        let nearest = midi.round();
        let (pos, align) = axes.text_anchor(scale.t_of(midi), 1.0, 6.0, -2.0);
        super::lattice::outlined_text(
            &painter,
            pos,
            align,
            format!(
                "{}{} {:+.0}\u{a2} \u{b7} {:.1} Hz",
                KEY_NAMES[nearest as usize % 12],
                display_octave_of(nearest as i32),
                (midi - nearest) * 100.0,
                midi_to_hz(midi),
            ),
            egui::FontId::monospace(10.5 * label_scale),
            theme::text(),
            theme::well(),
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

    fn axes(rect: egui::Rect, orientation: SpectralOrientation) -> Axes {
        let cfg = SpectrumConfig { orientation, ..Default::default() };
        Axes::new(rect, &cfg)
    }

    /// Across: time runs along the pane (now/spectrum on the left, past to
    /// the right); pitch climbs bottom to top.
    #[test]
    fn across_runs_time_sideways_with_pitch_climbing() {
        let a = axes(WIDE, SpectralOrientation::Horizontal);
        assert_eq!(a.at(0.0, 0.0), egui::pos2(10.0, 120.0), "low pitch, now edge (left)");
        assert_eq!(a.at(0.0, 1.0), egui::pos2(310.0, 120.0), "low pitch, past edge (right)");
        assert_eq!(a.at(1.0, 0.0), egui::pos2(10.0, 20.0), "high pitch, now edge");
        assert_eq!(a.pitch_len(), 100.0, "pitch is the short (vertical) side");
        assert_eq!(a.depth_len(), 300.0, "time is the long (horizontal) side");
    }

    /// Upright: time runs down the pane (now/spectrum on top, past below);
    /// pitch runs left to right.
    #[test]
    fn upright_runs_time_downward_with_pitch_across() {
        let a = axes(TALL, SpectralOrientation::Vertical);
        assert_eq!(a.at(0.0, 0.0), egui::pos2(10.0, 20.0), "low pitch, now edge (top)");
        assert_eq!(a.at(1.0, 0.0), egui::pos2(110.0, 20.0), "high pitch, top");
        assert_eq!(a.at(0.0, 1.0), egui::pos2(10.0, 320.0), "low pitch, past edge (bottom)");
        assert_eq!(a.pitch_len(), 100.0, "pitch is the short (horizontal) side");
        assert_eq!(a.depth_len(), 300.0, "time is the long (vertical) side");
    }

    #[test]
    fn auto_orientation_follows_the_pane_shape() {
        let wide = axes(WIDE, SpectralOrientation::Auto); // time along the width
        let tall = axes(TALL, SpectralOrientation::Auto); // time down the height
        // High pitch at the now edge: on the wide pane that's up top, on the
        // tall one it's to the right.
        assert_eq!(wide.at(1.0, 0.0), egui::pos2(10.0, 20.0));
        assert_eq!(tall.at(1.0, 0.0), egui::pos2(110.0, 20.0));
    }

    /// Hover has to name the pitch the pointer is actually over, in either
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
                let a = axes(rect, orientation);
                for step in 0..=10 {
                    let p = step as f32 / 10.0;
                    // Any depth: the inverse reads the pitch axis only.
                    let back = a.pitch_at(a.at(p, 0.37));
                    assert!((back - p).abs() < 1e-4, "{orientation:?}: {p} -> {back}");
                }
            }
        }
    }

    /// A gridline label at the now edge of a wide (Across) pane sits just
    /// inside it and grows up-and-inward (LEFT_BOTTOM anchor).
    #[test]
    fn gridline_labels_sit_just_inside_the_now_edge() {
        let a = axes(WIDE, SpectralOrientation::Horizontal);
        let (pos, align) = a.text_anchor(0.5, 0.0, 3.0, 2.0);
        assert_eq!(pos, egui::pos2(12.0, 67.0));
        assert_eq!(align, egui::Align2::LEFT_BOTTOM);
    }

    /// Whichever way the pane is turned, a label anchored just inside an
    /// edge grows inward rather than off the pane.
    #[test]
    fn label_anchors_grow_into_the_pane() {
        for orientation in [SpectralOrientation::Horizontal, SpectralOrientation::Vertical] {
            let a = axes(WIDE, orientation);
            let (pos, align) = a.text_anchor(0.5, 0.0, 3.0, 2.0);
            // A nominal 40x12 label placed by this anchor.
            let box_ = align.anchor_size(pos, egui::vec2(40.0, 12.0));
            assert!(WIDE.contains_rect(box_), "{orientation:?}: {box_:?} escapes {WIDE:?}");
        }
    }

    /// With the roll off, the spectrum gets the whole depth axis — the
    /// layout the voice-bar/curve calibration was set up against.
    #[test]
    fn the_roll_only_takes_depth_when_it_is_shown() {
        // Isolate the roll's depth share. The spectrogram claims depth the
        // same way and is on by default, so turn it off to test the roll alone.
        let mut cfg = SpectrumConfig { roll_fraction: 0.4, ..Default::default() };
        cfg.show_spectrogram = false;
        cfg.show_roll = false;
        assert_eq!(spectrum_share(&cfg), 1.0);
        cfg.show_roll = true;
        assert_eq!(spectrum_share(&cfg), 0.6);
        cfg.roll_fraction = 1.0;
        assert_eq!(spectrum_share(&cfg), 0.0, "the roll may take the whole pane");
    }

    /// The whole pane, painted in every orientation with a roll that has
    /// held notes, bent notes, notes off the pitch range and notes older
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
                for roll_fraction in [0.0, 0.55, 1.0] {
                    let shapes = paint(rect, orientation, roll_fraction);
                    assert!(!shapes.is_empty(), "{orientation:?} drew nothing");
                }
            }
        }
    }

    /// The axis labels are haloed like the lattice's node names. What sits
    /// behind them is a picture — a bright spectrogram slab, the spectrum's
    /// own fill — so plain text has no contrast to rely on, and a label you
    /// can't read doesn't say which pitch a lane is.
    #[test]
    fn the_axis_labels_are_haloed() {
        let shapes = paint(WIDE, SpectralOrientation::Horizontal, 0.55);
        let mut runs: std::collections::HashMap<String, usize> = Default::default();
        for shape in &shapes {
            if let egui::Shape::Text(t) = shape {
                *runs.entry(t.galley.text().to_owned()).or_default() += 1;
            }
        }
        assert!(!runs.is_empty(), "the pane drew no labels at all");
        for (label, stamps) in runs {
            assert!(
                stamps > 1,
                "{label:?} was drawn once, so it carries no halo — bare text over the \
                 spectrogram is what this guards against",
            );
        }
    }

    /// The readout names its own unit, and switches to kHz where an analyzer
    /// axis does.
    #[test]
    fn the_hz_readout_carries_its_unit() {
        assert_eq!(hz_readout(69.0), "440 Hz", "A440, the one value worth checking by hand");
        assert_eq!(hz_readout(lattice_core::spectrum::SPECTRUM_MIN_MIDI), "20 Hz");
        assert_eq!(hz_readout(lattice_core::spectrum::SPECTRUM_MAX_MIDI), "20.0 kHz");
        // The switch is at 1000 Hz exactly, not somewhere near it.
        let khz = lattice_core::spectrum::hz_to_midi(1000.0);
        assert_eq!(hz_readout(khz), "1.0 kHz");
        assert!(hz_readout(khz - 0.1).ends_with(" Hz"));
    }

    /// A C gridline has to land on a C. The range used to be a pair of octave
    /// numbers, so its start WAS a C and stepping twelves from it worked; now
    /// it can start anywhere.
    #[test]
    fn c_gridlines_land_on_cs_wherever_the_range_starts() {
        assert_eq!(first_c_at_or_above(48.0), 48, "a range already on a C keeps it");
        assert_eq!(first_c_at_or_above(40.5), 48);
        assert_eq!(first_c_at_or_above(47.99), 48);
        // The axis floor is 20 Hz, which is not a C — the first C above it is MIDI 24.
        assert_eq!(first_c_at_or_above(lattice_core::spectrum::SPECTRUM_MIN_MIDI), 24);
    }

    /// The settings pane, whose pitch-range bar derives rects from a PAIR of
    /// values — the shape of thing that folds to zero area and panics egui.
    /// Painted at both the widest and the narrowest range it allows.
    #[test]
    fn the_settings_pane_paints_at_either_extreme_of_the_pitch_range() {
        let axis =
            (lattice_core::spectrum::SPECTRUM_MIN_MIDI, lattice_core::spectrum::SPECTRUM_MAX_MIDI);
        for (low, high) in [axis, (40.5, 40.5 + crate::PITCH_RANGE_MIN_SPAN), (axis.0, axis.0)] {
            let mut state =
                SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.low_midi = low;
            state.spectrum_config.high_midi = high;
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 700.0));
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| spectrum_settings_pane(ui, &mut state),
            );
            assert!(!output.shapes.is_empty(), "{low}..{high} drew nothing");
        }
    }

    /// A state blob carrying a collapsed or inverted pitch range must not
    /// take the editor down with it.
    #[test]
    fn a_degenerate_pitch_range_still_paints() {
        for (low, high) in [(60.0, 60.0), (90.0, 30.0)] {
            let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.low_midi = low;
            state.spectrum_config.high_midi = high;
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                    spectral_pane(&mut child, &mut state, 100.0, 1.0, 0);
                },
            );
            assert!(!output.shapes.is_empty(), "{low}..{high} drew nothing");
        }
    }

    /// Run one frame of the Spectral pane into `rect` and count the shapes
    /// it emitted.
    fn paint(
        rect: egui::Rect,
        orientation: SpectralOrientation,
        roll_fraction: f32,
    ) -> Vec<egui::Shape> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = orientation;
        state.spectrum_config.roll_fraction = roll_fraction;
        state.spectrum_config.roll_outline_width = 2.0;
        state.spectrum_config.roll_seconds = 10.0;
        state.view.bloom_strength = 1.2; // exercise the note-glow passes
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
        // the pitch range; and one still held at `now`.
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
                spectral_pane(&mut child, &mut state, now, 1.0, 0);
            },
        );
        output.shapes.into_iter().map(|s| s.shape).collect()
    }
}
