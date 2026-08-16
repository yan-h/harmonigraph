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
    use crate::{SpectralOrientation, SpectrumWindow};

    // What the page reaches, said once at the top rather than repeated on the
    // sections: every setting here is the analyzer's, and the Spiral is the
    // same analyzed frame wound onto a disc rather than a second one.
    ui.weak("Drives the Analyzer pane — and the Spiral, which reads the same analyzer.");

    // ---- Plot -----------------------------------------------------------
    // Which way the plot runs, and how much of the pitch axis it shows. Time's
    // own extent is the roll's Span, and lives with the roll — it is the one
    // axis setting that means nothing without the layer it measures; the
    // Level's lives with the Spectrum, which is not the only layer reading it.
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
    ValueBar::new(&mut cfg.marking_scale, crate::SCALE_BAR_RANGE, "Marking size")
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
         the peak it is making.\n\nA ribbon takes its color from the pitch \
         gradient, which is the Colors page's — the same colors the \
         lattice draws a sounding note in, so a note is the same color in both \
         pictures. The heatmap's own gradient is on that page too, and is a \
         different one.",
    );
    ValueBar::new(&mut cfg.roll_seconds, crate::ROLL_SECONDS_MIN..=crate::ROLL_SECONDS_MAX, "Span")
        .eased(true)
        .decimals(1)
        .display(span_readout)
        .show(ui)
        .on_hover_text(
            "Seconds of history the roll spans end to end, up to 10 minutes. \
             The scale is logarithmic, so the short spans get most of the \
             travel. The spectrogram fills the whole of it, as far back as it \
             has been listening -- what a long span costs is grain, not reach: \
             its detail is merged into wider time slabs, and zooming in is what \
             asks for it back.\n\nOr set it on the \
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
        "How far a SOUNDING note carries past the now-line and into the \
         spectrum, and how much of that it spends fading out. A sounding note's \
         ribbon ends on the line and the peak it is making stands on the other \
         side of it, so this is the ribbon reaching across the join — which \
         notes are down reads off the picture instead of off the ends of the \
         ribbons. 0 stops every note square on the line.\n\n\
         As a share of the ANALYZER, not a length: 10% reaches a tenth of the \
         way across the spectrum, so the same setting draws the same picture on \
         a window of any size, and dragging the divider over the curve shortens \
         the tongues with it rather than letting them cover what is left. (The \
         Outline bar above is in points instead, and deliberately — an edge \
         wants to be the same weight everywhere, where a reach into another \
         picture wants to be the same share of it.)\n\n\
         The bar is the lead itself, read outward from the now-line: solid to \
         the first handle, then fading, and gone by the second. Close them for \
         a tongue that ends square, open the inner one to the outer for one \
         that fades the whole way across.\n\n\
         The divider still draws over it, unbroken — the ribbon passes under \
         the line rather than eating it, which is what keeps the boundary \
         followable while a chord is reaching over it.",
    );
    ValueBar::new(&mut cfg.roll_lead_release, 0.0..=crate::ROLL_LEAD_RELEASE_MAX, "Lead release")
        .decimals(2)
        .display(|v| format!("{v:.2} s"))
        .show(ui)
        .on_hover_text(
            "How long a note's lead takes to fade away once you let go of the \
             key. 0 takes it the instant the note stops.\n\nThe lead is a claim \
             about the present — this is sounding — so it has to go when the \
             note does, and going in one frame is a tongue of ink vanishing \
             rather than ending. Given a fifth of a second it dims instead, \
             riding the ribbon's own end as that scrolls away from the line: \
             the release becomes something the picture does rather than \
             something that happens to it.\n\nIn seconds, so it is the same \
             gesture at every Span. At a long one the ribbon barely moves \
             while it happens and the tongue simply dissolves on the spot.",
        );
    ui.checkbox(&mut cfg.note_names, "Note names").on_hover_text(
        "Write each note's name on its own ribbon, at the end of it you read \
         first — the left of the note where time runs across the pane, its \
         top where time runs down — with the note running away under the rest \
         of the name. For reading the heatmap: a band of energy sits at some \
         height on an axis marked only in decades of hertz, and the ribbon \
         over that band is the same note, so naming the ribbon names the \
         band.\n\n\
         Names are the lattice's own, in its own hand: the node's spelling \
         with its accidental and comma mark, so a just third reads E- rather \
         than as an E and a cents offset.\n\nWhere repeats of a note come too \
         fast to name each one, the first keeps its name and the next waits \
         for clear room. A note you are HOLDING is named whatever the crowd, \
         but only where its name is the one waiting at the now-line — with \
         the spectrum on the right or along the bottom a held note takes its \
         turn like any other, its name being pinned to the take instead. \
         Needs Note history on: a name labels a ribbon.",
    );
    ui.add_enabled_ui(cfg.note_names && cfg.show_roll, |ui| {
        ui.checkbox(&mut cfg.note_names_travel, "Name the far end").on_hover_text(
            "Write the name on the other end of the ribbon — the end you read \
             SECOND — instead.\n\nThese are the ribbon's two ends, so this \
             moves every name on the roll and not only the ones you are \
             holding: a played note's name sits at the head of its ribbon or \
             at its tail.\n\nWhich musical end reads first is the \
             orientation's, so this flips with it. With the spectrum on the \
             LEFT or on TOP the name is on the note's leading edge: a held \
             note's name stays where you can read it while you play, and then \
             starts moving at the release — a movement nothing in the music \
             made, and a drone held longer than the Span never scrolls at all. \
             With the spectrum on the RIGHT or along the BOTTOM it is on the \
             onset, pinned to a moment of the take: it travels with the picture \
             from the first frame of the note and the release changes nothing \
             about it, at the price that a note held longer than the Span \
             carries its name off the far edge with its onset and scrolls the \
             rest of its ribbon unnamed, and that a note younger than its own \
             name wears it over the spectrum until its ribbon is long enough to \
             hold it.\n\nEither way a name keeps the same small gap from the \
             end it is written on: it leaves the picture when that end does, \
             rather than waiting on the edge while its own note scrolls out \
             from under it. The one thing that holds a name off that gap is \
             the pane's outer edge, and only with the Roll/Analyzer split \
             dragged so far that a young note's name has no analyzer left to \
             lie over.\n\nSo this is \
             the switch between those \
             two, whichever way round the pane has them.\n\nThe offline \
             render's whole-song layout is anchored at the onset either way: \
             it lays the take out as a still, and the onset is the one end of \
             a note that stands in one place there.",
        );
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
