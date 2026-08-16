//! The Display tab's Analyzer page: every setting the Spectral display
//! carries, and the readouts its bars are dragged against. The heatmap's
//! colors are the one thing dialled elsewhere — they are a color table, and
//! both of those are on the Colors page ([`super::super::color`]).

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
/// the UI state). The Display tab's Analyzer page.
pub(crate) fn spectrum_settings_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    use crate::{SpectralOrientation, SpectrumTapers, SpectrumWindow};

    // What the page reaches, said once at the top rather than repeated on the
    // sections: every setting here is the analyzer's, and the Spiral is the
    // same analyzed frame wound onto a disc rather than a second one.
    ui.weak("Drives the Analyzer pane — and the Spiral, which reads the same analyzer.");

    // ---- Plot -----------------------------------------------------------
    // Which way the plot runs, and how much of the pitch axis it shows. Time's
    // own extent is the roll's Span, and lives with the roll — it is the one
    // axis setting that means nothing without the layer it measures; the Level
    // range lives with the Spectrum, which is not the only layer reading it.
    //
    // "Plot" rather than "Axes" for exactly that reason: two of the three axis
    // extents are deliberately elsewhere, and a heading promising all three is
    // one a reader learns not to trust.
    //
    // A plain heading rather than `section`: what stands above it is the page's
    // own scope line rather than a section, so a rule between the two would cut
    // the page off from its first heading instead of separating two groups.
    ui.heading("Plot");
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
        "The slice of the spectrum on show — and the Spiral's window. The scale \
         is logarithmic: an octave is the same width everywhere. Drag either \
         end, between them to slide, double-click for the full axis — or drag \
         and scroll on the picture itself.",
    );
    // No choice of what the markings say. They are the analyzer-standard
    // 1-2-5 frequency series, and were switchable to one at every C with
    // Bitwig octave numbers — which is what the note NAMES on the ribbons
    // already say, in the lattice's own spelling, at the pitch they are
    // sounding rather than at the nearest C below it.
    ValueBar::new(&mut cfg.marking_scale, crate::SCALE_BAR_RANGE, "Marking size")
        .show(ui)
        .on_hover_text(
            "Size of the axis labels and the pointer readout. Fixed against \
             zoom — the axis doesn't change size when you zoom it.",
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
    // Beside the window rather than folded into it: the window trades time
    // against pitch, this trades cost and contrast against the estimate's own
    // noise, and no single row of buttons can name both.
    button_row(ui, |ui| {
        ui.label("Tapers");
        for (tapers, label) in [
            (SpectrumTapers::One, "1"),
            (SpectrumTapers::Three, "3"),
            (SpectrumTapers::Five, "5"),
        ] {
            ui.selectable_value(&mut cfg.tapers, tapers, label).on_hover_text(match tapers {
                SpectrumTapers::One => "one window: the sharpest picture, and the speckliest",
                SpectrumTapers::Three => "three looks at the same audio: about half the \
                     speckle, ~4.7 dB less room between a partial and the haze",
                SpectrumTapers::Five => "five looks: steadier again, and coarser at the \
                     bottom of the axis",
            });
        }
    });

    // Both ends of the height scale on one control, like the pitch range: the
    // window on the spectrum's dynamics rather than just where it bottoms out.
    RangeBar::new(
        &mut cfg.floor_db,
        &mut cfg.ceiling_db,
        crate::LEVEL_MIN_DB..=crate::LEVEL_MAX_DB,
        "Level range",
    )
    .min_span(crate::LEVEL_RANGE_MIN_SPAN)
    .display(db_readout)
    .show(ui)
    .on_hover_text(
        "The slice of the level scale on show: the low end reads as silence, \
         the high end as full height and the brightest heatmap cell. Pull the \
         top down to lift quiet material into the picture. Double-click for the \
         full scale.",
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

    ValueBar::new(&mut cfg.keyline, 0.0..=1.0, "Outline").show(ui).on_hover_text(
        "A light rim along the spectrum's profile, so its shape holds over the \
         heatmap. 0 draws none. Each note ribbon has its own — see Outline under \
         Piano roll.",
    );

    // ---- Piano roll -----------------------------------------------------
    section(ui, "Piano roll");
    ui.checkbox(&mut cfg.show_roll, "Note history").on_hover_text(
        "Draw incoming MIDI as a scrolling roll over the same pitch axis. \
         Ribbons wear the note colors from the Colors page, so a note matches \
         the lattice.",
    );
    ValueBar::new(&mut cfg.roll_seconds, crate::ROLL_SECONDS_MIN..=crate::ROLL_SECONDS_MAX, "Span")
        .eased(true)
        .decimals(1)
        .display(span_readout)
        .show(ui)
        .on_hover_text(
            "Seconds of history the roll shows end to end, up to 10 minutes; \
             short spans get most of the drag. Or drag the roll along its time \
             axis — away from the now-line zooms in.",
        );
    ValueBar::new(&mut cfg.roll_thickness, 0.2..=2.0, "Note width")
        .show(ui)
        .on_hover_text(
            "Ribbon width, in semitones of the pitch axis — a note is as wide \
             as the interval it would cover, at any zoom.",
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
        "A dark surround that stands each ribbon off the heatmap — in points, \
         so it is the same edge at every zoom. Solid to the inner handle, gone \
         by the outer. 0 draws none.",
    );
    edge_bar(
        ui,
        (&mut cfg.roll_lead, &mut cfg.roll_lead_fade),
        crate::ROLL_LEAD_MAX,
        "Lead",
        {
            let fresh = crate::SpectrumConfig::default();
            (fresh.roll_lead, fresh.roll_lead_fade)
        },
        |v| format!("{:.1}%", v * 100.0),
    )
    .on_hover_text(
        "How far a sounding note reaches past the now-line into the spectrum, \
         as a share of it — so which notes are down reads off the picture. \
         Solid to the inner handle, gone by the outer. 0 stops notes square on \
         the line.",
    );
    ValueBar::new(&mut cfg.roll_lead_release, 0.0..=crate::ROLL_LEAD_RELEASE_MAX, "Lead release")
        .decimals(2)
        .display(|v| format!("{v:.2} s"))
        .show(ui)
        .on_hover_text(
            "Seconds the lead takes to fade once the key is released. 0 cuts it \
             the instant the note stops.",
        );
    ui.checkbox(&mut cfg.note_names, "Note names").on_hover_text(
        "Name each ribbon, in the lattice's own spelling — a just third reads \
         E- rather than E plus cents. Where repeats crowd, the first keeps its \
         name and the next waits for room. Needs Note history.",
    );
    ui.add_enabled_ui(cfg.note_names && cfg.show_roll, |ui| {
        ui.checkbox(&mut cfg.note_names_travel, "Name the far end").on_hover_text(
            "Write names on the ribbon's other end — anchored to the note's \
             onset rather than its newest edge. Held notes then travel with the \
             picture instead of waiting at the now-line. Flips with the Now-line \
             setting.",
        );
        ValueBar::new(&mut cfg.note_name_scale, crate::SCALE_BAR_RANGE, "Name size")
            .show(ui)
            .on_hover_text(
                "Size of the ribbon names. They already grow as the pitch range \
                 narrows; this sets their size relative to the ribbon.",
            );
    });

    // ---- Spectrogram ----------------------------------------------------
    section(ui, "Spectrogram");
    ui.checkbox(&mut cfg.show_spectrogram, "Heatmap").on_hover_text(
        "A frequency-vs-time heatmap of the audio, behind the roll on the same \
         time axis. Reads the Spectrum's Level range and Tilt. Turn Note history \
         off to see it alone.",
    );
    // Where its colors are, rather than a second copy of the bars: the heatmap
    // gradient is one of the two color tables, and both are dialled on the
    // Colors page (see [`crate::panes::color`]).
    ui.weak("Its colors are set on the Colors page.");
    // The checkbox and that pointer are the whole of the section. An opacity, a
    // contrast curve and a private level range would each go here and each is
    // deliberately absent — see
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
