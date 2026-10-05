//! Lattice note layers, their sizes and timing. Their light follows on the same page.

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{choice_row, StackBar, ValueBar};
use harmonigraph_scene::{
    AnimationOrder, SpectralReading, ViewConfig, GAP_MAX, MARK_DELAY_MAX, MAX_SPAN, MIN_SPAN,
    PITCH_CEIL, PITCH_FLOOR, SPECTRAL_BALLISTICS_MAX, SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN,
    SPECTRAL_WIDTH_MAX, SPECTRAL_WIDTH_MIN,
};
use harmonigraph_scene::{LATTICE_GROUND_RANGE, RADIAL_START_RANGE, STAGGER_SPREAD_RANGE};

// The Lattice page's independently folded note controls ([`super::pages`]).

/// Octaves: which octaves of the pitch class are sounding, shown as arcs of a
/// pitch axis shared by the MIDI ring, audio ring and melody/bass marks.
pub(super) fn octaves(ui: &mut egui::Ui, view: &mut ViewConfig) {
    let mut count = view.octave_count as f32;
    ValueBar::new(&mut count, MIN_SPAN as f32..=MAX_SPAN as f32, "Octaves")
        .integer()
        .show(ui)
        .on_hover_text("Equal octave slices shared by the MIDI ring, audio ring and marks. Notes beyond the range use the end slices.");
    view.octave_count = count as u32;
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
}

/// Audio ring: what the ring inside the octave band measures — one reading of
/// the analyzer's spectrum, or none.
///
/// First of the layers, which is where it sits in the stack: reaching the
/// node's own centre, a gap in from the octave Band below. It is the one
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
/// chosen: Pitch tolerance is the fold's kernel and Pitch span is the
/// spectrum's window, and neither means anything to the other. Both are shown
/// either way rather than swapped in and out, so the section keeps its height
/// and the bars keep their place as the row is clicked along. A hidden layer
/// keeps an explanation of how to restore it; the Note layers bar remains its
/// only visibility control.
pub(super) fn audio_ring(ui: &mut egui::Ui, view: &mut ViewConfig) {
    if !view.spectral_ring_draws() {
        crate::widgets::weak(ui, "Audio ring — hidden");
        crate::widgets::weak(ui, "Increase Audio width in Note layers to show it.");
        return;
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
    // WHICH NODES wear the ring, where the Layers bar in Note layers is how thick
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
    crate::widgets::threshold(ui, &mut view.spectral_ring_gate, &mut view.spectral_ring_hysteresis)
        .on_hover_text("MIDI notes always show their rings. For other nodes, the loudest wedge opens the ring at this threshold; hysteresis sets the closing threshold. Ring visibility follows Note fade.");
    crate::widgets::response(
        ui,
        &mut view.spectral_ring_attack,
        &mut view.spectral_ring_release,
        SPECTRAL_BALLISTICS_MAX,
        ["Ring attack", "Ring release"],
        1000.0,
    ).on_hover_text("Ring attack and release smooth the ring levels after the Analyzer’s Live attack and release.");
    // The FOLD's kernel, and so inert under Spectrum rather than merely
    // without audio: the spectrum reading shows a whole window of pitch per
    // wedge, and a kernel there would blur the one axis the window exists to
    // resolve. Its own setting is the Pitch span bar below.
    let folding = view.spectral_reading == SpectralReading::Fold;
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
    // The SPECTRUM reading's zoom, under the Pitch tolerance it stands opposite:
    // how much pitch a wedge shows, where Pitch tolerance is how much of it
    // counts as the node's.
    let zoomed = view.spectral_reading == SpectralReading::Spectrum;
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

/// Geometry first: every layer is sized on the same radius budget.
pub(super) fn layers(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // Every layer's size, in the one bar that can show where each of them
    // lands: one stack read outward from the node's center — where it begins,
    // and then a width apiece — and the picture on the bar is the node's own
    // cross-section (`StackBar`). A whole-note setting rather than any layer's,
    // which is why it is here and not split across the headings below — none of
    // the four numbers could be read without the other three, since a layer's
    // inner edge is a sum over everything inside it.
    //
    // Directly above the Gap, because the two are one idea: the sizes are
    // the layers and that gap is the padding standing between them, the bar
    // draws both, and dragging it is visibly the stack opening up.
    StackBar::new(view).show(ui).on_hover_text(
            "Node layers from the center out: empty center, audio ring, MIDI octave ring, then melody and bass marks. \
                     Drag a handle to resize its layer; zero width hides it. \
                     Double-click resets.",
        );
    // One node-wide padding directly under the bar that draws its radial use.
    // The same value spaces the concentric layers and cuts the sectors, so the
    // two axes carry one rhythm of empty space. It is a whole-note setting
    // rather than any one layer's, which is what puts it in Note layers.
    //
    // Stored in quad uv, where 1.0 is the edge no ring may cross, and read
    // out as a percentage of the TRUE node radius (`QUAD_UV_PERCENT`): that
    // edge is 180%, the budget the whole stack on the Layers bar shares.
    // Numeric entry uses the displayed percentage too; the widget converts it
    // back to the stored uv.
    ValueBar::new(&mut view.ring_gap, 0.0..=GAP_MAX, "Gap")
        .unit(super::QUAD_UV_PERCENT, "%")
        .decimals(1)
        .show(ui)
        .on_hover_text(
            "Space between concentric layers and between octave sectors, as a percentage of the node radius. \
                     0% joins both layers and sectors.",
        );
    // L*, shared with the gradient brightness scale. Black is a colour here;
    // layer width, rather than brightness, controls whether a ring exists.
    ValueBar::new(&mut view.lattice_ground, LATTICE_GROUND_RANGE, "Silent slice brightness")
        .unit(1.0, "%")
        .integer()
        .show(ui)
        .on_hover_text(
            "Brightness of silent MIDI octave slices in sounding or fading nodes, and the audio ring's quiet endpoint. \
             0% is black, 100% is white. Idle lattice positions have their own label/cross brightness.",
        );
}

/// Shared visibility timing followed by the MIDI slices' motion and ordering.
pub(super) fn motion(ui: &mut egui::Ui, view: &mut ViewConfig, params: &dyn ParamBackend) {
    let key = ParamKey::Fade;
    let mut seconds = params.get(key);
    let before = seconds;
    let response = crate::widgets::fade(ui, &mut seconds, &mut view.fade_shape);
    if response.drag_started() {
        params.begin_set(key);
    }
    if seconds != before {
        params.set(key, seconds);
    }
    if response.drag_stopped() {
        params.end_set(key);
    }
    ui.add_enabled_ui(view.marks_draw(), |ui| {
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
    // Built off `ALL` with an exhaustive match rather than written out, the
    // way `SpectralOrientation`'s row is and for its reason: another order
    // cannot reach this pane without a name and a hint of its own.
    // Every hint here is about WHEN a slice starts and nothing else: the orders
    // differ in the delay each slice waits, never in what it then does, which
    // is always the same smooth arrival.
    let orders = AnimationOrder::ALL.map(|order| {
            let (label, hint) = match order {
                AnimationOrder::Circular => (
                    "Circular",
                    "Slices start one after another at the seam where the highest and lowest meet, sweeping from low to high pitch.",
                ),
                AnimationOrder::Bidirectional => (
                    "Bidirectional",
                    "Both ends start at the seam where the highest and lowest meet, then sweep toward each other across the ring.",
                ),
                AnimationOrder::RandomStagger => (
                    "Random stagger",
                    "Each slice takes a delay of its own, in an order scrambled per node and per press. The note's release reuses the same order.",
                ),
            };
            (order, label, hint)
        });
    choice_row(ui, "Slice order", &mut view.note_animation.order, &orders);
    crate::widgets::checkbox(ui, &mut view.note_animation.lit_first, "Lit slices first")
        .on_hover_text("The slices a note lights start with the first slice on arrival and with the last one on release, so they appear first and disappear last. Every other slice keeps the slice order's timing. Needs a stagger spread above zero.");
    ValueBar::new(&mut view.note_animation.stagger_spread, STAGGER_SPREAD_RANGE, "Stagger spread")
                .unit(100.0, "%")
                .show(ui)
                .on_hover_text("Time between the first and last slice starts, as a percentage of Note fade. Every slice still animates for the whole Note fade, so the arrival and the release each last that much longer -- and a note released before it finishes departs without order. Zero starts every slice together.");
    ValueBar::new(&mut view.note_animation.radial_start, RADIAL_START_RANGE, "Starting offset & scale")
            .unit(100.0, "%").show(ui)
            .on_hover_text("Starting offset and scale of each MIDI slice and mark. -100% grows from a point at the node center; 0% starts at rest; +100% starts twice as far out and at twice its final size. Scale follows offset so slice and gap proportions stay consistent.");
}
