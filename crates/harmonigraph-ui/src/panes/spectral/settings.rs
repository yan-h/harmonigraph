//! The Analyzer tab: every setting the Spectral display carries, and the
//! readouts its bars are dragged against.

use crate::widgets::{button_row, choice_row, option_label, RangeBar, ValueBar};
use crate::SharedState;
use crate::panes::{edge_bar, section};

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

/// A duration as the roll's Span bar reads it out, carrying its own unit
/// rather than naming one in the label: the bar runs from a second to ten
/// minutes, and "300" under a fixed "(s)" is a number you have to divide
/// before it means anything. "5m 00s" is the same value already read.
///
/// Tenths under a minute, whole seconds above it. The scale is eased, so the
/// short spans get most of the travel and are where a tenth is a visible
/// step; a tenth of a second inside five minutes is not.
pub(super) fn span_readout(seconds: f32) -> String {
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
pub(crate) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
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
    // makes it a log-frequency zoom) and read out in Hz.
    RangeBar::new(
        &mut cfg.low_midi,
        &mut cfg.high_midi,
        harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI..=harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI,
        "Pitch range",
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
         zoom it around the pointer. (Dragging the other way zooms what that \
         part of the pane measures instead: the roll's Span over the roll, the \
         Level over the spectrum.)",
    );
    // No choice of what the markings say. They are the analyzer-standard
    // 1-2-5 frequency series, and were switchable to one at every C with
    // Bitwig octave numbers — which is what the note NAMES on the ribbons
    // already say, in the lattice's own spelling, at the pitch they are
    // sounding rather than at the nearest C below it.
    ValueBar::new(&mut cfg.marking_scale, crate::SCALE_BAR_RANGE, "Label size")
        .show(ui)
        .on_hover_text(
            "Size of the pane's own markings: the label at each frequency, and \
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
    RangeBar::new(
        &mut cfg.floor_db,
        &mut cfg.ceiling_db,
        crate::LEVEL_MIN_DB..=crate::LEVEL_MAX_DB,
        "Level",
    )
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
         scale.\n\nOr set the high end on the display itself: drag the \
         spectrum along the depth axis, away from its baseline to zoom in.",
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
        // Five signed numbers side by side, so `option_label` sets them in
        // monospace: a proportional face gives "0.0" and "-1.5" different
        // widths and leaves the row visibly uneven, where digits of one width
        // make it a scale.
        for step in crate::TILT_STEPS {
            ui.selectable_value(&mut cfg.tilt, step, option_label(&format!("{step:.1}")));
        }
    });

    ValueBar::new(&mut cfg.keyline, 0.0..=1.0, "Edge").show(ui).on_hover_text(
        "A light rim along the spectrum's profile. It sits over the \
         spectrogram, whose colors run from black to near-white, so where the \
         curve is the same brightness as the cell behind it the shape is lost \
         without one. 0 draws none.\n\nA note ribbon has its own edge — see \
         Outline, under Piano roll.",
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
    edge_bar(
        ui,
        (&mut cfg.roll_outline, &mut cfg.roll_outline_fade),
        crate::ROLL_OUTLINE_MAX,
        "Outline",
        {
            let fresh = crate::SpectrumConfig::default();
            (fresh.roll_outline, fresh.roll_outline_fade)
        },
        |v| format!("{v:.1}"),
    )
    .on_hover_text(
        "How far a dark surround stands off a note, in points, and how much of \
         that it spends fading out. It wraps every side, so a note is a bounded \
         object over the spectrogram rather than a ribbon dissolving into it — \
         and dark, because the spectrogram's palettes run to near-white and \
         that is the one color no cell of one can hide. 0 draws none.\n\nIn \
         points rather than semitones, so it is the same edge at every zoom; at \
         a wide zoom the ribbons are thinner than this and neighbours' outlines \
         reach over each other.\n\nThe bar is the outline itself, read outward \
         from the note: solid to the first handle, then fading, and gone by the \
         second. Drag between them to stand the note further off without \
         blurring its edge, the inner handle to soften it (together they close \
         for a hard edge), the outer one to reach further out from where it \
         already starts to soften.",
    );
    ui.checkbox(&mut cfg.note_names, "Note names").on_hover_text(
        "Write each note's name on its own ribbon, at the leading edge — so a \
         held note's name waits at the now-line and travels off with the note \
         when you let go. For reading the heatmap: a band of energy sits at \
         some height on an axis marked only in decades of hertz, and the \
         ribbon over that band is the same note, so naming the ribbon names \
         the band.\n\n\
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
