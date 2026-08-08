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

/// The heatmap's level->color gradient on the same preview and three bars the
/// Nodes tab dials the lattice's pitch gradient with, over a row of presets.
///
/// Three bars and not six: the group is the gradient itself across the top, and
/// under it the arc on the spectrum bar, the brightness pair on one of its own
/// and the chroma pair on another, each a picture of what its numbers COMPOSE —
/// see `panes::nodes::spectrum_group`, which is the same set over the same type
/// and says why a six-number gradient costs three rows and a preview rather
/// than six rows.
///
/// **What differs is the axis, and only the readouts show it.** There the
/// range is pitch, so a bar's two ends are the lowest and highest notes; here
/// it is the analyzer's Level, so they are silence and a full bucket. The bars
/// themselves cannot tell — they are handed a [`harmonigraph_scene::Gradient`]
/// and a home to reset to, and nothing in either names an axis — so what says
/// which is the tooltip, and the tooltips below are written for level rather
/// than shared with the Nodes tab's.
///
/// **The presets come first**, ahead of the preview the Nodes tab opens its
/// group with, and deliberate: a heatmap palette is a thing people pick by name
/// before they dial it, and the four names are what this pane offered before it
/// offered any knobs at all. They write the bars below and are not a mode — see
/// [`crate::SpectrogramPreset`]. The preview then sits between the names and
/// the bars, which is where both of them are read against it.
fn spectrogram_gradient_group(ui: &mut egui::Ui, cfg: &mut crate::SpectrumConfig) {
    use crate::widgets::{GradientPreview, SpectrumBar, SpreadBar};
    use crate::SpectrogramPreset;

    // The gradient a double-click on any of the three goes home to. The fresh
    // heatmap's, NOT the lattice's, which is what the bars assume when a caller
    // names none: a Spectral pane resetting onto the Nodes tab's arc would land
    // on a picture this pane has never opened on, and the bars carry no text
    // entry to dial it back with.
    let home = crate::SpectrumConfig::default().spectrogram_gradient;
    button_row(ui, |ui| {
        ui.label("Palette").on_hover_text(
            "Four looks the heatmap opens on, each written straight into the \
             bars below — press one and then dial it. Nothing remembers which \
             was pressed, a look being six numbers rather than a mode: once a \
             bar has moved, the picture is the picture.",
        );
        for preset in SpectrogramPreset::ALL {
            if ui
                .button(preset.label())
                .on_hover_text(preset.hint())
                .clicked()
            {
                cfg.spectrogram_gradient = preset.gradient();
            }
        }
    });
    // The row first, the colors last — see [`GradientPreview`].
    let preview = GradientPreview::reserve(ui);
    SpectrumBar::new(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "The level->color spectrum: how far round the color circle the level \
         range walks, out of the whole turn the bar stands for. The hues it \
         takes fill from the left, silence first; the ones it does not are \
         dimmed. The track is hue alone — the brightness and chroma bars below \
         move the picture above, not the circle, which is what leaves the hues \
         showing at Mono, where the arc itself has no width. Drag the handle \
         to widen or narrow it, drag the track to turn the circle under it, \
         double-click to reset. The button past the right end runs the whole \
         thing the other way round the circle.",
    );
    SpreadBar::brightness(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "The stretch of brightness the level range spends, in CIELab L*: the \
         two numbers are silence and a full bucket, in that order. Silence \
         wants 0 — the heatmap is laid on a black bed, so a lifted floor draws \
         the plane's own edge where the history runs out, which is occasionally \
         what you want to see. Drag either end to move it, drag between them to \
         slide the whole stretch, drag one end past the other to draw loud dark \
         on a pale field, double-click to reset.",
    );
    SpreadBar::chroma(&mut cfg.spectrogram_gradient).home(home).show(ui).on_hover_text(
        "The stretch of color the level range spends, each end as a share of \
         the most that cell's own brightness and hue can hold — 100% is as \
         vivid as the screen goes without distorting the color, 0 is grey. The \
         two numbers are silence and a full bucket, in that order. Closing them \
         together gives every level the same share of the color available to \
         it, which is what the three colored presets do; taking both to 0 is \
         Mono. Near the top the ramp rides the gamut's own boundary, whose \
         corners between the screen's primaries show up as steps in an \
         otherwise smooth sweep.",
    );
    preview.show(ui, &cfg.spectrogram_gradient).on_hover_text(
        "The gradient itself, silence on the left and a full bucket on the \
         right: every one of the six numbers the bars below carry, composed \
         into the colors the heatmap draws with. A picture rather than a \
         control — the three bars under it are what move it.",
    );
}

/// Settings for the Spectral pane's display and analyzer (persisted with
/// the UI state).
pub(crate) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{SpectralOrientation, SpectrumWindow};

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
         and dark, because every heatmap PRESET starts at black and climbs, so \
         black is what its cells are furthest from. (Take the Brightness bar's \
         low end up and that stops being true; the ribbon still carries its own \
         color.) 0 draws none.\n\nIn points rather than semitones, so it is the \
         same edge at every zoom; at a wide zoom the ribbons are thinner than \
         this and neighbours' outlines reach over each other.\n\nThe bar is the \
         outline itself, read outward from the note: solid to the first handle, \
         then fading, and gone by the second. Drag between them to stand the \
         note further off without blurring its edge, the inner handle to soften \
         it (together they close for a hard edge), the outer one to reach \
         further out from where it already starts to soften.",
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

    // ---- Spectrogram ----------------------------------------------------
    section(ui, "Spectrogram");
    ui.checkbox(&mut cfg.show_spectrogram, "Heatmap").on_hover_text(
        "A frequency-vs-time heatmap of the audio, drawn in the roll's \
         region on the same time axis — so each column of energy lines up \
         with the notes that made it. Reads the Spectrum's Level and Tilt \
         for intensity. Turn Note history off to see the heatmap alone.",
    );
    spectrogram_gradient_group(ui, cfg);
    // The gradient is the whole of it. An opacity, a contrast curve and a
    // private level range would each go here and each is deliberately absent —
    // see
    // [`spectrogram_gradient`](crate::SpectrumConfig::spectrogram_gradient) for
    // why the neutral setting is the only one worth having.

    // One button for the pane's two accumulations rather than one under each
    // section, because they are one picture: the ribbons and the heatmap are
    // drawn in the same region on the same time axis, and what makes either
    // worth clearing is stale history in that region — which is both of them
    // at once, whatever put it there. A button per section clears one layer
    // and leaves the other's leftovers in the same rectangle.
    //
    // At the pane's foot rather than in Piano roll or Spectrogram, since it
    // belongs to neither: under a section heading a button reads as that
    // section's, and this one names what it takes so it can stand after both.
    // The lattice trail is a different pane's and stays there; the Video
    // pane's "Clear everything" is what takes all three at once.
    button_row(ui, |ui| {
        if ui
            .button("Clear roll and spectrogram")
            .on_hover_text(
                "Forget the played-note timeline and the accumulated spectral \
                 history, emptying the pane of everything it has drawn over \
                 time. A note held across this is gone from the roll until it \
                 is played again.",
            )
            .clicked()
        {
            state.tracker.clear_roll();
            state.spectrum.clear_history();
        }
    });
}
