//! The node settings on the Display tab's Lattice page: how a sounding note is
//! drawn — its core mark, the audio ring and octave band around it, the
//! melody/bass marks on the outer held notes, and the fade and clearance the
//! whole node wears. Everything here is a layer of the *played note*, or the
//! whole of one.
//!
//! The pitch gradient and Bloom are NOT here, though a node wears both: they
//! are color, so they are on the Colors page ([`super::color`]) with the other
//! table. The text a node carries is [`super::labels`], kept out because a
//! label rides a hovered and a remembered node as readily as a sounding one.

use super::spectral::settings::ms_readout;
use super::{edge_bar, param_bar, section};
use crate::params::{seconds, ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, OctaveStrip, StackBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    Pulse, SpectralReading, ViewConfig, GAP_MAX, GLOW_REACH_MAX, GLOW_STRENGTH_MAX,
    MARK_DELAY_MAX, MIN_EXTRA_SIZE, PITCH_CEIL,
    PITCH_FLOOR, SPECTRAL_BALLISTICS_MAX, SPECTRAL_GATE_MAX, SPECTRAL_GATE_MIN,
    SPECTRAL_HYSTERESIS_MAX, SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN,
    SPECTRAL_WIDTH_MAX, SPECTRAL_WIDTH_MIN,
};

/// The sounding-note controls: the whole note first — the time it takes to
/// arrive and leave, and the clearance it keeps — then each layer of it reading
/// outward from the center, and last the sweep the outermost layer can be set
/// to run.
///
/// **Whole-note before the layers, because none of those settings belongs to a
/// layer** — the SIZES especially, which are one stack read outward from the
/// node's center, and the Ring gap, which is the one number standing every
/// layer of it off the one inside — and because they are the ones reached for
/// most.
/// Filing them after Core, Octaves and the marks would put the most-used
/// controls under the ones reached for least, on the strength of an outward
/// reading they are not part of.
///
/// **Every layer's size is one bar, and each section below is then what that
/// layer IS.** The four widths are not four independent numbers — a layer's
/// inner edge is a sum over everything inside it, so a bar apiece can say how
/// thick a ring is and never where it lands — and a heading apiece would split
/// the one question a size on a node is asked, where does this sit, across four
/// places holding a quarter of the answer each. The Layers bar in Note is the
/// whole of it ([`StackBar`]), and what is left under each heading is the
/// reading it carries, the colours it wears and the switches that are its own.
///
/// The layers then run in stack order, from the center out: Core, the audio
/// ring a Ring gap outside it, the octave band a Ring gap outside that, and the
/// melody/bass marks past the band — the same order [`ViewConfig::rings`] lays
/// them down in, so the column reads down as the node reads outward. Shimmer is
/// last because it is the sweep the outermost layer carries rather than a layer
/// of its own.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    note_section(ui, &mut state.view, params);
    core_section(ui, &mut state.view);
    audio_section(ui, &mut state.view);
    octaves_section(ui, &mut state.view);
    melody_bass_section(ui, &mut state.view);
    shimmer_section(ui, &mut state.view);
}

/// Core: the mark at a sounding node's center. One continuous shape, sized by
/// the innermost handle of the Layers bar (0 = off, like Bloom) and morphed by
/// Solidity from a soft glow (0) to the classic solid orb (1), painted as one
/// calm disc that blends the sounding octaves' colors. One bar and no style row:
/// the paint is not a choice. Independent of the Octaves layer.
fn core_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Core");
    ui.add_enabled_ui(view.core_radius > 0.0, |ui| {
        ValueBar::new(&mut view.core_solidity, 0.0..=1.0, "Solidity")
            .show(ui)
            .on_hover_text(
                "0 = a soft glow, 1 = the classic solid orb; in \
                 between the disc fades in over its glow and its \
                 edge crisps",
            );
    });
}

/// Octaves: which octaves of the pitch class are sounding, shown as arcs of a
/// pitch axis that runs once round the node. Independent of the Core.
fn octaves_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Octaves");
    // Octaves, Center and the fringe are the axis; how thick the ring they are
    // drawn on is, and where it sits, is the Layers bar up in Note — the third
    // of its four handles, named MIDI there. The bar names layers by where each
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
    // stack's answer — a gap out from the audio ring, or from the core where
    // that ring is off — so the only thing left to say about it is how thick it
    // is.
    //
    // No Solidity bar and no Backdrop switch: the glyphs are always the crisp
    // classic shapes, and the silent octaves always stand in behind the
    // sounding ones — that backdrop is what completes the ring, so a lone
    // octave still reads as a whole note. How BRIGHT it stands is the At rest
    // section's Ground bar at the foot of the page, which is not this layer's
    // to own: the audio ring's silence and the grid's lines are the same grey.
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
    ui.add_enabled_ui(view.mark_melody || view.mark_bass, |ui| {
        // The Delay is about a mark that is DRAWN — when it arrives — so it is
        // gated on there being one. The strip's depth is the Layers bar's
        // outermost handle, and 0 is its off position, where `mark_extension`
        // returns no coverage (the Core section gates its own Solidity on a
        // radius of 0 the same way). The enclosing block already grays this on
        // both marks being off, so what it is gated on either way is
        // `marks_draw` — the same predicate `derive_scene` folds the shimmer
        // on, and the one the Shimmer section reads. Written as the depth alone
        // because that is the half this block adds; the pair is what has to
        // agree.
        ui.add_enabled_ui(view.mark_thickness > 0.0, |ui| {
            // How long an end has to be HELD before its mark answers. Here
            // rather than in with the note-wide settings at the head of the
            // pane, because it is about these two marks alone: the octave
            // sectors they continue and the core answer immediately whatever
            // this says.
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
    });
}

/// The patterns the marks' sheet can be laid in, for the Shimmer row.
///
/// A table beside the row rather than four arms written into it: a pattern is a
/// shape the light takes, and each one's description is a sentence about that
/// shape rather than about the marks, so it belongs next to the others it is
/// told apart from.
const SHIMMER_PATTERNS: &[(Pulse, &str, &str)] = &[
    (Pulse::Off, "Off", "Steady — no sweep on the marks"),
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

/// Shimmer: the shape the sheet crossing the marks takes, and how it is sized
/// and paced. The whole feature, under the one heading that names it.
///
/// The pattern is the first row and the four bars follow it, because the row
/// says WHETHER there is a sweep and the bars only say what it looks like. It
/// sits here rather than in Melody/bass, even though the marks are what the
/// sheet crosses: a "Shimmer" row under that heading with a "Shimmer" section
/// below it splits one feature across two places and spends its name twice.
///
/// The two gates are different questions and are written as two. The pattern
/// needs a mark to lay light on, so it follows [`ViewConfig::marks_draw`]
/// — the same predicate `derive_scene` folds `pulse_marks` off with, so a view
/// carrying a pattern with no end marked, or no mark width, is not
/// shimmering. The bars need light to shape, so they additionally follow
/// [`Pulse::sweeps`]: with the pattern Off they have nothing to move. Gating
/// the pattern on `sweeps()` too would strand it — Off could never be left.
fn shimmer_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Shimmer");
    ui.add_enabled_ui(view.marks_draw(), |ui| {
        // Off is its own option rather than a checkbox beside the row: one row
        // says both whether the marks shimmer and how.
        choice_row(ui, "Pattern", &mut view.pulse_marks, SHIMMER_PATTERNS);
        ui.add_enabled_ui(view.pulse_marks.sweeps(), |ui| {
            ValueBar::new(&mut view.shimmer_speed, 0.0..=6.0, "Speed")
                .show(ui)
                .on_hover_text(
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
            ValueBar::new(&mut view.shimmer_intensity, 0.0..=2.0, "Intensity")
                .show(ui)
                .on_hover_text(
                    "How far a peak stands above the trough beside it — the \
                     same ratio on dark notes as bright ones. 0 is no shimmer, \
                     1 the tuned depth.",
                );
            ValueBar::new(&mut view.shimmer_softness, 0.0..=1.0, "Softness")
                .show(ui)
                .on_hover_text(
                    "How gradually the light arrives: high fades peak into \
                     trough across the whole period, low narrows the peak to a \
                     hard band.",
                );
        });
    });
}

/// Audio ring: what the ring between the core and the octave band measures —
/// one reading of the analyzer's spectrum, or none.
///
/// Second of the layers, which is where it sits in the stack: a Ring gap out
/// from the Core above it, a Ring gap in from the octave Band below. It is the one
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
    // the ring off, the way it turns the core, the band and the marks off. An
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
        ValueBar::new(
            &mut view.spectral_ring_attack,
            0.0..=SPECTRAL_BALLISTICS_MAX,
            "Ring attack",
        )
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
        .display(|cents| {
            if cents < 10.0 { format!("{cents:.1}¢") } else { format!("{cents:.0}¢") }
        })
        .show(ui)
        .on_hover_text(
            "How much spectrum one wedge shows, in cents around that octave's \
             own pitch. At the top of the bar a wedge spans its whole octave and \
             the ring joins into one continuous reading. Spectrum reading only.",
        );
    });
}

/// Note: what the whole node does rather than any one layer of it — the time
/// it takes to arrive and leave, the curve it runs on, and the gap it clears
/// around itself.
///
/// One section rather than a heading apiece, because they are one idea: none
/// is about the core, the octave glyphs, the audio ring or the melody/bass
/// marks in particular, and all apply to whichever of those happen to be drawn.
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
    // lands: the four widths are one stack read outward from the node's center,
    // and the picture on the bar is the node's own cross-section (`StackBar`).
    // A whole-note setting rather than any layer's, which is why it is here and
    // not split four ways across the headings below — none of the four could be
    // read without the other three, since a layer's inner edge is a sum over
    // everything inside it.
    //
    // Directly above the Ring gap, because the two are one idea: the sizes are
    // the layers and that gap is the padding standing between them, the bar
    // draws both, and dragging it is visibly the stack opening up. The Octave
    // gap under it is not on this bar's axis at all, and is here anyway — see
    // there.
    StackBar::new(view)
        .show(ui)
        .on_hover_text(
            "Every layer of a node, from its center out: the core, the audio \
             ring, the octave band and the melody/bass strip, each named on its \
             own cell where there is room for it. Drag a handle to set the layer \
             inside it — 0 removes that layer and the ones outside close up. The \
             line is the node's edge, which only the marks may cross. \
             Double-click to restore.",
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
    // being a unit the four widths cannot have, and the ANGULAR one costs the
    // stack nothing — it cuts slices out of a ring already placed. What a node
    // could not say with one number is a ring standing well off its neighbour
    // while the slices stay tight, or the reverse.
    //
    // Read out as a PERCENTAGE of the node's radius, which is what quad uv 1.0
    // is (`scene.node_radius`, a quarter of the lattice spacing — the classic
    // core disc reaches 0.46 of it). That makes the whole stack a budget of
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
    // Here rather than with the sevens-layer controls, where a "Sevenths" name
    // misreads what it does: the clearance is cut by every DRAWING node on
    // every sheet, the home one included and at any sheet count, so it belongs
    // to the node and not to the depth axis. Named Clearance for that; the
    // `sevens_` field names stay — they are what saved projects spell.
    //
    // A percentage of the node's radius, as the two gaps above it are, this
    // being the same unit measured from the same place — a reach of 0.30 and a
    // gap of 0.052 are two shares of one length, and a page that read one of
    // them as a bare number would be saying they are different kinds of
    // quantity. Whole percents, which is the resolution the readout has always
    // had here.
    edge_bar(
        ui,
        (&mut view.sevens_gutter, &mut view.sevens_gutter_soft),
        0.5,
        "Clearance",
        {
            let fresh = ViewConfig::default();
            (fresh.sevens_gutter, fresh.sevens_gutter_soft)
        },
        |v| format!("{:.0}%", v * 100.0),
    )
    .on_hover_text(
        "The dark gap a node clears around its own shape, over grid lines and \
         back sheets — around each layer it is drawing, including an audio ring \
         with no note under it, as a share of the node's radius. Solid to the \
         inner handle, faded out by the outer. 0 draws none.",
    );
    // The glow, last, and in Note rather than beside the Core or with Bloom on
    // the Colors page. It is the whole node's, not a layer's: it is the core's
    // own skirt taken off the disc and grown over the node entire, so a node's
    // light is laid in the same octave colours its band and its disc are and
    // reaches this far past its outermost drawn edge.
    //
    // Not with Bloom, which it otherwise reads like, because the two are
    // different settings of different things. Bloom is one number over every
    // picture the plugin draws and it thresholds, so it is the bright end of
    // the gradient blowing out — colour, which is why it sits with the table
    // that makes it. This is a layer of a node, dialled where a node's other
    // layers are.
    //
    // A share of the node's radius, the unit the two gaps and the Clearance
    // above it read in, and measured from the same place: the reach is a
    // distance out from the node's edge exactly as the Clearance is.
    ValueBar::new(&mut view.glow_reach, 0.0..=GLOW_REACH_MAX, "Glow")
        .decimals(3)
        .display(|v| format!("{:.0}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "How far past its own edge a node's light spreads, as a share of \
             its radius. Neighbouring nodes' light melds where it overlaps. 0 \
             turns it off, and gives the core its own skirt back.",
        );
    // Grayed rather than hidden while the reach is 0, so the rows keep their
    // place and the numbers they are dialled to stay readable — the same
    // arrangement the audio ring's own settings use.
    ui.add_enabled_ui(view.glow_reach > 0.0, |ui| {
        ValueBar::new(&mut view.glow_strength, 0.0..=GLOW_STRENGTH_MAX, "Glow strength")
            .show(ui)
            .on_hover_text("How much light it lays down.");
        // The moat, beneath the light it stands off, and read out like the
        // Clearance it is made of: this bar widens that same knockout, layer by
        // layer, so the two are one hole dialled from two places and a person
        // comparing them is comparing shares of the node's radius.
        ValueBar::new(&mut view.glow_gap, 0.0..=GAP_MAX, "Glow gap")
            .decimals(3)
            .display(|v| format!("{:.1}%", v * 100.0))
            .show(ui)
            .on_hover_text(
                "The dark gap held between a node's crisp layers — its rings, \
                 its band, its marks and its disc — and the light around them, \
                 as a share of the node's radius. It is cut through the \
                 neighbours' light as well as the node's own. 0 lets the glow \
                 up to the edge of every layer.",
            );
    });
}
