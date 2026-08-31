//! The node settings on the Display tab's Lattice page: how a sounding note is
//! drawn — the audio ring at its centre, the octave band around that, the
//! melody/bass marks on the outer held notes, the fade and clearance the whole
//! node wears, and the light it gives off. Everything here is a layer of the
//! *played note*, or the whole of one.
//!
//! The pitch gradient and Bloom are NOT here, though a node wears both: they
//! are color, so they are on the Colors page ([`super::color`]) with the other
//! table. The text a node carries is [`super::labels`], kept out because a
//! label rides a hovered and a remembered node as readily as a sounding one.

use super::spectral::settings::ms_readout;
use super::{param_bar, section};
use crate::params::{seconds, ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, GlowCurveBar, OctaveStrip, StackBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    Pulse, ShadowKernel, SpectralReading, ViewConfig, GAP_MAX, GLOW_BALLISTICS_MAX, GLOW_REACH_MAX,
    GLOW_SHADOW_CURVE_MAX, GLOW_SHADOW_CURVE_MIN, GLOW_SHADOW_GAIN_MAX, GLOW_SHADOW_MAX,
    GLOW_SHADOW_NAME_MAX, GLOW_STRENGTH_MAX, MARK_DELAY_MAX, MIN_EXTRA_SIZE, PITCH_CEIL,
    PITCH_FLOOR, SHADOW_SHAPE_MAX, SHADOW_SHAPE_MIN, SPECTRAL_BALLISTICS_MAX, SPECTRAL_GATE_MAX,
    SPECTRAL_GATE_MIN, SPECTRAL_HYSTERESIS_MAX, SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN,
    SPECTRAL_WIDTH_MAX, SPECTRAL_WIDTH_MIN,
};

/// The sounding-note controls: the whole note first — the time it takes to
/// arrive and leave, the clearance it keeps, and the light it gives off — then
/// each layer of it reading outward from the center, and last the sweep the
/// outermost layer can be set to run.
///
/// **Whole-note before the layers, because none of those settings belongs to a
/// layer** — the SIZES especially, which are one stack read outward from the
/// node's center, and the Ring gap, which is the one number standing every
/// layer of it off the one inside — and because they are the ones reached for
/// most.
/// Filing them after Audio ring, Octaves and the marks would put the most-used
/// controls under the ones reached for least, on the strength of an outward
/// reading they are not part of. The Glow is whole-note too and sits with Note
/// for that reason, under a heading of its own for the reason its section
/// gives.
///
/// **Every layer's size is one bar, and each section below is then what that
/// layer IS.** The three widths are not three independent numbers — a layer's
/// inner edge is a sum over everything inside it, so a bar apiece can say how
/// thick a ring is and never where it lands — and a heading apiece would split
/// the one question a size on a node is asked, where does this sit, across
/// three places holding a third of the answer each. The Layers bar in Note is
/// the whole of it ([`StackBar`]), and what is left under each heading is the
/// reading it carries, the colours it wears and the switches that are its own.
///
/// The layers then run in stack order, from the center out: the audio ring on
/// the stack's own start, the octave band a Ring gap outside it, and the
/// melody/bass marks past the band — the same order [`ViewConfig::rings`] lays
/// them down in, so the column reads down as the node reads outward. Shimmer is
/// last because it is the sweep the outermost layer carries rather than a layer
/// of its own.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    note_section(ui, &mut state.view, params);
    glow_section(ui, &mut state.view);
    audio_section(ui, &mut state.view);
    octaves_section(ui, &mut state.view);
    melody_bass_section(ui, &mut state.view);
    shimmer_section(ui, &mut state.view);
}

/// Octaves: which octaves of the pitch class are sounding, shown as arcs of a
/// pitch axis that runs once round the node. Independent of the audio ring.
fn octaves_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Octaves");
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
        "How many octaves one turn of the node covers, of the eleven slots. \
         Drag between the handles for full-size octaves, outside them for small \
         extras at each end. Notes past either end light the outermost slice.",
    );
    // Whole semitones, because that is the step the wheel can act on and what
    // the readout can name.
    ValueBar::new(&mut view.octave_center, PITCH_FLOOR..=PITCH_CEIL, "Center")
        .integer()
        .display(super::pitch_readout)
        .show(ui)
        .on_hover_text(
            "The pitch at the top of the wheel, on every node. Each node shows \
             its own octaves nearest this one.",
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
        ValueBar::new(&mut view.octave_extra_size, MIN_EXTRA_SIZE..=1.0, "Extra size")
            .show(ui)
            .on_hover_text(
                "How wide an extra octave draws, as a share of an even slice. \
                 Under 1 an extra is always narrower than a full octave; 1 \
                 makes the wheel even.",
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
        ValueBar::new(&mut view.octave_extra_blend, 0.0..=1.0, "Extra blend")
            .show(ui)
            .on_hover_text(
                "How the extras grade: 0 is a flat fringe, 1 one ramp from the \
                 outermost extra inward. The outermost never moves.",
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
    section(ui, "Melody / bass");
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
        ui.checkbox(&mut view.mark_melody, "Melody")
            .on_hover_text("Extend the highest held note's octave slice past the band");
        ui.checkbox(&mut view.mark_bass, "Bass")
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
        // Linear, unlike the wide bars that need easing to be draggable
        // at their fine end: one second of travel puts a hundredth of it
        // — the readout's own resolution — a couple of pixels apart, and
        // the settings that matter (a passing sixteenth at 120bpm is
        // 125ms) sit in the first third rather than crushed at the
        // bottom. Read out in seconds, the one value in this section that
        // is not a length.
        ValueBar::new(&mut view.mark_delay, 0.0..=MARK_DELAY_MAX, "Delay")
            .display(seconds)
            .show(ui)
            .on_hover_text(
                "How long a note must stay the melody or the bass before \
                 its mark fades in — keeps fast playing from flickering \
                 marks. 0 marks instantly.",
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
    (
        Pulse::Bands,
        "Bands",
        "Parallel bands laid diagonally, travelling along their own normal. \
         The plainest reading of light crossing the lattice",
    ),
    (
        Pulse::Checker,
        "Checker",
        "Two crossed gratings multiplied: a checkerboard with the corners \
         rounded off, its light and dark cells swapping as the sheet slides",
    ),
    (
        Pulse::Hex,
        "Hex",
        "Three gratings sixty degrees apart: a honeycomb of bright cells. \
         Tessellates with the lattice where a checkerboard fights it — the \
         rows here run three ways, not two",
    ),
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
    section(ui, "Shimmer");
    // Off is its own option rather than a checkbox beside the row: one row
    // says both whether the lattice shimmers and how.
    choice_row(ui, "Pattern", &mut view.pulse_marks, SHIMMER_PATTERNS);
    ui.add_enabled_ui(view.pulse_marks.sweeps(), |ui| {
        ValueBar::new(&mut view.shimmer_speed, 0.0..=6.0, "Speed").show(ui).on_hover_text(
            "How fast the sheet travels, in lattice units a second — \
                 the same rate on screen and in a render. 0 freezes it.",
        );
        // Eased, because the range is three orders wide and the useful
        // settings are not spread evenly over it: the tight end is a
        // different picture every few hundredths (0.05 to 0.1 halves the
        // periods on a node), where the wide end changes little between 8
        // and 15. Geometric travel gives each end the same share of the bar.
        ValueBar::new(&mut view.shimmer_width, 0.05..=15.0, "Spacing")
            .eased(true)
            .show(ui)
            .on_hover_text(
                "The pattern's wavelength, one bright peak to the next, in \
                 lattice units. Around five nodes reads as a sweep; under \
                 one node it becomes a texture.",
            );
        ValueBar::new(&mut view.shimmer_intensity, 0.0..=2.0, "Intensity").show(ui).on_hover_text(
            "How far a peak stands above the trough beside it — the \
                 same ratio on dark notes as bright ones. 0 is no shimmer, \
                 1 the tuned depth.",
        );
        ValueBar::new(&mut view.shimmer_softness, 0.0..=1.0, "Softness").show(ui).on_hover_text(
            "How gradually the light arrives: high fades peak into \
                 trough across the whole period, low narrows the peak to a \
                 hard band.",
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
    // "Reading" and not "Ring", though the ring is what it fills: what this row
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
            "Reading",
            &mut view.spectral_reading,
            &[
                (
                    SpectralReading::Fold,
                    "Fold",
                    "One reading per wedge, taken at that octave's own pitch: how \
                     much is sounding there, folded onto the node. A timbre \
                     lights a constellation around its own node. The reading for \
                     a screenful of nodes.",
                ),
                (
                    SpectralReading::Spectrum,
                    "Spectrum",
                    "Each wedge is a window of the raw spectrum bent into its \
                     arc, so a partial's detuning paints off-center. The reading \
                     for one node up close.",
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
        ValueBar::new(&mut view.spectral_ring_gate, SPECTRAL_GATE_MIN..=SPECTRAL_GATE_MAX, "Gate")
            // A percentage of the Level window, which is the axis the ring's
            // own colours are read off — so what the number names is a colour
            // on the ring rather than a dB the analyzer's window could move
            // out from under.
            .display(|level| format!("{:.0}%", level * 100.0))
            .show(ui)
            .on_hover_text(
                "How loud a node's loudest wedge has to read before that node \
                 draws a ring at all, as a share of the analyzer's Level \
                 window. 0 rings every node, silence included; dialled up, a \
                 ring means something is sounding there. A node you are playing \
                 rings whatever this says, and rings arrive and leave on the \
                 Fade.",
            );
        // Under the Gate because it is a property OF the gate rather than a
        // second decision beside it: what it moves is where the same threshold
        // sits for a bucket that is already lit.
        ValueBar::new(
            &mut view.spectral_ring_hysteresis,
            0.0..=SPECTRAL_HYSTERESIS_MAX,
            "Gate hold",
        )
        .display(|level| format!("{:.0}%", level * 100.0))
        .show(ui)
        .on_hover_text(
            "How far the Gate drops once a node is lit, so a level sitting on \
             the threshold stops switching. 0 is one threshold. The Fade sets \
             how SLOWLY a ring comes and goes; this sets how RARELY.",
        );
        ValueBar::new(&mut view.spectral_ring_attack, 0.0..=SPECTRAL_BALLISTICS_MAX, "Ring attack")
            .display(ms_readout)
            .show(ui)
            .on_hover_text(
                "How long a wedge takes to brighten toward a louder reading. Its \
             own time, not the analyzer's: the Spectral pane wants what is \
             there, and the ring wants whether a harmonic is present.",
            );
        ValueBar::new(
            &mut view.spectral_ring_release,
            0.0..=SPECTRAL_BALLISTICS_MAX,
            "Ring release",
        )
        .display(ms_readout)
        .show(ui)
        .on_hover_text(
            "How long it takes to dim toward a quieter one. The knob for \
             twinkle: most of what shimmers between partials is the estimate \
             wobbling downward.",
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
            "Tolerance",
        )
        .display(|cents| format!("{cents:.0}¢"))
        .show(ui)
        .on_hover_text(
            "How far off an octave's own pitch a partial may sit and still light \
             its wedge, in cents. A weight, not a cutoff: distance reads as \
             dimness. Narrow suits just intonation; tempered material wants it \
             wider. Fold reading only.",
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
            "Zoom",
        )
        // A decimal below ten cents: the bar's floor is 0.5¢, and "{:.0}"
        // would read it out as the zero the floor exists to forbid.
        .display(|cents| if cents < 10.0 { format!("{cents:.1}¢") } else { format!("{cents:.0}¢") })
        .show(ui)
        .on_hover_text(
            "How much spectrum one wedge shows, in cents around that octave's \
             own pitch. At the top of the bar a wedge spans its whole octave and \
             the ring joins into one continuous reading. Spectrum reading only.",
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
    section(ui, "Note");
    // The note's timing and the curve it runs on, in that order. Fade is an
    // automatable param and Fade curve a view setting, so the two are stored apart
    // (`ViewConfig::envelope` is where they are put back together); the pane
    // is where they have to LOOK like the one setting they are.
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a note takes to arrive, and to leave once released — the whole \
         node at once, the audio ring's coming and going included. Short notes \
         still reach full brightness. 0 switches on and off outright.",
    );
    // Linear like every bar around it, and for the same reason: the whole
    // range is one unit, so every hundredth of it — the readout's own
    // resolution — is already a couple of pixels of travel, and there is no
    // fine end for an ease to rescue. The one bar in the group that is NOT a
    // duration, hence no seconds on the readout — it is the shape the Fade
    // above it is drawn with.
    //
    // The one bar in the pane carrying a picture of itself, and the reason is
    // that its number says nothing: a Fade of 0.15 is a length anyone can feel
    // and a Fade curve of 0.35 is a position on a scale with no unit and no
    // landmarks. The line is drawn RISING, as an arrival, because that is the
    // function itself — a release is the same curve upside down, and picking
    // the falling one would be picking a direction the setting does not have.
    ValueBar::new(&mut view.fade_shape, 0.0..=1.0, "Fade curve")
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
            "The curve both ends of the Fade run on, drawn as an arrival. 0 is \
             a straight line; higher moves fast and settles slowly, the way a \
             struck note decays.",
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
        "A node's cross-section, from its center out: the empty middle its \
             light fills, the audio ring, the octave band and the melody/bass \
             strip, each named on its own cell where there is room for it. Drag \
             a handle to set the stretch inside it — 0 removes a layer and the \
             ones outside close up, and on the middle it seats the whole stack \
             on the node's center. The line is the node's edge, which only the \
             marks may cross. Double-click to restore.",
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
    // exactly as it is being dialled in. Typing still takes the bare stored
    // number, as it does under every other display formatter here.
    ValueBar::new(&mut view.ring_gap, 0.0..=GAP_MAX, "Ring gap")
        .decimals(3)
        .display(|v| format!("{:.1}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "The padding between one ring of a node and the next, and before \
             the melody/bass marks, as a share of the node's radius. 0 closes \
             the stack up solid.",
        );
    ValueBar::new(&mut view.octave_gap, 0.0..=GAP_MAX, "Octave gap")
        .decimals(3)
        .display(|v| format!("{:.1}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "The padding between one octave sector and the next — on the band, \
             on the audio ring's wedges and down a mark's sides — as a share of \
             the node's radius. 0 closes the ring into a solid annulus.",
        );
    // What a node CLEARS around itself is not here: it is the Glow section's
    // Shadow directly below, which is one length for the hole and the shadow laid
    // over it. A bar of its own here would be the same distance said twice.
    //
    // The glow is NOT here either, though it is the whole node's as everything
    // above is: it has a section of its own directly below, and a heading is
    // what lets every bar in it drop the "Glow" from its name.
}

/// Glow: the light a node gives off — the ONLY light it has, every layer of
/// the stack being a crisp shape. Laid in the same octave colours the band is,
/// reaching past the node's outermost drawn edge and filling the empty middle
/// its rings stand around.
///
/// **A section of its own, directly under Note**, and not the tail of it: a
/// glow is the whole node's rather than any layer's, which is what Note is
/// for, but it is also one feature carrying a bar for every question asked of
/// one light. A column of them all named "Glow …" at the foot of Note is one
/// word the eye has to skip past to find the one that differs; a heading says
/// the word once. It leads the layers rather than following Shimmer because,
/// like everything in Note, it is reached for more than any one layer is.
///
/// **Not with Bloom**, which it otherwise reads like, because the two are
/// different settings of different things. Bloom is one number over every
/// picture the plugin draws and it thresholds, so it is the bright end of the
/// gradient blowing out — colour, which is why it sits with the table that
/// makes it. This is a layer of a node, dialled where a node's other layers
/// are.
///
/// The bars run in the order the questions do: what the light IS before what
/// colour it is, both before what holds it off the rings, and its own clock
/// last. Everything under Reach greys while Reach is 0 rather than hiding, so
/// the rows keep their place and the numbers they are dialled to stay
/// readable — the arrangement the audio ring's own settings use.
fn glow_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Glow");
    // A share of the node's radius, the unit the two gaps and the Clearance in
    // Note read in, and measured from the same place: the reach is a distance
    // out from the node's edge exactly as the Clearance is. Eased, because the
    // bar spans two pictures rather than one range of one: the accent — a halo
    // reaching about as far as the gap to a neighbour — is the bottom eighth
    // of it, and the wash is everything above. Cubic travel gives the accent
    // half the bar, so a light meant to sit on its own node is still dialled a
    // hundredth at a time, and the far end is reachable in the same drag.
    ValueBar::new(&mut view.glow_reach, 0.0..=GLOW_REACH_MAX, "Reach")
        .eased(true)
        .decimals(3)
        .display(|v| format!("{:.0}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "How far past its own edge a node's light spreads, as a share of \
             its radius. Neighbouring nodes' light melds where it overlaps — \
             around 100% reaches a neighbour, and several hundred washes the \
             whole lattice in one field. 0 turns the glow off, leaving the \
             rings alone.",
        );
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
        ValueBar::new(&mut view.glow_strength, 0.0..=GLOW_STRENGTH_MAX, "Strength")
            .show(ui)
            .on_hover_text("How much light a node lays down. 1 is the tuned amount.");
        GlowCurveBar::new(&mut view.glow_curve).show(ui).on_hover_text(
            "How the light fades from the node's full-bright centre at the \
                 left to the edge of its Reach at the right. Drag the three \
                 points to set the light left one, two and three quarters of \
                 the way out. Points cannot cross, so the glow always fades \
                 outward; lift the far point for a long quiet tail. \
                 Double-click to restore.",
        );
        // What colour the light comes out, between the amount of it and the
        // Shadow under it. "Color blend" and not "Spread": under this heading,
        // beside a Reach that is about distance, a "spread" reads as how far
        // the light goes, and this moves no light at all. It reads as a
        // percentage because it is a SHARE — of a whole turn — and not a
        // distance.
        ValueBar::new(&mut view.glow_blend, 0.0..=1.0, "Color blend")
            .display(|v| format!("{:.0}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "How far round the node its own colours are averaged into the \
                 colour of its light. The glow is what the node is PLAYING, \
                 blurred round it — the octave band's colours and the melody \
                 and bass marks', each weighted by how lit it is and how wide \
                 the Layers bar made it, with the audio ring left out: a ring \
                 is what the room is doing rather than the note, so it is drawn \
                 and never shone. 0% keeps each sector's colour a distinct arc \
                 and 100% averages the node into one tint.",
            );
        // The Shadow, beneath the light it lands on: ONE width, read out like
        // the two gaps and the Clearance, every one of them a share of the
        // node's radius, so a person comparing them is comparing one unit. A
        // tenth of a percent, as the gaps are.
        ValueBar::new(&mut view.glow_shadow, 0.0..=GLOW_SHADOW_MAX, "Shadow")
            .display(|v| format!("{:.1}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "How wide a shadow everything in the lattice casts — a node's \
                 audio ring, octave band and marks, the cross standing at each \
                 resting position, and every note name — as a share of the \
                 node's radius. Each item's shadow is a blur of its own ink, \
                 laid over whatever is already behind it, so a thin ring casts \
                 a fainter one than a wide band and a nearer item darkens a \
                 farther one wherever the two overlap. Nothing darkens itself. \
                 0% is the picture with no shadow at all. Double-click to \
                 restore.",
            );
        ValueBar::new(&mut view.glow_shadow_depth, 0.0..=1.0, "Shadow depth")
            .display(|v| format!("{:.0}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "How dark a shadow gets where it is deepest. 100% takes what is \
                 under a solid ring to black, and lower leaves it sitting in a \
                 dimmer pool of the light around it rather than in a void. It \
                 is a FLOOR: a wide caster reaches it and a hairline one stops \
                 short, which is how a shadow says how thick the ink casting it \
                 is. 0% is the picture with no shadow at all.",
            );
        // The kernel, above the three bars that shape what it draws: it is the
        // only setting here that changes what the blur IS rather than how the
        // blur is spent, and the only one that costs atlas.
        choice_row(
            ui,
            "Kernel",
            &mut view.glow_shadow_kernel,
            &[
                (
                    ShadowKernel::Gaussian,
                    "Gaussian",
                    "One plain blur. The shape a shadow has had all along, and \
                     what every other setting here is calibrated against. One \
                     cell of the shadow atlas per item.",
                ),
                (
                    ShadowKernel::TwoScale,
                    "Two-scale",
                    "A tight core with a wide skirt under it, 70/30 at 1:3. The \
                     cheapest departure from a plain blur: the shadow keeps a \
                     definite edge near the ink and carries a faint pool well \
                     past where a Gaussian has stopped. Two cells per item.",
                ),
                (
                    ShadowKernel::Distance,
                    "Distance",
                    "Not a blur at all: how far each point stands from the \
                     nearest ink, spent through the Shadow shape bar. A blur \
                     of a hairline is a smudge whose width is the hairline's, \
                     so at a wide Shadow the whole lattice reads as one soft \
                     blob; a distance is the same at a hairline as at a slab, \
                     so a letter's counters, a cross's arms and a corner all \
                     keep their shape however wide this is dialled. What it \
                     gives up is the pocket between two strokes standing \
                     close.",
                ),
            ],
        );
        // The two that shape the depth rather than set it, under the pair they
        // act on: how much of it the picture's THINNEST ink gets, and where
        // along the shadow's width it sits.
        // The gain is the BLUR family's, on the same rule the two bars below it
        // are gated by: a distance field gives a hairline the whole depth at
        // its own edge by construction, so `shadow_kernel` returns before the
        // gain is ever read on a distance row and the bar moves nothing there.
        ui.add_enabled_ui(!view.glow_shadow_kernel.floods(), |ui| {
            ValueBar::new(&mut view.glow_shadow_gain, 0.0..=GLOW_SHADOW_GAIN_MAX, "Shadow gain")
                .display(|v| format!("{v:.2}x"))
                .show(ui)
                .on_hover_text(
                    "How much of the Shadow depth the picture's THINNEST ink \
                     gets, on the blur rows of the Kernel picker. A shadow is \
                     a blur of the thing casting it, so a wide band blurs to a \
                     solid pool and a hairline ring or a stroke of type blurs \
                     to a faint smudge — and without this they would land at \
                     wildly different darknesses from one depth. Turning it up \
                     brings the thin ink up toward the depth the bar names and \
                     leaves the wide ink exactly where it is, until at the top \
                     everything in the lattice is one flat silhouette. 0x is \
                     the picture with no shadow at all. A Distance row needs \
                     none of it: a hairline already stands at the whole depth \
                     against its own ink.",
                );
        });
        ValueBar::new(
            &mut view.glow_shadow_curve,
            GLOW_SHADOW_CURVE_MIN..=GLOW_SHADOW_CURVE_MAX,
            "Shadow curve",
        )
        .display(|v| format!("{v:.2}"))
        .show(ui)
        .on_hover_text(
            "Where along a shadow's width its darkness sits. Both ends stay \
             put — the pool under solid ink stays at the depth, and the far \
             edge stays at nothing — and this bends everything between them. \
             Below 1 fills the middle out into a broad even pool that gives \
             way suddenly at its edge; above 1 pulls the darkness in tight \
             against the ink and lets the rest go gently, which reads as a \
             rim. 1 is the plain blur, and the shadow is exactly the shape of \
             the blur that made it.",
        );
        // The distance family's own profile bar, live only where a distance
        // row is picked: a blur row reads it not at all, and a bar that moves
        // nothing is worse than one that is plainly not for this row. Grayed
        // rather than hidden, so the section does not change height under the
        // picker and the hover text is there to say what would switch it on.
        ui.add_enabled_ui(view.glow_shadow_kernel.floods(), |ui| {
            ValueBar::new(
                &mut view.glow_shadow_shape,
                SHADOW_SHAPE_MIN..=SHADOW_SHAPE_MAX,
                "Shadow shape",
            )
            .display(|v| format!("{v:.2}"))
            .show(ui)
            .on_hover_text(
                "How a distance shadow gives way as it leaves the ink, on the \
                 Distance rows of the Kernel picker. 1 spends the darkness \
                 evenly across the whole width — the shadow eases off the way \
                 a blur does. Lower steepens it right against the ink and \
                 leaves the rest of the width carrying a faint haze, which \
                 reads as a thin dark skin on the shape rather than a pool \
                 around it. Both ends stay put: solid at the ink, nothing at \
                 the far edge.",
            );
        });
        // The one bar in the section a single caster keeps to itself, and the
        // one that breaks "one Shadow width across the picture" — see
        // `ViewConfig::glow_shadow_name` for why a letterform is the ink
        // allowed to ask.
        ValueBar::new(&mut view.glow_shadow_name, 0.0..=GLOW_SHADOW_NAME_MAX, "Name shadow")
            .display(|v| format!("{:.0}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "How wide a note name's own shadow is, as a share of the \
                 Shadow width everything else in the lattice casts. 100% is \
                 one width across the whole picture, which is what every other \
                 item takes. A name is the only ink here whose SHAPE has to be \
                 read, so the blur that says how thick a ring is may not be \
                 the blur that keeps a letter legible: wider lifts the name \
                 off the lattice on a soft pool, narrower keeps the shadow \
                 inside the letterforms, and 0% is the letters dropped as a \
                 hard-edged copy of themselves with no blur at all.",
            );
        // The ink's own share of the same field, under the bar that says the
        // ground's: the pair is one question asked twice, and the answers are
        // free of each other on purpose — a dark pool with a tinted ring in it
        // is a picture no single coupled dial can name. Only the LIT ink is
        // dialled, the rest of the lattice always taking the whole field, for
        // the reason the hover text gives.
        ValueBar::new(&mut view.glow_wash, 0.0..=1.0, "Wash")
            .display(|v| format!("{:.0}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "How much of the light a SOUNDING slice washes over its own \
                 ink — a lit octave indicator, a wedge the analyzer is \
                 reading, and the melody or bass mark continuing one. 100% \
                 lays the whole field over it and the slice melts into its own \
                 halo; lower brings it back out of the light until, at 0%, it \
                 is drawn exactly as it is with the glow off. \
                 Everything else in the lattice — a silent slice's grey, an \
                 empty wedge, the resting crosses between the nodes — always \
                 takes the whole light, which is what keeps it from reading as \
                 holes punched where the light is brightest. It reads the \
                 light before the shadow takes any, so it is free of the depth \
                 above: a full depth with a wash on it is a slice standing in \
                 a dark pool and still wearing the halo's colour.",
            );
        // The light's own clock, last, under everything it shapes. Its own pair
        // and not the note Fade in Note, because a halo is the slow part of the
        // picture: on the layers' envelopes it flickers with the marks, which
        // are meant to be fast.
        ValueBar::new(&mut view.glow_attack, 0.0..=GLOW_BALLISTICS_MAX, "Attack")
            .display(ms_readout)
            .show(ui)
            .on_hover_text(
                "How long a node's light takes to come up behind the note that \
                 lit it. Its colour arrives on the same time, so a chord's hue \
                 morphs in rather than switching.",
            );
        ValueBar::new(&mut view.glow_release, 0.0..=GLOW_BALLISTICS_MAX, "Release")
            .display(ms_readout)
            .show(ui)
            .on_hover_text(
                "How long it takes to leave. A node keeps glowing after every \
                 layer on it has gone silent, holding the colour it last had, \
                 which is what makes the light read as light rather than as one \
                 more layer of the node.",
            );
    });
}
