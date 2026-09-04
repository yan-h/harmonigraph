//! Lattice note layers, their sizes and timing. Lighting is on its own page.

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, OctaveStrip, StackBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    Pulse, SpectralReading, ViewConfig, GAP_MAX, MARK_DELAY_MAX, MIN_EXTRA_SIZE, PITCH_CEIL,
    PITCH_FLOOR, SPECTRAL_BALLISTICS_MAX, SPECTRAL_GATE_MAX, SPECTRAL_GATE_MIN,
    SPECTRAL_HYSTERESIS_MAX, SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN, SPECTRAL_WIDTH_MAX,
    SPECTRAL_WIDTH_MIN,
};

/// Sizes and timing first, then the audio and MIDI layers and their accents.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    note_section(ui, &mut state.view, params);
    audio_section(ui, &mut state.view);
    octaves_section(ui, &mut state.view);
    melody_bass_section(ui, &mut state.view);
    shimmer_section(ui, &mut state.view);
}

/// Octaves: which octaves of the pitch class are sounding, shown as arcs of a
/// pitch axis that runs once round the node. Independent of the audio ring.
fn octaves_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "MIDI octave ring");
    // Octaves, Center and the fringe are the axis; how thick the ring they are
    // drawn on is, and where it sits, is the Layers bar up in Note — the
    // middle of its three handles, named MIDI there. The bar names layers by where each
    // one's reading comes FROM, which is what tells the two middle rings apart:
    // the analyzer's spectrum on the inner one, the played notes on this one.
    // This heading names the pitch axis drawn on it instead, that being what
    // the rows below set. How wide the cut between two indicators is belongs to
    // neither: it is the Octave gap, up in Note with the other padding.
    //
    // COUNTS and a CENTER rather than a pitch range: a slice is always exactly
    // one octave, so an indicator can never stand for less pitch than it
    // names — which a continuous window cannot promise, its two ends falling
    // wherever they like between two of a node's octaves and cutting the
    // indicators there short. Which register the wheel is about is the Center;
    // how much keyboard reaches round it is the two counts.
    //
    // One strip rather than a bar each, because the two are not independent:
    // they share the eleven-slice budget, and the thing being traded — how
    // much of the ring each octave keeps — is what the strip draws.
    OctaveStrip::new(
        &mut view.octave_count,
        &mut view.octave_extras,
        view.octave_extra_size,
        view.octave_extra_blend,
    )
    .show(ui)
    .on_hover_text(
        "Octaves shown around each node. \
                 Drag between the handles for full-size octaves, outside for smaller outer octaves. \
                 Notes beyond the range use the end slices.",
    );
    // Whole semitones, because that is the step the wheel can act on and what
    // the readout can name.
    ValueBar::new(&mut view.octave_center, PITCH_FLOOR..=PITCH_CEIL, "Center pitch")
        .integer()
        .unit(1.0, " MIDI")
        .display(|midi| format!("{} / {midi:.0} MIDI", super::pitch_readout(midi)))
        .show(ui)
        .on_hover_text(
            "Pitch at the top of every octave ring. Each node shows its own octaves nearest this pitch. Type a MIDI note number to set it.",
        );
    // The fringe is two bars rather than a list of named curves: the size is
    // the only thing that sets the outermost extra, the blend says how the
    // ones inside it climb toward the full-size octaves, and a wheel with no
    // extras is even rather than a mode beside them.
    //
    // With no extras there is no second tier, so neither bar has anything to
    // say. Not the whole of when the strip above is one flat row, though: a
    // fringe at size 1 is a second tier the same width as the first, and the
    // size bar is live there to drag back off it.
    ui.add_enabled_ui(view.octave_extras > 0, |ui| {
        ValueBar::new(&mut view.octave_extra_size, MIN_EXTRA_SIZE..=1.0, "Outer octave scale")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Width of each outer octave relative to an equal slice. 1× makes all slices equal.",
            );
    });
    // Inert with one extra a side, where a ramp has nothing to rise between:
    // the outermost extra is pinned by the size and the full-size octaves take
    // the rest, so there is no slice in between for a step to land on. Two is
    // the first fringe the bar can move
    // (`the_blend_is_inert_below_two_extras`). Equally inert at size 1, where
    // both tiers are already the same width. The blend can only say how a
    // fringe falls away, never whether there is one.
    ui.add_enabled_ui(view.octave_extras > 1 && view.octave_extra_size < 1.0, |ui| {
        ValueBar::new(&mut view.octave_extra_blend, 0.0..=1.0, "Outer octave taper")
            .percent()
            .show(ui)
            .on_hover_text(
                "Size transition from outer to inner octaves. \
                 0% keeps outer slices equally small; \
                 100% widens them gradually toward the center.",
            );
    });
    // The padding between one indicator and the next is NOT here: it is the
    // Octave gap, up in Note beside the Ring gap. The two paddings are one
    // question asked on a node's two axes — how far apart do its pieces read —
    // and a person dialling one is looking at the other, which a heading
    // between them costs. What this section keeps is the axis alone: how the
    // turn is shared out, rather than how wide the cut between two shares is.
    //
    // No size bar under these, and no on/off either: both are the Layers bar's,
    // where 0 is this layer's off position as it is on every other. The band is
    // one WIDTH there rather than a pair of radii, because where it sits is the
    // stack's answer — a gap out from the audio ring, or the stack's own start
    // where that ring is off — so the only thing left to say about it is how
    // thick it is.
    //
    // No solidity control and no Backdrop switch: the glyphs are always the crisp
    // classic shapes, and the silent octaves always stand in behind the
    // sounding ones — that backdrop is what completes the ring, so a lone
    // octave still reads as a whole note. How BRIGHT it stands is the At rest
    // section's Ground bar at the foot of the page, which is not this layer's
    // to own: the audio ring reads its own silence in that same grey.
    //
    // No Shimmer row either: the glyphs are what says which octaves sound, and
    // a sheet laid over that reading costs it — so the sweep belongs to the
    // marks, which carry no such reading. What reaches this layer is the mark
    // sheet crossing the one slice each mark extends, from the row in
    // Melody/bass below.
}

/// Melody / bass: mark the outer held notes so a chord's top and bottom line
/// read at a glance.
fn melody_bass_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Melody and bass marks");
    // Two boxes, not a four-way row: the marks are independent, they are
    // told apart by which slice each one extends rather than by hue, and a
    // note that is at once the highest and the lowest -- a lone held note, or
    // a chord whose top and bottom share a pitch class -- is one slice
    // extended once.
    //
    // Side by side, because they are that pair: two ends of one idea, both
    // named in the heading above them, and short enough that a column of two
    // spends a row on saying nothing. A `button_row` rather than a bare
    // `horizontal` so a narrow pane wraps them instead of running Bass off
    // the edge.
    button_row(ui, |ui| {
        ui.checkbox(&mut view.mark_melody, "Mark highest note")
            .on_hover_text("Extend the highest held note's octave slice past the band");
        ui.checkbox(&mut view.mark_bass, "Mark lowest note")
            .on_hover_text("Extend the lowest held note's octave slice past the band");
    });
    // A mark is the marked octave's own slice continued outward: it stands off
    // the band by the Ring gap, as every layer of the stack stands off the one
    // inside it, and its SIDES are cut by the Octave gap -- the same padding
    // one sector stands off the next, so it reads as that indicator's own piece
    // rather than as a ring around everything.
    // The Delay is about a mark that is DRAWN — when it arrives — so it is
    // gated on there being one: an end has to be marked AND the strip's depth
    // (the Layers bar's outermost handle) has to leave it something to draw
    // with, where `mark_extension` returns no coverage at 0 — `marks_draw`.
    ui.add_enabled_ui(view.marks_draw(), |ui| {
        // How long an end has to be HELD before its mark answers. Here
        // rather than in with the note-wide settings at the head of the
        // pane, because it is about these two marks alone: the octave
        // sectors they continue answer immediately whatever this says.
        //
        // A linear bar gives practical delays (a sixteenth note at 120 bpm
        // lasts 125 ms) enough room without easing. Display whole milliseconds;
        // the stored value and range stay in seconds.
        ValueBar::new(&mut view.mark_delay, 0.0..=MARK_DELAY_MAX, "Mark delay")
            .unit(1000.0, " ms")
            .decimals(0)
            .show(ui)
            .on_hover_text(
                "Time the highest or lowest note must stay in place before its mark appears. \
                 Increase to avoid flicker during fast passages. \
                 0 ms marks immediately.",
            );
    });
}

/// The patterns the shimmer's sheet can be laid in, for the Shimmer row.
///
/// A table beside the row rather than four arms written into it: a pattern is a
/// shape the light takes, and each one's description is a sentence about that
/// shape rather than about the lattice, so it belongs next to the others it is
/// told apart from.
const SHIMMER_PATTERNS: &[(Pulse, &str, &str)] = &[
    (Pulse::Off, "Off", "Steady — no sweep"),
    (Pulse::Bands, "Bands", "Diagonal bands of light moving across the melody and bass marks."),
    (Pulse::Checker, "Checker", "A moving checkerboard of light on the melody and bass marks."),
    (Pulse::Hex, "Hex", "A moving honeycomb of light on the melody and bass marks."),
];

/// Shimmer: the shape the sheet crossing the lattice takes, and how it is
/// sized and paced. The whole feature, under the one heading that names it.
///
/// The pattern is the first row and the four bars follow it, because the row
/// says WHETHER there is a sweep and the bars only say what it looks like. It
/// sits after Melody/bass because a mark's own strip rides the same sheet,
/// but the pattern reaches every octave slice a note lights whether or not
/// either mark is switched on.
///
/// The bars gate on [`Pulse::sweeps`] alone: with the pattern Off they have
/// nothing to move. The pattern row itself is always draggable — Off is
/// where a session leaves it when nothing should shimmer, not a state the
/// pane has to protect the row from reaching.
fn shimmer_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Mark shimmer");
    // Off is its own option rather than a checkbox beside the row: one row
    // says both whether the lattice shimmers and how.
    choice_row(ui, "Pattern", &mut view.pulse_marks, SHIMMER_PATTERNS);
    ui.add_enabled_ui(view.pulse_marks.sweeps(), |ui| {
        ValueBar::new(&mut view.shimmer_speed, 0.0..=6.0, "Travel speed")
        .unit(1.0, " steps/s").show(ui).on_hover_text(
            "Pattern movement in lattice steps per second. 0 freezes the pattern.",
        );
        // Eased, because the range is three orders wide and the useful
        // settings are not spread evenly over it: the tight end is a
        // different picture every few hundredths (0.05 to 0.1 halves the
        // periods on a node), where the wide end changes little between 8
        // and 15. Geometric travel gives each end the same share of the bar.
        ValueBar::new(&mut view.shimmer_width, 0.05..=15.0, "Pattern spacing")
        .unit(1.0, " steps")
            .eased(true)
            .show(ui)
            .on_hover_text(
                "Distance between bright peaks, in lattice steps. \
                 Larger spacing makes broad sweeps; smaller spacing makes a fine texture.",
            );
        ValueBar::new(&mut view.shimmer_intensity, 0.0..=2.0, "Contrast")
        .unit(1.0, "×").show(ui).on_hover_text(
            "Brightness contrast between peaks and troughs. 0 removes shimmer; 1× is the reference contrast.",
        );
        ValueBar::new(&mut view.shimmer_softness, 0.0..=1.0, "Edge softness")
        .percent().show(ui).on_hover_text(
            "Transition from a bright peak to a dark trough. 0% gives narrow, hard bands; 100% gives a smooth wave.",
        );
    });
}

/// Audio ring: what the ring inside the octave band measures — one reading of
/// the analyzer's spectrum, or none.
///
/// First of the layers, which is where it sits in the stack: reaching the
/// node's own centre, a Ring gap in from the octave Band below. It is the one
/// section here that says what a layer MEASURES where the rest only size and
/// colour what is already there, and the name carries the "ring" so the heading
/// says which layer that is.
///
/// One choice row and not two boxes, because there is one indicator here and
/// two ways to fill it: both readings answer "what is sounding at this node",
/// both draw in the same annulus in the same colours, and neither touches the
/// MIDI picture. Two boxes would have to say what BOTH ticked means, and the
/// only honest answer — one drawn over the other in the same ring — is a
/// picture nobody can read.
///
/// Each reading's own setting sits under the row, greyed when the other is
/// chosen: Tolerance is the fold's kernel and Zoom is the spectrum's window,
/// and neither means anything to the other. Both are shown either way rather than
/// swapped in and out, so the section keeps its height and the bars keep their
/// place as the row is clicked along.
fn audio_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Audio ring");
    if !view.spectral_ring_draws() {
        ui.weak("Use the Audio handle in Note layers above to give the ring a width.");
    }
    // "Ring display" and not "Ring", though the ring is what it fills: what this row
    // picks is which of two measurements the ring carries, which is the word the
    // rest of the audio channel uses for it, and a row named for the layer would
    // read as the layer's own switch when it is nothing of the kind.
    //
    // No Off among the readings, and the Layers bar is why: a width of 0 turns
    // the ring off, the way it turns the band and the marks off. An
    // Off here would be a second switch for this one layer, in a place no other
    // layer keeps one, and the two would then have to be read together to know
    // whether there is a ring.
    //
    // Grayed with the ring off rather than hidden, so the section keeps its
    // height and its rows keep their place as the handle is dragged to nothing.
    ui.add_enabled_ui(view.spectral_ring_draws(), |ui| {
        choice_row(
            ui,
            "Ring display",
            &mut view.spectral_reading,
            &[
                (
                    SpectralReading::Fold,
                    "Octave levels",
                    "One audio level per octave slice. Useful for seeing which harmonics are present across many nodes.",
                ),
                (
                    SpectralReading::Spectrum,
                    "Spectrum",
                    "The detailed spectrum within each octave slice. Useful for inspecting detuning on one node at close zoom.",
                ),
            ],
        );
    });
    // WHICH NODES wear the ring, where the Layers bar up in Note is how thick
    // it is: a node whose loudest wedge does not reach this level draws no ring
    // at all. Both readings, so it sits above the pair of bars that are each one
    // reading's own — it is a question about the layer rather than about a
    // measurement.
    //
    // It asks about the nodes NOBODY IS PLAYING, and that is worth knowing
    // while dragging it: a node the keys have lit keeps its ring for as long as
    // the note lasts whatever this says, and a ring comes and goes on the Note
    // section's Fade rather than at the instant the level crosses. Both are in
    // the hover text for the same reason — a bar that looks inert on the node
    // you are watching is a bar that reads as broken.
    //
    // Greyed with the ring off, like the Reading row above it: there is no ring
    // for it to hold back, and it is not what would bring one back. The switch
    // that would is the layer's own width, which is the Layers bar's second
    // handle — never greyed, since a control that greyed itself out at 0 could
    // not be dragged off it.
    ui.add_enabled_ui(view.spectral_ring_draws(), |ui| {
        ValueBar::new(&mut view.spectral_ring_gate, SPECTRAL_GATE_MIN..=SPECTRAL_GATE_MAX, "Ring threshold")
            // A percentage of the Level window, which is the axis the ring's
            // own colours are read off — so what the number names is a colour
            // on the ring rather than a dB the analyzer's window could move
            // out from under.
            .percent()
            .show(ui)
            .on_hover_text(
                "Minimum audio level needed to show a ring, as a percentage of the Spectrum level range on Analyzer. \
                 MIDI notes always show their rings. \
                 0% also shows silent rings.",
            );
        // Under the Gate because it is a property OF the gate rather than a
        // second decision beside it: what it moves is where the same threshold
        // sits for a bucket that is already lit.
        ValueBar::new(
            &mut view.spectral_ring_hysteresis,
            0.0..=SPECTRAL_HYSTERESIS_MAX,
            "Threshold hysteresis",
        )
        .percent()
        .show(ui)
        .on_hover_text(
            "How far the threshold drops once a ring appears, in percentage points of the Spectrum level range. \
                 Increase to stop rings flickering near the threshold.",
        );
        ValueBar::new(&mut view.spectral_ring_attack, 0.0..=SPECTRAL_BALLISTICS_MAX, "Level attack")
            .unit(1000.0, " ms").decimals(0)
            .show(ui)
            .on_hover_text(
                "Response time when audio in an octave slice gets louder. \
                 Independent of the Analyzer curve. \
                 0 ms responds immediately.",
            );
        ValueBar::new(
            &mut view.spectral_ring_release,
            0.0..=SPECTRAL_BALLISTICS_MAX,
            "Level release",
        )
        .unit(1000.0, " ms").decimals(0)
        .show(ui)
        .on_hover_text(
            "Response time when audio in an octave slice gets quieter. \
                 Increase to steady fluctuating harmonics. \
                 0 ms responds immediately.",
        );
    });
    // The FOLD's kernel, and so inert under Spectrum rather than merely
    // without audio: the spectrum reading shows a whole window of pitch per
    // wedge, and a kernel there would blur the one axis the window exists to
    // resolve. Its own setting is the Zoom bar below.
    let folding = view.spectral_ring_draws() && view.spectral_reading == SpectralReading::Fold;
    ui.add_enabled_ui(folding, |ui| {
        ValueBar::new(
            &mut view.spectral_width,
            SPECTRAL_WIDTH_MIN..=SPECTRAL_WIDTH_MAX,
            "Pitch tolerance",
        )
        .unit(1.0, "¢")
        .decimals(0)
        .show(ui)
        .on_hover_text(
            "Pitch distance over which audio can light an octave slice. \
                 More distant pitches appear dimmer. \
                 Widen for tempered music. \
                 Used by Octave levels only.",
        );
    });
    // The SPECTRUM reading's zoom, under the Tolerance it stands opposite: how
    // much pitch a wedge shows, where Tolerance is how much of it counts as the
    // node's.
    let zoomed = view.spectral_ring_draws() && view.spectral_reading == SpectralReading::Spectrum;
    ui.add_enabled_ui(zoomed, |ui| {
        ValueBar::new(
            &mut view.spectral_ring_range,
            SPECTRAL_RANGE_MIN..=SPECTRAL_RANGE_MAX,
            "Pitch span",
        )
        // A decimal below ten cents: the bar's floor is 0.5¢, and "{:.0}"
        // would read it out as the zero the floor exists to forbid.
        .unit(1.0, "¢").decimals(1)
        .display(|cents| if cents < 10.0 { format!("{cents:.1}¢") } else { format!("{cents:.0}¢") })
        .show(ui)
        .on_hover_text(
            "Frequency span within each octave slice, in cents. 1200¢ shows a full octave. Used by Spectrum only.",
        );
    });
}

/// Note: what the whole node does rather than any one layer of it — the time
/// it takes to arrive and leave, the curve it runs on, and the shadow it clears
/// around itself.
///
/// One section rather than a heading apiece, because they are one idea: none
/// is about the audio ring, the octave glyphs or the melody/bass marks in
/// particular, and all apply to whichever of those happen to be drawn.
/// Fade especially — one time for the node rather than one per layer, so a
/// release reads as a single gesture instead of pieces of the node going dark at
/// different moments.
fn note_section(ui: &mut egui::Ui, view: &mut ViewConfig, params: &dyn ParamBackend) {
    section(ui, "Note layers");
    // The note's timing and the curve it runs on, in that order. Fade is an
    // automatable param and Fade curve a view setting, so the two are stored apart
    // (`ViewConfig::envelope` is where they are put back together); the pane
    // is where they have to LOOK like the one setting they are.
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Fade-in and fade-out time for the whole node, including audio-ring visibility. \
                 Short notes still reach full brightness. \
                 0 ms switches immediately.",
    );
    // Linear like every bar around it, and for the same reason: the whole
    // range is one unit, so every hundredth of it — the readout's own
    // resolution — is already a couple of pixels of travel, and there is no
    // fine end for an ease to rescue. The one bar in the group that is NOT a
    // duration, hence no seconds on the readout — it is the shape the Fade
    // above it is drawn with.
    //
    // The one bar in the pane carrying a picture of itself, and the reason is
    // that its number says nothing: the Fade's seconds are a length anyone can
    // feel, while a Fade curve is a position on a scale with no unit and no
    // landmarks. The line is drawn RISING, as an arrival, because that is the
    // function itself — a release is the same curve upside down, and picking
    // the falling one would be picking a direction the setting does not have.
    ValueBar::new(&mut view.fade_shape, 0.0..=1.0, "Fade curve")
        .percent()
        .curve(|shape, p| {
            // The scene's own curve, not a second copy of the formula: the
            // preview is only worth drawing if it cannot disagree with the
            // notes, and nothing on screen would show the disagreement. A
            // one-second arrival read `p` seconds in IS the shape at that
            // fraction of any duration the Fade actually RUNS, the curve
            // being in the fraction alone — at a Fade of 0 there is no
            // transition for it to be a fraction of, and the line goes on
            // describing a curve the notes are not taking.
            harmonigraph_core::Envelope { attack_time: 1.0, shape, ..Default::default() }
                .attack(p as f64, 0.0)
        })
        .show(ui)
        .on_hover_text(
            "Shape of the note fade. \
                 0% is linear; higher values change quickly at first and settle slowly. \
                 The line previews the fade-in.",
        );
    // Every layer's size, in the one bar that can show where each of them
    // lands: one stack read outward from the node's center — where it begins,
    // and then a width apiece — and the picture on the bar is the node's own
    // cross-section (`StackBar`). A whole-note setting rather than any layer's,
    // which is why it is here and not split across the headings below — none of
    // the four numbers could be read without the other three, since a layer's
    // inner edge is a sum over everything inside it.
    //
    // Directly above the Ring gap, because the two are one idea: the sizes are
    // the layers and that gap is the padding standing between them, the bar
    // draws both, and dragging it is visibly the stack opening up. The Octave
    // gap under it is not on this bar's axis at all, and is here anyway — see
    // there.
    StackBar::new(view).show(ui).on_hover_text(
        "Node layers from the center out: empty center, audio ring, MIDI octave ring, then melody and bass marks. \
                 Drag a handle to resize its layer; zero width hides it. \
                 Double-click resets.",
    );
    // A node's two paddings, together and directly under the bar that draws one
    // of them. They are the same question asked on the node's two axes — how
    // far apart do its pieces read — so a person dialling one is looking at the
    // other, and they are compared by their numbers, which a heading between
    // them costs. Both are whole-note settings rather than any one layer's,
    // which is what puts them in Note at all.
    //
    // Two bars rather than one because the two axes answer differently: the
    // RADIAL one is measured on the Layers bar's own axis, every unit it takes
    // being a unit the three widths cannot have, and the ANGULAR one costs the
    // stack nothing — it cuts slices out of a ring already placed. What a node
    // could not say with one number is a ring standing well off its neighbour
    // while the slices stay tight, or the reverse.
    //
    // Read out as a PERCENTAGE of the node's radius, which is what quad uv 1.0
    // is (`scene.node_radius`, a quarter of the lattice spacing, and the edge
    // no ring may cross). That makes the whole stack a budget of
    // 100%, which is the picture the Layers bar draws, and it is the same unit
    // the Clearance below reads in. A tenth of a percent is exactly the
    // resolution three decimals of the stored number gives, so the readout
    // trades no precision for the point: the fresh 5.2% and the 4.8% beside it
    // are one number at a coarser one, which is where the bar would go quiet
    // exactly as it is being dialled in. Numeric entry uses the displayed
    // percentage too; the widget converts it back to the stored fraction.
    ValueBar::new(&mut view.ring_gap, 0.0..=GAP_MAX, "Layer gap").percent().show(ui).on_hover_text(
        "Space between concentric layers, as a percentage of the node radius. 0% joins the layers.",
    );
    ValueBar::new(&mut view.octave_gap, 0.0..=GAP_MAX, "Sector gap")
        .percent()
        .show(ui)
        .on_hover_text(
            "Space between octave slices in the audio ring, MIDI ring and marks, as a percentage of the node radius. \
                 0% joins the slices.",
        );
}
