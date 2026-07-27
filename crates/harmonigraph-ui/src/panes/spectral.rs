//! The Spectral pane — the audio FFT curve, the sounding voices, and the
//! piano roll of what has been played, all over a shared MIDI-pitch axis —
//! and its settings pane.
//!
//! Everything is drawn in an abstract *(pitch, depth)* plane and mapped to
//! the screen by [`Axes`] at the last moment, so the whole pane turns
//! together when its orientation changes and no element has to know which
//! way is up. The roll's drawing lives next door in [`super::roll`].

use crate::widgets::{button_row, choice_row, RangeBar, ValueBar};
use crate::{theme, SharedState};
use super::{names, nearest_visible_node, node_pointed_at, section};
use egui::Sense;

/// A MIDI note as the frequency an analyzer would label it: whole hertz down
/// low, kHz to one decimal above 1000, each carrying its unit so the number
/// says what it is. Three or four significant figures is all a range readout
/// can use — "16744 Hz" is noise where "16.7 kHz" is a number you can read at
/// a glance while dragging.
fn hz_readout(midi: f32) -> String {
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

/// A duration as the roll's Span bar reads it out, carrying its own unit
/// rather than naming one in the label: the bar runs from a second to ten
/// minutes, and "300" under a fixed "(s)" is a number you have to divide
/// before it means anything. "5m 00s" is the same value already read.
///
/// Tenths under a minute, whole seconds above it. The scale is eased, so the
/// short spans get most of the travel and are where a tenth is a visible
/// step; a tenth of a second inside five minutes is not.
fn span_readout(seconds: f32) -> String {
    // Rounded to tenths BEFORE the branch, so a span that displays as a
    // whole minute is written as one — 59.97s reads "1m 00s", never "60.0s".
    let tenths = (seconds.max(0.0) * 10.0).round() as i64;
    if tenths < 600 {
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else {
        let whole = (tenths + 5) / 10;
        format!("{}m {:02}s", whole / 60, whole % 60)
    }
}

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state).
pub(super) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{SpectralOrientation, SpectrogramColor, SpectrumWindow};

    // ---- Axes -----------------------------------------------------------
    // Which way the plot runs, and how much of the pitch axis it shows. Time's
    // own extent is the roll's Span, and lives with the roll — it is the one
    // axis setting that means nothing without the layer it measures.
    //
    // A plain heading rather than `section`: this is the top of the pane, and
    // a leading rule there is a line under nothing.
    ui.heading("Axes");
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
    choice_row(ui, "Now-line", &mut cfg.orientation, &sides);
    // One control for both ends, because the two ends are one thing: the
    // window onto the analyzer's axis. Dragged in MIDI note (which is what
    // makes it a log-frequency zoom) and read out in Hz. Labelled, since a
    // RangeBar carries no label of its own and the heading above is naming
    // the group rather than this bar.
    ui.label("Pitch range");
    RangeBar::new(
        &mut cfg.low_midi,
        &mut cfg.high_midi,
        harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI..=harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI,
    )
    .min_span(crate::PITCH_RANGE_MIN_SPAN)
    .display(hz_readout)
    .show(ui)
    .on_hover_text(
        "The slice of the spectrum on show. Drag either end to move it, drag \
         between them to slide the whole range (it squishes when it meets an \
         end), double-click for the full axis. The scale is logarithmic — \
         equal distances are equal musical intervals — so an octave is the \
         same width wherever it sits.\n\nOr set it on the display itself: drag \
         the Analyzer pane across the pitch axis to pan the range, scroll to \
         zoom it around the pointer. (Dragging the other way, along time, zooms \
         the roll's Span instead.)",
    );
    // No choice of what the gridlines say. They are the analyzer-standard
    // 1-2-5 frequency series, and were switchable to a line at every C with
    // Bitwig octave numbers — which is what the note NAMES on the ribbons
    // already say, in the lattice's own spelling, at the pitch they are
    // sounding rather than at the nearest C below it.
    ValueBar::new(&mut cfg.marking_scale, crate::SCALE_BAR_RANGE, "Label size")
        .show(ui)
        .on_hover_text(
            "Size of the pane's own markings: the label on each gridline, and \
             the pitch readout that follows the pointer. Fixed against the \
             zoom, unlike the note names on the ribbons -- a marking says what \
             the axis is, and the axis does not change size when you zoom it",
        );

    // ---- Audio spectrum -------------------------------------------------
    // Always analyzed: the pane IS the analyzer, the spectrogram reads the
    // same buckets, and giving the whole depth axis to the roll is what the
    // divider is for.
    section(ui, "Spectrum");
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

    // Both ends of the height scale on one control, like the pitch range: the
    // window on the spectrum's dynamics rather than just where it bottoms out.
    // Named by a label of its own — a RangeBar carries no label, and the
    // section heading above it is naming the whole group here, not this bar.
    ui.label("Level");
    RangeBar::new(&mut cfg.floor_db, &mut cfg.ceiling_db, crate::LEVEL_MIN_DB..=crate::LEVEL_MAX_DB)
        .min_span(crate::LEVEL_RANGE_MIN_SPAN)
        .display(db_readout)
        .show(ui)
        .on_hover_text(
            "The slice of the level scale on show. The low end is what reads \
             as silence; the high end is what reads as full height — and as \
             the brightest spectrogram cell — so pulling it down from 0 dB (a \
             full-scale sine) lifts quiet material into the whole picture \
             instead of the bottom of it. Drag either end to move it, drag \
             between them to slide the window, double-click for the full \
             scale.",
        );
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
        // Monospace: five signed numbers side by side, where a
        // proportional face gives "0.0" and "-1.5" different widths and
        // leaves the row of buttons visibly uneven. Digits of one width
        // make it a scale.
        for step in crate::TILT_STEPS {
            let label = egui::RichText::new(format!("{step:.1}")).monospace();
            ui.selectable_value(&mut cfg.tilt, step, label);
        }
    });

    ValueBar::new(&mut cfg.keyline, 0.0..=1.0, "Edge").show(ui).on_hover_text(
        "A light rim along the spectrum's profile and around each note \
         ribbon. Both sit over the spectrogram, whose colors run from black \
         to near-white, so either can end up the same brightness as what is \
         behind it and lose its shape. 0 draws none.",
    );

    // ---- Piano roll -----------------------------------------------------
    section(ui, "Piano roll");
    ui.checkbox(&mut cfg.show_roll, "Note history").on_hover_text(
        "Draw incoming MIDI as a scrolling roll over the same pitch axis. \
         Time runs away from the spectrum, so a note leaving the roll meets \
         the peak it is making.",
    );
    ValueBar::new(&mut cfg.roll_seconds, crate::ROLL_SECONDS_MIN..=crate::ROLL_SECONDS_MAX, "Span")
        .eased(true)
        .decimals(1)
        .display(span_readout)
        .show(ui)
        .on_hover_text(
            "Seconds of history the roll spans end to end, up to 10 minutes. \
             The scale is logarithmic, so the short spans you live in get most \
             of the travel. The spectrogram fills the most recent few minutes \
             of a long span; the notes span the whole of it.\n\nOr set it on the \
             display itself: drag the roll along the time axis, away from the \
             now-line to zoom in.",
        );
    ValueBar::new(&mut cfg.roll_thickness, 0.2..=2.0, "Note width")
        .show(ui)
        .on_hover_text(
            "Ribbon width in semitones of the pitch axis — so it holds its \
             musical meaning as the pitch range is zoomed, and a note is as \
             wide as the interval it would cover",
        );
    ui.checkbox(&mut cfg.note_names, "Note names").on_hover_text(
        "Write each note's name on its own ribbon, at the leading edge — so a \
         held note's name waits at the now-line and travels off with the note \
         when you let go. For reading the heatmap: a band of energy sits at \
         some height on an axis marked only every octave, and the ribbon over \
         that band is the same note, so naming the ribbon names the band.\n\n\
         Names are the lattice's own, in its own hand: the node's spelling \
         with its accidental and comma mark, so a just third reads E- rather \
         than as an E and a cents offset.\n\nWhere repeats of a note come too \
         fast to name each one, the first keeps its name and the next waits \
         for clear room — except a note you are holding, which is always \
         named. Needs Note history on: a name labels a ribbon.",
    );
    ui.add_enabled_ui(cfg.note_names && cfg.show_roll, |ui| {
        ValueBar::new(&mut cfg.note_name_scale, crate::SCALE_BAR_RANGE, "Name size")
            .show(ui)
            .on_hover_text(
                "Overall size of those names.\n\nThey already follow the pitch \
                 zoom, in proportion: a ribbon's width is set in semitones, so \
                 narrowing the range fattens every ribbon, and a name keeps the \
                 same relation to the ribbon it is written on -- five times its \
                 size here at the tightest two-octave range. This sets what \
                 that size is",
            );
    });
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
    ui.checkbox(&mut cfg.show_spectrogram, "Heatmap").on_hover_text(
        "A frequency-vs-time heatmap of the audio, drawn in the roll's \
         region on the same time axis — so each column of energy lines up \
         with the notes that made it. Reads the Spectrum's Level and Tilt \
         for intensity. Turn Note history off to see the heatmap alone.",
    );
    choice_row(
        ui,
        "Palette",
        &mut cfg.spectrogram_color,
        &[
            (SpectrogramColor::Mono, "Mono", "Grayscale; the most neutral over the roll"),
            (SpectrogramColor::Ice, "Ice", "Black-blue-cyan-white"),
            (SpectrogramColor::Aurora, "Aurora", "Violet-teal-green-yellow (even ramp)"),
            (SpectrogramColor::Magma, "Magma", "Indigo-magenta-orange-cream (even ramp)"),
        ],
    );
    // The palette is the whole of it. An opacity, a contrast curve and a
    // private level range would each go here and each is deliberately absent —
    // see [`spectrogram_color`](crate::SpectrumConfig::spectrogram_color) for
    // why the neutral setting is the only one worth having.
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

/// Depth budget for the spectrum curve, as a fraction of the *spectrum's
/// share* of the depth axis: a full-scale (0 dB) sine tops out here, leaving
/// the last stretch as headroom so a loud partial doesn't run into the pane's
/// edge.
const PLOT_HEIGHT_FRACTION: f32 = 0.85;

/// The 1 kHz pivot of the tilt slope, as a MIDI pitch.
const TILT_PIVOT_MIDI: f32 = 83.213_1;

/// Point size of an axis gridline's label — a dozen standing marks that
/// should stay quiet.
///
/// Doubled from the 10 it was drawn at before the Label size bar existed. The
/// bar went to 2 the first time it was tried and stayed there — so the number
/// was wrong rather than the bar wanted, and rebasing it leaves the bar
/// reading 1 at the size the pane is actually read at.
pub(super) const MARKING_PT: f32 = 20.0;

/// The whole pitch axis, in semitones — the widest the range opens, and the
/// zoom the note names' built-in size is dialled for.
const FULL_PITCH_SPAN: f32 =
    harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI;

/// How much bigger a note name draws with the pitch range zoomed to `span`
/// semitones: 1 across the whole axis, growing in PROPORTION as it narrows.
///
/// The names are written ON the ribbons, and a ribbon's width is set in
/// SEMITONES — so zooming the pitch range draws every ribbon thicker by
/// exactly the zoom factor while leaving the name that names it the size it
/// was. That is the mismatch this answers, and proportional answers it
/// squarely: a name keeps a constant share of the pitch axis, so it holds the
/// same relation to its ribbon at every zoom and the picture simply gets
/// bigger, type and all.
///
/// Which turns "how large can a name get" into "how far may the range be
/// CLOSED", and that is answered where it belongs — see
/// [`PITCH_RANGE_MIN_SPAN`](crate::PITCH_RANGE_MIN_SPAN), whose two octaves
/// against this ten-octave axis are what cap a name at five times its dialled
/// size. One law with a limit at the end beats a law that bends in the middle:
/// a softened curve holds the far end down by making a name disagree with its
/// ribbon at every zoom in between, which is where the reading actually
/// happens.
///
/// Thinning follows for free: [`plan`](super::names::plan) measures the room a
/// name demands from the size it is drawn at, so names that have grown are
/// spaced further apart and the ones that no longer fit are dropped, exactly
/// as they are at any other size.
///
/// Never SMALLER than the built-in size: the reference is the widest range the
/// axis offers, so the only direction left is up.
///
/// Says nothing about which way the pane is turned. The span is in semitones,
/// not in points, so a Top or Bottom pane (whose pitch axis is horizontal) scales
/// exactly as a wide one does — [`Axes`] is the only thing here that knows a
/// screen side, and this is not it.
fn name_zoom(span: f32) -> f32 {
    FULL_PITCH_SPAN / span.clamp(crate::PITCH_RANGE_MIN_SPAN, FULL_PITCH_SPAN)
}

/// The pane the sizes here are quoted against: a pitch axis 860 points long,
/// which is what the Spectral pane gets in the 1512x886 window they were
/// dialled in. Halve the pane and the type halves with it, along with every
/// ribbon and every band it is written over.
///
/// The PITCH axis, of the two: a ribbon's width is set in semitones, so that
/// is the axis whose length decides how big the picture under a name is. It is
/// also the short side, and so the one a window resize tends to take first.
pub(super) const REFERENCE_PITCH_LEN: f32 = 860.0;

/// The sizes this pane sets text at for one frame, as multiples of each
/// piece's built-in point size.
///
/// Three factors go into each: the pane's own size against
/// [`REFERENCE_PITCH_LEN`], the user's bar for that kind of text, and — for
/// the note names alone — the pitch zoom. Both come out snapped, since a scale
/// that follows a continuous zoom otherwise asks egui for a new font size on
/// every frame of a drag (see [`crate::text::snap_scale`]).
#[derive(Clone, Copy, Debug)]
struct TextScales {
    /// The axis gridline labels and the hover readout.
    markings: f32,
    /// The name written on each ribbon.
    names: f32,
}

fn text_scales(cfg: &crate::SpectrumConfig, axes: &Axes, span: f32, ppp: f32) -> TextScales {
    let pane = axes.pitch_len() / REFERENCE_PITCH_LEN;
    TextScales {
        markings: crate::text::snap_scale(pane * cfg.marking_scale, MARKING_PT, ppp),
        names: crate::text::snap_scale(
            pane * cfg.note_name_scale * name_zoom(span),
            names::LABEL_PT,
            ppp,
        ),
    }
}

/// How loud `power` reads at pitch `midi`, on a 0..1 scale: the configured
/// floor is 0, the configured ceiling is 1, and the tilt lifts treble by
/// its slope above the 1 kHz pivot. The spectrum curve's height and the
/// spectrogram's cell intensity both read from this, so the two always agree
/// on what "loud" means for a given bucket.
pub(crate) fn loudness(cfg: &crate::SpectrumConfig, power: f32, midi: f32) -> f32 {
    loudness_db(cfg, power_db(power), midi)
}

/// A bucket's power as dB — the form the spectrogram's history already stores,
/// and the only thing [`loudness`] does with power before mapping it.
pub(crate) fn power_db(power: f32) -> f32 {
    10.0 * power.max(1e-12).log10()
}

/// [`loudness`] from a bucket already in dB, so the heatmap (whose columns are
/// stored that way) never takes a `log10` per pixel.
///
/// The heatmap reads exactly this and nothing of its own. Giving it a private dB
/// window and a contrast curve is tempting — a curve is read as a shape against
/// a baseline and a picture is read as a picture, so the ranges that suit them
/// need not coincide. What that argument misses is that one range IS what makes
/// "loud" the same claim in both halves of one pane, and a second one is a
/// second thing to keep in step.
pub(crate) fn loudness_db(cfg: &crate::SpectrumConfig, power_db: f32, midi: f32) -> f32 {
    let db = power_db - cfg.tilt * (midi - TILT_PIVOT_MIDI) / 12.0;
    // Never trust the pair to be ordered or apart, exactly as the pitch range
    // is not trusted: the bar can't produce a collapsed one, a hand-edited
    // state blob can, and dividing by its zero span paints NaN geometry that
    // takes the editor — and with it the host — down.
    let ceiling = cfg.ceiling_db.max(cfg.floor_db + crate::LEVEL_RANGE_MIN_SPAN);
    ((db - cfg.floor_db) / (ceiling - cfg.floor_db)).clamp(0.0, 1.0)
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
/// names a screen side. Four layouts, named for the side the now-line is on —
/// **Left** (time left->right, pitch bottom->top), **Right** (the same flipped
/// along time), **Top** (time top->bottom, pitch left->right) and **Bottom**.
/// In all four the spectrum sits at the now-line end.
#[derive(Clone, Copy)]
pub(super) struct Axes {
    pub rect: egui::Rect,
    /// Time (the depth axis) runs down the pane rather than along it, with
    /// pitch across. See [`SpectralOrientation`](crate::SpectralOrientation).
    time_vertical: bool,
    /// Time runs against its screen axis — leftward, or upward.
    time_reversed: bool,
}

impl Axes {
    pub(super) fn new(rect: egui::Rect, cfg: &crate::SpectrumConfig) -> Axes {
        Axes {
            rect,
            time_vertical: cfg.orientation.is_time_vertical(),
            time_reversed: cfg.orientation.is_time_reversed(),
        }
    }

    /// The screen point at pitch fraction `p` and depth (time) fraction `d`.
    pub fn at(&self, p: f32, d: f32) -> egui::Pos2 {
        // Depth measured from whichever edge the now-line is on. Reversed, the
        // past runs back toward the origin of the screen axis; pitch is not
        // touched by it, so it keeps reading low-to-high the conventional way.
        let d = if self.time_reversed { 1.0 - d } else { d };
        if self.time_vertical {
            // Time runs down the pane (Top) or up it (Bottom); pitch runs
            // across, low left to high right.
            egui::pos2(
                self.rect.left() + self.rect.width() * p,
                self.rect.top() + self.rect.height() * d,
            )
        } else {
            // Time runs along the pane, rightward (Left) or leftward (Right);
            // pitch climbs, low bottom to high top.
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
    pub(super) fn dir_pitch(&self) -> egui::Vec2 {
        (self.at(1.0, 0.0) - self.at(0.0, 0.0)).normalized()
    }

    /// Which way the depth axis points on screen (unit vector).
    pub(super) fn dir_depth(&self) -> egui::Vec2 {
        (self.at(0.0, 1.0) - self.at(0.0, 0.0)).normalized()
    }

    /// A line clean across the depth axis at pitch `p` — the shape of
    /// every pitch gridline.
    pub fn across_depth(&self, p: f32) -> [egui::Pos2; 2] {
        [self.at(p, 0.0), self.at(p, 1.0)]
    }

    /// A line clean across the pitch axis at depth `d` — the shape of the
    /// roll's "now" line and the divider.
    pub fn across_pitch(&self, d: f32) -> [egui::Pos2; 2] {
        [self.at(0.0, d), self.at(1.0, d)]
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

    /// The depth fraction under a screen position — the inverse of the
    /// depth half of [`at`](Self::at). Unclamped.
    fn depth_at(&self, pos: egui::Pos2) -> f32 {
        let d = if self.time_vertical {
            (pos.y - self.rect.top()) / self.rect.height().max(1.0)
        } else {
            (pos.x - self.rect.left()) / self.rect.width().max(1.0)
        };
        if self.time_reversed { 1.0 - d } else { d }
    }

    /// A grab band `half` pixels either side of depth `d`, spanning the
    /// pitch axis — the splitter's hit area. Kept inside the pane, so the
    /// handle stays grabbable when the split is pushed all the way to an
    /// edge.
    fn depth_band(&self, d: f32, half: f32) -> egui::Rect {
        let band = egui::Rect::from_two_pos(self.at(0.0, d), self.at(1.0, d))
            .expand2(self.dir_depth().abs() * half);
        band.intersect(self.rect)
    }

    /// Anchor and alignment for a text label at `(p, d)`, offset `along`
    /// pixels up the pitch axis and `into` pixels up the depth axis, and
    /// growing in those same directions. One helper covers both
    /// orientations: the growth direction is read off the axes rather
    /// than case-matched per side.
    pub(super) fn text_anchor(
        &self,
        p: f32,
        d: f32,
        along: f32,
        into: f32,
    ) -> (egui::Pos2, egui::Align2) {
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
pub(super) fn spectrum_share(cfg: &crate::SpectrumConfig) -> f32 {
    if cfg.show_roll || cfg.show_spectrogram {
        (1.0 - cfg.roll_fraction).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Half-width of the divider's grab band, in points. Wider than the hairline
/// it drags: the band is invisible until the pointer is inside it, so it has
/// to forgive an aim that is a few points off the line.
const SPLIT_GRAB_HALF: f32 = 6.0;

/// The spectrum/far-region divider, draggable where it is drawn.
///
/// Deliberately NOT a dock separator: the spectrum, spectrogram and roll are
/// one pane on purpose — the spectrum's baseline sits ON this line and the
/// roll's notes arrive at it, which is how you see they are sounding — so
/// making them separate
/// panes would put that shared calibration across a pane boundary (and dock
/// panes detach, which this divider must not). Instead the handle drags
/// [`roll_fraction`](crate::SpectrumConfig::roll_fraction) directly, so the
/// drag persists with the rest of the UI state. It is the only way to set the
/// split: a "Roll share" bar on the Analyzer tab would be a second control for
/// the same field, and dragging the boundary you can see beats aiming a number
/// at it.
///
/// Returns the handle's response so the caller can paint its highlight last,
/// over the plots. `surface` keeps the docked pane and the Video preview from
/// sharing one interaction id.
fn drag_split(
    ui: &mut egui::Ui,
    axes: &Axes,
    state: &mut SharedState,
    surface: usize,
) -> egui::Response {
    let band = axes.depth_band(spectrum_share(&state.spectrum_config), SPLIT_GRAB_HALF);
    let response = ui
        .interact(band, egui::Id::new(("spectral-split", surface)), Sense::drag())
        .on_hover_cursor(if axes.time_vertical {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        });
    if let Some(pointer) = response.dragged().then(|| response.interact_pointer_pos()).flatten() {
        // Track the pointer absolutely rather than accumulating deltas: pushed
        // against either limit, an accumulated split drifts out of sync and
        // stops following the cursor on the way back. The band is a few pixels
        // wide, so grabbing it off-center snaps imperceptibly. Depth runs away
        // from the spectrum, so the roll gets what is left.
        state.spectrum_config.roll_fraction = (1.0 - axes.depth_at(pointer)).clamp(0.0, 1.0);
    }
    response
}

/// The `surface` that is the real, docked Analyzer pane. Slot 1 is the Video
/// tab's preview and the offline renderer never has a pointer, so this is the
/// one copy of the pane a person can actually navigate.
const DOCKED_SURFACE: usize = 0;

/// How much one point of scroll zooms, as an exponent — a full notch of a
/// mouse wheel (~50 points) closes the range by about a third, and a trackpad's
/// finer stream lands proportionally.
const ZOOM_PER_SCROLL_POINT: f32 = 0.008;

/// How much one point of drag along the time axis zooms the Span, as an
/// exponent. Sized so the whole 1..600 s range is a comfortable sweep and back:
/// from the 12 s default, ~410 points of drag toward the past reaches 1 s and
/// ~650 the other way reaches 600 s.
const TIME_ZOOM_PER_DRAG_POINT: f32 = 0.006;

/// How far a drag must lean along the time axis before it is a Span zoom rather
/// than a pitch pan, in points. Panning is the default — the axes do NOT both
/// move at once, because the Span is a value being dialled in (it shows on the
/// Analyzer tab's bar), and a pitch pan with a few points of horizontal slop in
/// it would leave the time axis quietly breathing. The margin is small enough
/// that the pitch the picture slides by before a zoom takes over is a fraction
/// of a semitone.
const TIME_ZOOM_LEAN: f32 = 4.0;

/// Drag or scroll to navigate the picture instead of aiming the Analyzer tab's
/// bars at it: across the pitch axis to pan the range, the wheel to zoom it,
/// and along the time axis to zoom the Span. All three write
/// [`SpectrumConfig`](crate::SpectrumConfig) — `low_midi`/`high_midi` and
/// `roll_seconds` — so a navigated view persists and reads back out on those
/// bars.
///
/// The two axes carry DIFFERENT gestures because they are not the same kind of
/// axis. The pitch range is a window that can sit anywhere on the analyzer's
/// axis, so it pans and zooms. The time axis is anchored: its near edge is
/// always `now`, so there is nothing to pan to — which leaves a drag along it
/// free to mean zoom, and means the zoom is always about the now-line rather
/// than about the pointer the way the pitch wheel is. Dragging toward the past
/// therefore spreads the picture away from the now-line, i.e. zooms in.
///
/// A Span zoom is only taken from a drag that STARTED in the far region, where
/// the time axis actually is. Over the spectrum's own share the depth axis is
/// dB, not time, so a drag there would be moving something that isn't under the
/// hand.
///
/// Docked pane only. The Video tab draws this same pane as its preview, and
/// that tab's body is a vertical `ScrollArea` — a wheel spent zooming there is
/// a wheel the tab cannot be scrolled with, which is the only thing
/// that keeps its controls reachable when the preview squeezes it. (The
/// divider is a different case and stays live in the preview: it is a handle
/// you aim at, not a gesture over the whole surface.)
fn drag_zoom(
    ui: &egui::Ui,
    axes: &Axes,
    response: &egui::Response,
    state: &mut SharedState,
    surface: usize,
) {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};

    if surface != DOCKED_SURFACE {
        return;
    }
    // Where the far region begins, and so which drags have a time axis under
    // them. At 1.0 there is no far region at all (both layers off, or the
    // divider dragged shut) and the Span has nothing on screen to zoom.
    let split = spectrum_share(&state.spectrum_config);
    let cfg = &mut state.spectrum_config;
    let mut low = cfg.low_midi;
    let mut span = (cfg.high_midi - cfg.low_midi).max(crate::PITCH_RANGE_MIN_SPAN);
    // Nothing is written unless a gesture actually moved something. The range
    // is also the Analyzer tab's range bar, and rewriting it every frame would
    // put this function's rounding between that bar and the value it shows.
    let mut moved = false;

    // Zoom about the pitch under the pointer, so the note being looked at
    // stays put while the range closes in on it. `contains_pointer` rather
    // than `hovered()`: the divider sits on top of the pane, and the wheel
    // should keep zooming while the pointer crosses it.
    if response.contains_pointer() {
        // Wheel and trackpad pinch both zoom. egui routes ctrl+wheel and pinch
        // into `zoom_delta` and zeroes the scroll for them, so the two can't
        // double up here.
        let (scroll, pinch) = ui.ctx().input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * ZOOM_PER_SCROLL_POINT).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            let anchor = ui
                .ctx()
                .pointer_hover_pos()
                .map_or(0.5, |p| axes.pitch_at(p).clamp(0.0, 1.0));
            let held = low + anchor * span;
            span = (span / factor).clamp(crate::PITCH_RANGE_MIN_SPAN, widest_span());
            low = held - anchor * span;
            moved = true;
        }
    }

    // Grab the picture. Per-frame deltas rather than the absolute tracking the
    // divider uses — pushed against an end of an axis, an absolute anchor would
    // keep accumulating off-screen and the view would sit still on the way back.
    if response.dragged() {
        let delta = response.drag_delta();
        // Which axis this drag is on, decided from the TOTAL travel since the
        // press: a per-frame decision would flip between the two on the jitter
        // of a slow drag, and this way an L-shaped drag simply hands over once
        // its second leg is the longer one.
        let along_time = split < 1.0
            && ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .zip(response.interact_pointer_pos())
                .is_some_and(|(from, at)| {
                    let total = at - from;
                    axes.depth_at(from) >= split
                        && total.dot(axes.dir_depth()).abs()
                            > total.dot(axes.dir_pitch()).abs() + TIME_ZOOM_LEAN
                });
        if along_time {
            // Dragging toward the past pulls the picture away from the now-line
            // it is anchored on, spreading it — so the seconds it spans shrink.
            let along = delta.dot(axes.dir_depth());
            cfg.roll_seconds = (cfg.roll_seconds * (-along * TIME_ZOOM_PER_DRAG_POINT).exp())
                .clamp(crate::ROLL_SECONDS_MIN, crate::ROLL_SECONDS_MAX);
            ui.ctx().set_cursor_icon(if axes.time_vertical {
                egui::CursorIcon::ResizeVertical
            } else {
                egui::CursorIcon::ResizeHorizontal
            });
        } else {
            // The pitch under the pointer travels with it, so the range moves
            // the opposite way.
            let along = delta.dot(axes.dir_pitch());
            low -= along / axes.pitch_len().max(1.0) * span;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            moved = true;
        }
    }

    if !moved {
        return;
    }
    // Land inside the axis the analyzer actually covers. Same invariant
    // `SpectrumConfig::migrate_legacy` enforces on load: a range past the axis
    // draws a band with no buckets behind it. The upper bound takes a `max`
    // because at the full span the two ends meet, and float rounding is enough
    // to cross them — which `clamp` answers with a panic.
    let span = span.clamp(crate::PITCH_RANGE_MIN_SPAN, widest_span());
    cfg.low_midi = low.clamp(SPECTRUM_MIN_MIDI, (SPECTRUM_MAX_MIDI - span).max(SPECTRUM_MIN_MIDI));
    cfg.high_midi = cfg.low_midi + span;
}

/// The whole axis the analyzer covers, as a span — the widest the pitch range
/// can be zoomed out to.
fn widest_span() -> f32 {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    SPECTRUM_MAX_MIDI - SPECTRUM_MIN_MIDI
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

    /// Depth for take time `t`, WITHOUT clamping it into the region.
    ///
    /// For geometry that is meant to overhang the region's edge and be cut off
    /// by the pane's scissor rather than squashed against it — a note ribbon
    /// leaving the far end still owes its outline and rim, and clamping paints
    /// those as a blob sitting on the edge instead of sliding out under it.
    /// Only safe for times the caller has already bounded: an unbounded one
    /// gives an unbounded depth, and a note that started ten minutes ago would
    /// become a quad ten minutes long.
    pub(super) fn depth_of_unclamped(&self, t: f64) -> f32 {
        self.split + self.frac(t) as f32 * self.depth_span
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
/// Hovering a pitch here highlights the matching lattice node (if one is in
/// view) and reads the pitch out. Nothing comes back the other way: lighting
/// a band here for the lattice-hovered pitch class, in every octave, puts a
/// stripe across the whole picture — too loud an answer to a pointer resting
/// somewhere else.
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
    // Drag-sensing, so the pitch range can be panned and the time Span zoomed
    // by grabbing the picture (see `drag_zoom`). Registered BEFORE the
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
    // Re-snapshot: the two drags above just wrote `roll_fraction` and the pitch
    // range, and everything below has to be this frame's values, not the ones
    // from before the drag.
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
    // composites to black at every alpha. Drawn under the gridlines, so they
    // still read as pitch lanes across the region.
    if cfg.show_spectrogram && split < 1.0 {
        let bed = egui::Rect::from_two_pos(axes.at(0.0, split), axes.at(1.0, 1.0));
        painter.rect_filled(bed, 0.0, egui::Color32::BLACK);
    }

    // Axis gridlines: the analyzer-standard 1-2-5 frequency series, and only
    // that. The alternative is a line at every C with Bitwig octave numbers,
    // which answers a question the pane answers better elsewhere — every ribbon
    // carries its note NAME, spelled the lattice's way and placed at the pitch
    // that is sounding. What an axis is for is the other reading: where in the
    // spectrum a band sits, which is a frequency.
    //
    // The lines run the full depth, so they double as the roll's pitch lanes.
    // They lay down here, under the spectrum; their text labels are collected
    // and drawn last (below the voice bars), so a loud spectrum slab never
    // buries which pitch a lane is.
    let gridline = |p: f32| {
        painter.line_segment(axes.across_depth(p), egui::Stroke::new(1.0, theme::panel()));
    };
    let mut axis_labels: Vec<(f32, String)> = Vec::new();
    for hz in [20.0f32, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0] {
        let midi = hz_to_midi(hz);
        if !scale.contains(midi) {
            continue;
        }
        let t = scale.t_of(midi);
        gridline(t);
        let label = if hz >= 1_000.0 { format!("{}k", hz / 1_000.0) } else { format!("{hz}") };
        axis_labels.push((t, label));
    }

    // Nothing to pump here: the analyzer runs off the samples the shell pushes
    // (see `AudioSpectrum::push_samples`), so the spectrogram's columns arrive
    // whether or not the curve is drawn — and whether or not this pane is.
    // The far share of the depth axis: a spectrogram heatmap of the audio
    // and/or the piano roll of what has been played, both on the same
    // `now`-anchored time axis. The spectrogram lays down first (it's a
    // background); the roll's ribbons sit over it, and the live spectrum
    // curve over everything. Turning the ribbons off (`show_roll`) with the
    // spectrogram on leaves the heatmap alone.
    if split < 1.0 && cfg.show_spectrogram {
        super::spectrogram::draw_spectrogram(&painter, &axes, &scale, state, split, now, surface);
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
                super::spectrogram::cell_color(cfg.spectrogram_color, loudness(&cfg, power, midi))
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
            if let Some(edge) = super::roll::keyline(&cfg, 1.0) {
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
        // Stable order: voices iterate a HashMap and translucent bands
        // accumulate where they overlap, so the offline render must not
        // depend on it.
        voices.sort_unstable_by(|a, b| {
            a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
        });
        let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
        for voice in voices {
            let strength = voice.activation(now, state.frame_params.fade_time);
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

    // The now-line, where the roll hands over to the spectrum — drawn here,
    // after both things it divides, rather than at the end of the roll. It
    // marks the boundary between two pictures, so it has to sit ON them: from
    // inside the roll it goes down before the spectrum curve and the curve's
    // fill paints over it, and the spectrogram's quad reaches the same line
    // from the other side. Either one eating into it leaves a divider that
    // flickers with the music instead of holding still.
    //
    // Whole-song mode sweeps a playhead across a static layout instead, and
    // draws its own mark above. Always drawn (there is no setting): a note
    // crossing this line out of the roll and into its spectrum peak is how the
    // pane reads as one picture, so the boundary has to be marked.
    if !whole_song && split < 1.0 && split > 0.0 {
        painter.line_segment(axes.across_pitch(split), egui::Stroke::new(1.0, theme::hairline()));
    }

    // The roll goes on last, over the line it arrives at. A sounding note
    // reaching the boundary and painting across it IS the mark that it is
    // sounding — nothing else has to be drawn to say so, and nothing drawn
    // separately could sit against a rounded ribbon end as exactly as the
    // ribbon does. (Its ribbons occupy the far side of the split and the
    // spectrum the near side, so this only changes what happens ON the line.)
    if split < 1.0 && cfg.show_roll {
        super::roll::draw_roll(&painter, &axes, &scale, state, split, now, surface);
    }

    // Axis labels last, riding on top of the spectrogram, spectrum, and
    // voice bars: a label only earns its place if you can read which pitch a
    // lane is, and a loud slab would otherwise bury it. The gridlines
    // themselves stay underneath (drawn above) as pitch lanes.
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

    // Hovering here highlights the matching lattice node, if it is in view.
    // Gated on contains_pointer (pure geometry) rather than hovered(): the
    // divider sits on top of the pane, and hovered() would drop the highlight
    // every time the pointer crossed it.
    //
    // The pitch under the cursor is no longer also printed beside it. A
    // name-and-Hz readout tracking the pointer is a second thing moving over
    // a picture whose whole subject is movement, and the axis it moves along
    // is already labelled.
    //
    // Be exact about what did NOT replace it, because the highlight below
    // looks like it should have: a lit node says WHERE the pitch under the
    // pointer lives on the lattice, and nothing about how loud it is or
    // whether anything is playing there. That is the trade; the highlight
    // does not cover the readout's job.
    //
    // `node_pointed_at`, not `nearest_visible_node` — a pointer aims, it does
    // not play, and matching it against `Tuning::tolerance` is what made this
    // hover read as a glitch. See the note on that function.
    let hover = response.contains_pointer().then(|| ui.ctx().pointer_hover_pos()).flatten();
    if let Some(pointer) = hover {
        let midi = (min_midi + axes.pitch_at(pointer) * scale.span).clamp(min_midi, max_midi);
        // Cents from C, measured from MIDI 0 (which IS a C) rather than from
        // the range's own start: the range is continuous and generally does
        // not begin on a C. Measuring from it offsets every hovered pitch
        // class by wherever the zoom happened to start, so hovering the
        // spectrum lights up the wrong lattice node.
        let pc_cents = midi.rem_euclid(12.0) * 100.0;
        state.hovered = node_pointed_at(
            &state.view,
            &state.tuning,
            harmonigraph_core::PitchClass::from_cents(pc_cents),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpectralOrientation, SpectrumConfig};
    use harmonigraph_core::{NoteEvent, NoteEventKind};

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

    /// `power` for a level in dB, undoing the 10*log10 `loudness` applies.
    fn power_at(db: f32) -> f32 {
        10.0f32.powf(db / 10.0)
    }

    /// The note names follow the pitch zoom and the markings do not, which is
    /// the whole of the difference between text written ON the picture and
    /// text labelling the axis it is drawn against.
    ///
    /// Both ends of the zoom are pinned, because both are claims: at the whole
    /// axis a name is exactly the size it was dialled at, so nothing about the
    /// default view changes, and at the tightest range the analyzer offers it
    /// is five times that — the axis being ten octaves and the floor two, and
    /// the law being a constant share of the axis rather than some softened
    /// fraction of one.
    #[test]
    fn names_follow_the_pitch_zoom_and_markings_hold_still() {
        let cfg = SpectrumConfig::default();
        let axes = axes(reference_pane(), SpectralOrientation::Left);
        let at = |span| text_scales(&cfg, &axes, span, 2.0);
        // Every size comes out snapped onto a whole physical pixel, and a name
        // is 12.35pt, which is not one at 2x — so the law is met to within half
        // a pixel of type and no closer. See `text::snap_scale`.
        let pixel = 0.5 / (names::LABEL_PT * 2.0);

        let full = at(FULL_PITCH_SPAN).names;
        assert!((full - 1.0).abs() <= pixel, "the whole axis draws names at {full}, not 1");
        let tightest = at(crate::PITCH_RANGE_MIN_SPAN).names;
        assert!(
            (tightest - FULL_PITCH_SPAN / crate::PITCH_RANGE_MIN_SPAN).abs() <= pixel,
            "the tightest range draws names at {tightest}, not in proportion to its zoom",
        );
        // Monotone in between, and never under the size it started at: the
        // reference is the widest range there is, so the only way is up.
        let mut previous = 0.0;
        for span in [FULL_PITCH_SPAN, 96.0, 60.0, 36.0, crate::PITCH_RANGE_MIN_SPAN] {
            let names = at(span).names;
            assert!(names >= previous, "{span} semitones drew smaller names than the span above");
            previous = names;
        }
        // A range zoomed past either end (a hand-edited blob; the bars cannot
        // do it) still lands inside the band rather than off it.
        assert_eq!(at(0.0).names, tightest);
        assert_eq!(at(1e6).names, full);

        // The markings ignore all of it, and answer to their own bar.
        assert_eq!(at(FULL_PITCH_SPAN).markings, at(crate::PITCH_RANGE_MIN_SPAN).markings);
        let bigger = SpectrumConfig { marking_scale: 2.0, ..SpectrumConfig::default() };
        let doubled = text_scales(&bigger, &axes, 24.0, 2.0).markings;
        // Within a rung of the size ladder, which is what the bar's 2 is
        // rounded onto — see `text::snap_scale`.
        assert!((doubled / 2.0 - 1.0).abs() <= 0.04, "the bar's 2 drew at {doubled}");
        assert_eq!(
            text_scales(&bigger, &axes, 24.0, 2.0).names,
            at(24.0).names,
            "and the two bars are independent",
        );
    }

    /// A pane at the size these sizes were chosen at, so a test about anything
    /// else is not also a test about the pane being some other size.
    fn reference_pane() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, REFERENCE_PITCH_LEN))
    }

    /// Text shrinks with the pane it is drawn on — every kind of it, and in
    /// proportion.
    ///
    /// This is one mechanism doing three jobs that were three mechanisms: a
    /// window dragged narrower, the Render preview drawing this pane small,
    /// and the offline render drawing it large. A kind of text that missed it
    /// would come out at some other size in the video than in the pane the
    /// look was dialled in on, which is the divergence this codebase least
    /// wants.
    #[test]
    fn text_shrinks_with_the_pane() {
        let cfg = SpectrumConfig::default();
        let half = egui::Rect::from_min_size(
            reference_pane().min,
            egui::vec2(300.0, REFERENCE_PITCH_LEN * 0.5),
        );
        let full = axes(reference_pane(), SpectralOrientation::Left);
        let small = axes(half, SpectralOrientation::Left);
        let docked = text_scales(&cfg, &full, 48.0, 2.0);
        let shrunk = text_scales(&cfg, &small, 48.0, 2.0);
        assert!((shrunk.names / docked.names - 0.5).abs() < 0.02);
        assert!((shrunk.markings / docked.markings - 0.5).abs() < 0.02);
        // ...and at the reference pane the bars read what they say.
        assert!((docked.markings - 1.0).abs() < 0.02, "{}", docked.markings);
    }

    /// The Span readout carries its own unit, and switches to minutes at
    /// the point where seconds alone stop reading — including the seam,
    /// where a value that rounds up to a whole minute must be written as
    /// one rather than as "60.0s".
    #[test]
    fn the_span_readout_names_its_own_unit() {
        assert_eq!(span_readout(1.0), "1.0s");
        assert_eq!(span_readout(12.34), "12.3s");
        assert_eq!(span_readout(59.9), "59.9s");
        assert_eq!(span_readout(59.97), "1m 00s", "rounds up ACROSS the seam");
        assert_eq!(span_readout(60.0), "1m 00s");
        assert_eq!(span_readout(65.4), "1m 05s", "seconds are padded, so the width holds");
        assert_eq!(span_readout(90.0), "1m 30s");
        assert_eq!(span_readout(600.0), "10m 00s", "the top of the bar's range");
    }

    /// The level range is a window with two ends: the floor reads as silence
    /// and the ceiling as full height, wherever each is put. Pulling the
    /// ceiling down is what lets quiet material fill the picture.
    #[test]
    fn the_level_range_maps_floor_to_zero_and_ceiling_to_one() {
        // No tilt, so the pivot pitch drops out and dB is dB.
        let cfg =
            SpectrumConfig { floor_db: -60.0, ceiling_db: 0.0, tilt: 0.0, ..Default::default() };
        let at = |db| loudness(&cfg, power_at(db), TILT_PIVOT_MIDI);
        assert!(at(-60.0).abs() < 1e-4, "the floor is silence");
        assert!((at(0.0) - 1.0).abs() < 1e-4, "the ceiling is full height");
        assert!((at(-30.0) - 0.5).abs() < 1e-4, "and it is linear in dB between them");
        assert_eq!(at(-90.0), 0.0, "under the floor stays at silence");

        // A ceiling pulled down onto the material lifts it to full height,
        // which the fixed 0 dB top could not do.
        let quiet =
            SpectrumConfig { floor_db: -60.0, ceiling_db: -30.0, tilt: 0.0, ..Default::default() };
        assert!((loudness(&quiet, power_at(-30.0), TILT_PIVOT_MIDI) - 1.0).abs() < 1e-4);
        assert!((loudness(&quiet, power_at(-45.0), TILT_PIVOT_MIDI) - 0.5).abs() < 1e-4);
    }

    /// A hand-edited state blob can carry a collapsed or inverted pair; the
    /// bar cannot. Unclamped that divides by zero and paints NaN geometry,
    /// which egui panics on — inside the host, for a plugin.
    #[test]
    fn a_collapsed_level_range_still_maps_to_a_finite_number() {
        for (floor, ceiling) in [(-60.0, -60.0), (-20.0, -80.0), (0.0, 0.0)] {
            let cfg = SpectrumConfig {
                floor_db: floor,
                ceiling_db: ceiling,
                tilt: 0.0,
                ..Default::default()
            };
            for db in [-120.0, -60.0, -12.0, 0.0] {
                let level = loudness(&cfg, power_at(db), TILT_PIVOT_MIDI);
                assert!(
                    level.is_finite() && (0.0..=1.0).contains(&level),
                    "{floor}..{ceiling} dB at {db} dB gave {level}",
                );
            }
        }
    }

    /// Every orientation the pane offers — the loop the axis tests run over.
    ///
    /// [`SpectralOrientation::ALL`], not a second list of the same four names:
    /// that one is built through an exhaustive `match`, so a fifth variant
    /// fails to compile until it is added and every sweep below picks it up.
    /// A literal here would leave the sweeps quietly covering four of five.
    const EVERY_ORIENTATION: [SpectralOrientation; 4] = SpectralOrientation::ALL;

    /// Each orientation puts the NOW-line on the side it is named for, which
    /// is the whole meaning of the setting: that is where the spectrum sits,
    /// where a ribbon arrives, and where the heatmap's newest column is. The
    /// far corner pins the direction time then runs in.
    #[test]
    fn the_now_line_lands_on_the_side_the_orientation_names() {
        // WIDE is (10, 20)..(310, 120): 300 across, 100 down.
        let now_and_past = |o| {
            let a = axes(WIDE, o);
            (a.at(0.0, 0.0), a.at(0.0, 1.0))
        };
        assert_eq!(
            now_and_past(SpectralOrientation::Left),
            (egui::pos2(10.0, 120.0), egui::pos2(310.0, 120.0)),
            "now on the left, past to the right",
        );
        assert_eq!(
            now_and_past(SpectralOrientation::Right),
            (egui::pos2(310.0, 120.0), egui::pos2(10.0, 120.0)),
            "now on the right, past to the left",
        );
        assert_eq!(
            now_and_past(SpectralOrientation::Top),
            (egui::pos2(10.0, 20.0), egui::pos2(10.0, 120.0)),
            "now along the top, past below",
        );
        assert_eq!(
            now_and_past(SpectralOrientation::Bottom),
            (egui::pos2(10.0, 120.0), egui::pos2(10.0, 20.0)),
            "now along the bottom, past above",
        );
    }

    /// Pitch reads the conventional way in all four, rather than mirroring
    /// with time: low at the BOTTOM wherever time is horizontal, low at the
    /// LEFT wherever it is vertical. Flipping it along with time would turn
    /// Right and Bottom into upside-down pictures of their partners, where
    /// what they are for is the same picture arriving from the other side.
    #[test]
    fn pitch_climbs_the_same_way_in_the_pair_that_shares_an_axis() {
        for (o, low, high) in [
            (SpectralOrientation::Left, 120.0, 20.0),
            (SpectralOrientation::Right, 120.0, 20.0),
        ] {
            let a = axes(WIDE, o);
            assert_eq!(a.at(0.0, 0.5).y, low, "{o:?}: low pitch is not at the bottom");
            assert_eq!(a.at(1.0, 0.5).y, high, "{o:?}: high pitch is not at the top");
        }
        for o in [SpectralOrientation::Top, SpectralOrientation::Bottom] {
            let a = axes(WIDE, o);
            assert_eq!(a.at(0.0, 0.5).x, 10.0, "{o:?}: low pitch is not at the left");
            assert_eq!(a.at(1.0, 0.5).x, 310.0, "{o:?}: high pitch is not at the right");
        }
    }

    /// Which side is the pitch axis and which the time axis, in each pair.
    #[test]
    fn the_axes_take_the_pane_sides_the_orientation_asks_for() {
        for o in [SpectralOrientation::Left, SpectralOrientation::Right] {
            let a = axes(WIDE, o);
            assert_eq!(a.pitch_len(), 100.0, "{o:?}: pitch is the vertical side");
            assert_eq!(a.depth_len(), 300.0, "{o:?}: time is the horizontal side");
        }
        for o in [SpectralOrientation::Top, SpectralOrientation::Bottom] {
            let a = axes(TALL, o);
            assert_eq!(a.pitch_len(), 100.0, "{o:?}: pitch is the horizontal side");
            assert_eq!(a.depth_len(), 300.0, "{o:?}: time is the vertical side");
        }
    }

    /// Hover has to find the pitch the pointer is actually over, in every
    /// orientation — the lattice highlight hangs off this one inverse.
    #[test]
    fn pitch_at_inverts_at_whichever_way_the_axes_run() {
        for rect in [WIDE, TALL] {
            for orientation in EVERY_ORIENTATION {
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

    /// The divider drag reads the pointer through this inverse, so it has to
    /// agree with `at` in every orientation — a sign flip would send the
    /// handle the wrong way, and the two reversed layouts are exactly where a
    /// missing flip hides.
    #[test]
    fn depth_at_inverts_at_whichever_way_the_axes_run() {
        for rect in [WIDE, TALL] {
            for orientation in EVERY_ORIENTATION {
                let a = axes(rect, orientation);
                for step in 0..=10 {
                    let d = step as f32 / 10.0;
                    // Any pitch: the inverse reads the depth axis only.
                    let back = a.depth_at(a.at(0.37, d));
                    assert!((back - d).abs() < 1e-4, "{orientation:?}: {d} -> {back}");
                }
            }
        }
    }

    /// The grab band straddles the divider, stays inside the pane (so a
    /// divider dragged flat against an edge is still grabbable), and spans
    /// the pitch axis.
    #[test]
    fn the_split_band_straddles_the_divider_and_stays_inside_the_pane() {
        for rect in [WIDE, TALL] {
            for orientation in EVERY_ORIENTATION {
                let a = axes(rect, orientation);
                for split in [0.0, 0.5, 1.0] {
                    let band = a.depth_band(split, SPLIT_GRAB_HALF);
                    assert!(rect.contains_rect(band), "{orientation:?} @{split}: {band:?}");
                    assert!(band.contains(a.at(0.5, split)), "{orientation:?} @{split}: off-line");
                    // Thin across depth, full width across pitch.
                    let (thin, wide_) = if a.time_vertical {
                        (band.height(), band.width())
                    } else {
                        (band.width(), band.height())
                    };
                    assert!(thin <= 2.0 * SPLIT_GRAB_HALF, "{orientation:?}: band too thick");
                    assert_eq!(wide_, a.pitch_len(), "{orientation:?}: band must span pitch");
                }
            }
        }
    }

    /// Dragging the divider away from the spectrum GROWS the spectrum's
    /// share, in every orientation, by the distance dragged — the whole
    /// point of the handle, and the one thing an axis sign error breaks.
    ///
    /// The drag is taken along `dir_depth` rather than written out per case,
    /// so "away from the spectrum" means the same thing in the two reversed
    /// layouts, where it points back toward the screen's origin.
    #[test]
    fn dragging_the_divider_moves_the_split_with_the_pointer() {
        for (rect, orientation) in
            EVERY_ORIENTATION.map(|o| (if o.is_time_vertical() { TALL } else { WIDE }, o))
        {
            let a = axes(rect, orientation);
            let before = 0.5;
            let after = drag_divider(rect, orientation, before, a.dir_depth() * 30.0);
            // Depth runs away from the spectrum, so +30 px of depth takes
            // 30/depth_len off the roll's share.
            let expected = before - 30.0 / a.depth_len();
            assert!(
                (after - expected).abs() < 0.02,
                "{orientation:?}: {before} -> {after}, wanted ~{expected}",
            );
        }
    }

    /// And back the other way, into the spectrum: the roll grows.
    #[test]
    fn dragging_the_divider_into_the_spectrum_grows_the_roll() {
        for (rect, orientation) in
            EVERY_ORIENTATION.map(|o| (if o.is_time_vertical() { TALL } else { WIDE }, o))
        {
            let drag = axes(rect, orientation).dir_depth() * -30.0;
            let after = drag_divider(rect, orientation, 0.5, drag);
            assert!(after > 0.55, "{orientation:?}: roll share should have grown, got {after}");
        }
    }

    /// Press on the divider, drag by `delta`, and return the resulting
    /// `roll_fraction`. Three frames: egui needs the widget to exist before
    /// the press, and a drag only registers once the pointer has moved while
    /// held.
    fn drag_divider(
        rect: egui::Rect,
        orientation: SpectralOrientation,
        roll_fraction: f32,
        delta: egui::Vec2,
    ) -> f32 {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = orientation;
        state.spectrum_config.roll_fraction = roll_fraction;
        state.spectrum_config.show_roll = true;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let grab = axes(rect, orientation).at(0.5, 1.0 - roll_fraction);
        let frame = |events: Vec<egui::Event>, state: &mut SharedState| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run_ui(input, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                spectral_pane(&mut child, state, 100.0, 0);
            });
        };
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        frame(vec![egui::Event::PointerMoved(grab)], &mut state);
        frame(vec![egui::Event::PointerMoved(grab), press(grab, true)], &mut state);
        frame(vec![egui::Event::PointerMoved(grab + delta)], &mut state);
        frame(vec![press(grab + delta, false)], &mut state);
        state.spectrum_config.roll_fraction
    }

    /// A gridline label at the now edge of a wide (Left) pane sits just
    /// inside it and grows up-and-inward (LEFT_BOTTOM anchor).
    #[test]
    fn gridline_labels_sit_just_inside_the_now_edge() {
        let a = axes(WIDE, SpectralOrientation::Left);
        let (pos, align) = a.text_anchor(0.5, 0.0, 3.0, 2.0);
        assert_eq!(pos, egui::pos2(12.0, 67.0));
        assert_eq!(align, egui::Align2::LEFT_BOTTOM);
    }

    /// Whichever way the pane is turned, a label anchored just inside an
    /// edge grows inward rather than off the pane.
    #[test]
    fn label_anchors_grow_into_the_pane() {
        for orientation in EVERY_ORIENTATION {
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
            for orientation in EVERY_ORIENTATION {
                for roll_fraction in [0.0, 0.55, 1.0] {
                    let shapes = paint(rect, orientation, roll_fraction);
                    assert!(!shapes.is_empty(), "{orientation:?} drew nothing");
                }
            }
        }
    }

    /// The spectrogram heatmap is rebuilt and re-uploaded only when its inputs
    /// change; between the ~20 Hz FFT columns most frames just redraw the quad
    /// over the reused texture. Two frames with identical clock and history: the
    /// second finds a matching key and takes that fast path — and must draw
    /// exactly the quad the cold first frame built, since it reuses that build's
    /// geometry. A stale or mis-cached build would move the quad.
    #[test]
    fn a_cached_spectrogram_frame_matches_the_cold_build() {
        // The textured strip's per-vertex position + uv. The spectrogram is the
        // pane's only mesh (notes are paths, labels are text), so its geometry
        // is what these shapes carry — however many quads it is split into.
        fn quad(out: &egui::FullOutput) -> Vec<[f32; 4]> {
            let mut v = Vec::new();
            for c in &out.shapes {
                if let egui::Shape::Mesh(m) = &c.shape {
                    assert_eq!(m.indices.len(), m.vertices.len() / 4 * 6, "quads, please");
                    v.extend(m.vertices.iter().map(|x| [x.pos.x, x.pos.y, x.uv.x, x.uv.y]));
                }
            }
            v
        }

        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.show_spectrogram = true;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 79.0;
        state.spectrum_config.roll_seconds = 10.0;
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 0.4;
        for i in 0..40 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.1, &bins);
        }

        // ONE context across both frames, as in the live app: the cache hands
        // back a texture handle owned by this context.
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let now = 94.0;
        let mut frame = || {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(500.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                    spectral_pane(&mut child, &mut state, now, 0);
                },
            )
        };
        let cold = quad(&frame());
        assert!(!cold.is_empty(), "the spectrogram drew no textured quad to cache");
        let hit = quad(&frame());
        assert_eq!(cold, hit, "the cached frame drew a different quad than the cold build");
    }

    /// The strip reaches the now-line, but the newest column is older than that
    /// — half an analysis window, by construction — so its leading sliver has
    /// no data of its own and holds the newest column instead. Inside the live
    /// ring the texels past the newest one hold what they carried a lap ago (a
    /// column from a whole window back), so a `u` that ran on would paint that
    /// sliver with the far end of the window.
    ///
    /// Where `u` stops, the mesh SPLITS: a quad spanning the corner would
    /// interpolate it across itself, and since these are vertex UVs that
    /// rescales the whole image, once per slab as the corner crosses it. So the
    /// drawn strip is a flat leading quad (one `u` on all four corners) joined
    /// to the data quad at that same `u`.
    #[test]
    fn the_strip_holds_its_leading_sliver_instead_of_reading_round_the_ring() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.show_spectrogram = true;
        state.spectrum_config.roll_seconds = 2.0; // zoomed in: the sliver is widest
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
        for i in 0..100 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.02, &bins);
        }
        // The now-line, an analysis window's half-lag past the newest column.
        let now = 91.98 + 0.171;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(500.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                spectral_pane(&mut child, &mut state, now, 0);
            },
        );
        let mesh = out
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Mesh(m) => Some(m.clone()),
                _ => None,
            })
            .expect("the spectrogram drew no textured strip");

        assert_eq!(mesh.vertices.len(), 8, "two quads, split where `u` stops");
        let mut us: Vec<f32> = mesh.vertices.iter().map(|v| v.uv.x).collect();
        us.sort_by(f32::total_cmp);
        // Two values only — the corner, shared by the flat quad's four vertices
        // and the data quad's leading two, and the far end of the data.
        let (far, hold) = (us[0], us[7]);
        assert!(far < hold, "the data quad spans no time at all");
        assert_eq!(us.iter().filter(|u| **u == hold).count(), 6, "not one flat leading quad: {us:?}");
        assert_eq!(us.iter().filter(|u| **u == far).count(), 2, "the data quad bends: {us:?}");
    }

    /// The heatmap image is sized in DEVICE PIXELS, not points. It is stretched
    /// over the pane by the GPU, so sizing it in points builds it at the
    /// display's density divided by the scale factor and then upsamples — on a
    /// Retina screen, half the resolution in each axis, for a heatmap visibly
    /// softer than the pane around it. Same pane, twice the density, twice the
    /// rows.
    ///
    /// (Rows and not columns: the time axis picks its slab off `live_slab`'s
    /// ladder, so how much of a density increase reaches it depends on the span.)
    #[test]
    fn the_heatmap_image_is_built_at_device_pixels() {
        fn rows_at(ppp: f32) -> usize {
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.show_spectrogram = true;
            state.spectrum_config.roll_seconds = 10.0;
            let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
            bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 3] = 0.8;
            for i in 0..40 {
                state.spectrum.push_history(90.0 + f64::from(i) * 0.1, &bins);
            }
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            ctx.set_pixels_per_point(ppp);
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
            // Twice: `set_pixels_per_point` lands on the following frame.
            for _ in 0..2 {
                let _ = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                    |ui| {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                        spectral_pane(&mut child, &mut state, 94.0, 0);
                    },
                );
            }
            state.spectrum.spectrogram[0].tex.as_ref().expect("a heatmap was uploaded").size()[1]
        }

        let (one, two) = (rows_at(1.0), rows_at(2.0));
        assert!(one > 2, "no heatmap rows at 1x");
        // Exactly double, give or take the rounding of one pixel row.
        assert!(
            two.abs_diff(one * 2) <= 1,
            "{one} rows at 1x but {two} at 2x — the image is being sized in points",
        );
    }

    /// A sounding note is marked by its own ribbon crossing the now-line, so
    /// the roll has to be painted after the line rather than before it. Every
    /// separate mark drawn for the job sat wrong against a rounded ribbon end;
    /// the ribbon cannot.
    #[test]
    fn the_roll_paints_over_the_now_line() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 72.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 69,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                spectral_pane(&mut child, &mut state, 0.1, 0);
            },
        );
        // The now-line is the one hairline-colored segment clean across the
        // pitch axis; the roll is one paint callback (its notes are instanced
        // quads, not shapes) and must come after it — paint callbacks keep
        // their place in egui's draw order, which is what puts the roll over
        // the line and under the axis labels.
        let hairline = out.shapes.iter().position(|s| {
            matches!(&s.shape, egui::Shape::LineSegment { stroke, .. }
                if stroke.color == theme::hairline())
        });
        let note = out
            .shapes
            .iter()
            .rposition(|s| matches!(&s.shape, egui::Shape::Callback(_)));
        let (Some(hairline), Some(note)) = (hairline, note) else {
            panic!("expected both a now-line and a note ribbon in the frame");
        };
        assert!(note > hairline, "the note paints under the line it arrives at");
    }

    /// A note sounding where the visible lattice has no node is flagged by a
    /// band down the spectrum at its pitch — the lattice shows nothing for
    /// such a note by definition, so this pane is the only place you can learn
    /// one is playing. Put in the spectrum's territory rather than on the
    /// note, whose color is already saying which voice it is.
    #[test]
    fn an_off_lattice_note_gets_a_band_down_the_spectrum() {
        let bands = |tuning_offset: f32| {
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.low_midi = 55.0;
            state.spectrum_config.high_midi = 67.0;
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
            let out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                    spectral_pane(&mut child, &mut state, 0.05, 0);
                },
            );
            let want = theme::warning_text().gamma_multiply(0.3);
            out.shapes
                .into_iter()
                .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == want))
                .count()
        };
        assert_eq!(bands(0.0), 0, "a plain C has a node, so nothing to flag");
        assert_eq!(bands(0.5), 1, "half a semitone sharp has none");
    }

    /// The axis labels carry a rim, like the lattice's node names. What sits
    /// behind them is a picture — a bright spectrogram slab, the spectrum's
    /// own fill — so plain text has no contrast to rely on, and a label you
    /// can't read doesn't say which pitch a lane is.
    ///
    /// The rim is drawn from the glyph's own coverage now rather than by
    /// stamping the text, so what this can check is that every label is
    /// handed a rim color to draw it with.
    #[test]
    fn the_axis_labels_are_rimmed() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_fraction = 0.55;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                spectral_pane(&mut child, &mut state, 0.05, 0);
            },
        );
        // The labels leave the shape list as one paint callback; what is
        // checkable from here is that the pane emitted one at all, and the
        // glyphs' colors are checked where they are built (`crate::text`).
        assert!(
            out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Callback(_))),
            "the pane drew no label callback at all",
        );
    }

    /// The readout names its own unit, and switches to kHz where an analyzer
    /// axis does.
    #[test]
    fn the_hz_readout_carries_its_unit() {
        assert_eq!(hz_readout(69.0), "440 Hz", "A440, the one value worth checking by hand");
        assert_eq!(hz_readout(harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI), "20 Hz");
        assert_eq!(hz_readout(harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI), "20.0 kHz");
        // The switch is at 1000 Hz exactly, not somewhere near it.
        let khz = harmonigraph_core::spectrum::hz_to_midi(1000.0);
        assert_eq!(hz_readout(khz), "1.0 kHz");
        assert!(hz_readout(khz - 0.1).ends_with(" Hz"));
    }

    /// The settings pane, whose pitch-range bar derives rects from a PAIR of
    /// values — the shape of thing that folds to zero area and panics egui.
    /// Painted at both the widest and the narrowest range it allows.
    #[test]
    fn the_settings_pane_paints_at_either_extreme_of_the_pitch_range() {
        let axis =
            (harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI, harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI);
        for (low, high) in [axis, (40.5, 40.5 + crate::PITCH_RANGE_MIN_SPAN), (axis.0, axis.0)] {
            let mut state =
                SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
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
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.low_midi = low;
            state.spectrum_config.high_midi = high;
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(WIDE));
                    spectral_pane(&mut child, &mut state, 100.0, 0);
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
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = orientation;
        state.spectrum_config.roll_fraction = roll_fraction;
        state.spectrum_config.roll_seconds = 10.0;
        state.view.bloom_strength = 1.2; // exercise the note-glow passes
        // Exercise the spectrogram's mesh path in every orientation too, with
        // energy at both axis extremes (where cell clamping is most likely to
        // fold a quad to zero area — which egui panics on).
        state.spectrum_config.show_spectrogram = true;
        let mut spectrum_bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        spectrum_bins[0] = 1.0;
        spectrum_bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 0.5;
        spectrum_bins[harmonigraph_core::spectrum::SPECTRUM_BINS - 1] = 0.3;
        for i in 0..80 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.125, &spectrum_bins);
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

        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 500.0));
        let output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                spectral_pane(&mut child, &mut state, now, 0);
            },
        );
        output.shapes.into_iter().map(|s| s.shape).collect()
    }
}
