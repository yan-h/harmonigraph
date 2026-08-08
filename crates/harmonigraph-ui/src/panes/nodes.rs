//! The Nodes pane: how a sounding note is drawn — its core mark, the octave
//! ring around it, the melody/bass marks on the outer held notes, and the
//! color, fade, halo and cleared gutter the whole node wears. Everything here
//! is the *played note*; the surrounding structure and overlays live in
//! [`super::scene`].

use super::{edge_bar, param_bar, section};
use crate::params::{seconds, ParamBackend, ParamKey};
use crate::widgets::{
    button_row, choice_row, OctaveStrip, RangeBar, SpectrumBar, SpreadBar, ValueBar,
};
use crate::SharedState;
use harmonigraph_scene::{
    Pulse, ViewConfig, MARK_DELAY_MAX, MIN_EXTRA_SIZE, PITCH_CEIL, PITCH_FLOOR,
};

/// The sounding-note controls, top to bottom as the note reads outward: the
/// Core mark at its center, the Octaves ring around it, the melody/bass marks
/// on the outer notes, the sweep those marks can be set to run, and then the
/// settings that are not about any one of those layers but about all of them
/// at once. Scrolls so the full list is reachable in a short pane.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            core_section(ui, &mut state.view);
            octaves_section(ui, &mut state.view);
            melody_bass_section(ui, &mut state.view);
            shimmer_section(ui, &mut state.view);
            every_layer_section(ui, &mut state.view, params);
        });
}

/// Core: the mark at a sounding node's center. One continuous shape sized by
/// the radius (0 = off, like Bloom) and morphed by Solidity from a soft glow
/// (0) to the classic solid orb (1), painted as one calm disc that blends the
/// sounding octaves' colors. Two bars and no style row: the paint is not a
/// choice. Independent of the Octaves layer.
fn core_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    ui.heading("Core");
    ValueBar::new(&mut view.core_radius, 0.0..=0.9, "Radius")
        .show(ui)
        .on_hover_text(
            "Core size (disc and glow together); 0 turns it off, \
             0.46 is the classic disc",
        );
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
    // Octaves, Center and the fringe are the axis; Band and Gap below are
    // where it is drawn.
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
        "How many octaves one turn of a node covers, out of the eleven the \
         strip's slots stand for. Drag BETWEEN the handles to set how many \
         are drawn full size, OUTSIDE them to add small extra octaves at each \
         end. Every cell is one octave, drawn as tall as its share of the ring \
         against the widest octave on the wheel; notes past either end light \
         the outermost indicator on their side rather than vanishing",
    );
    // Whole semitones, because that is the step the wheel can act on and what
    // the readout can name.
    ValueBar::new(&mut view.octave_center, PITCH_FLOOR..=PITCH_CEIL, "Center")
        .integer()
        .display(pitch_readout)
        .show(ui)
        .on_hover_text(
            "The pitch at the TOP of the wheel — on every node, whatever its \
             pitch class. Each node draws the octaves of itself nearest this \
             one, and its ring is turned so they land on their own pitches: \
             the half octave below turns left, the half above turns right",
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
                "How wide one extra octave is, as a fraction of an even slice \
                 — the turn divided by every octave on the wheel. Under 1 an \
                 extra is always narrower than a full-size octave, however \
                 many of either there are; 1 is an even wheel, and 0.1 leaves \
                 the extras a tenth of an even slice",
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
                "How the extras GRADE. At 0 they are all the size above, a \
                 flat fringe meeting the full-size octaves in one step. \
                 Dragging right lifts the inner ones toward full size — which \
                 the full-size octaves pay for — until at 1 the whole wheel is \
                 one ramp from the outermost extra inward. The outermost one \
                 never moves",
            );
    });
    // No on/off: the layer is what says which octaves are sounding, which
    // is the node's whole outer half, and Band already reaches every size
    // from a hairline to the quad edge.
    //
    // Inner and outer radius are one control: the band is the ring between
    // them. Drag either edge, or drag between to slide the ring at a fixed
    // width; the min span keeps it from collapsing.
    RangeBar::new(&mut view.outer_inner, &mut view.outer_outer, 0.0..=1.0, "Band")
        .min_span(0.05)
        .show(ui)
        .on_hover_text(
            "The octave band's inner and outer radius. Inner 0 reaches the \
             node center (pie wedges); drag between the handles to move the \
             whole ring in or out.",
        );
    // One padding for the whole layer: between sectors, and
    // between the band and the melody/bass rings.
    ValueBar::new(&mut view.outer_gap, 0.0..=0.4, "Gap")
        .show(ui)
        .on_hover_text(
            "Padding inside the octave layer: between one octave \
             and the next, and between the band and the \
             melody/bass rings. 0 closes the octaves into a solid \
             annulus and seats the rings against it. Wide values \
             push the bass ring in toward the core -- raise Band \
             inner to make room",
        );
    // No Solidity and no Backdrop bar: both are fixed at 1. The glyphs are
    // always the crisp classic shapes, and the silent octaves always ghost
    // in behind the sounding ones — that backdrop is what completes the
    // ring, so a lone octave still reads as a whole note.
    //
    // No Shimmer row either: the glyphs are what says which octaves sound, and
    // a sheet laid over that reading costs it — so the sweep belongs to the
    // marks, which carry no such reading. What reaches this layer is the mark
    // sheet crossing the one slice each ring points at, from the row in
    // Melody/bass below.
}

/// Melody / bass: mark the outer held notes so a chord's top and bottom line
/// read at a glance.
fn melody_bass_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Melody / bass");
    // Two boxes, not a four-way row: the marks are independent, they are
    // told apart by radius rather than by hue, and a note that is at once
    // the highest and the lowest -- a lone held note, or a chord whose top
    // and bottom share a pitch class -- simply gets both.
    //
    // Side by side, because they are that pair: two ends of one idea, both
    // named in the heading above them, and short enough that a column of two
    // spends a row on saying nothing. A `button_row` rather than a bare
    // `horizontal` so a narrow pane wraps them instead of running Bass off
    // the edge.
    button_row(ui, |ui| {
        ui.checkbox(&mut view.mark_melody, "Melody")
            .on_hover_text("Ring the highest held note, just inside the octave band");
        ui.checkbox(&mut view.mark_bass, "Bass")
            .on_hover_text("Ring the lowest held note, just outside the octave band");
    });
    // The marks are full rings bracketing the octave band (melody
    // inside, bass outside), each slit either side of the octave
    // responsible so that stretch reads as its own piece.
    ui.add_enabled_ui(view.mark_melody || view.mark_bass, |ui| {
        ValueBar::new(&mut view.mark_thickness, 0.0..=0.3, "Thickness")
            .show(ui)
            .on_hover_text(
                "How thick both mark rings are, in the same units as \
                 the band radii and Gap. 0 turns the rings off; thick \
                 values grow the bass ring in over the core, so raise \
                 Band inner to make room",
            );
        // Both of the below are about a ring that is DRAWN — when it arrives
        // and what crosses it — so both are gated on there being one.
        // Thickness 0 is the documented off position, where `mark_ring`
        // returns no coverage (the Core section gates its own Solidity and
        // Style on a radius of 0 the same way). The enclosing block already
        // grays these on both marks being off, so what they are gated on
        // either way is `mark_rings_draw` — the same predicate `derive_scene`
        // folds the shimmer on, and the one the Shimmer section reads.
        // Written as the thickness alone because that is the half this block
        // adds; the pair is what has to agree.
        ui.add_enabled_ui(view.mark_thickness > 0.0, |ui| {
            // How long an end has to be HELD before its ring answers. Here
            // rather than in with the note-wide settings at the foot of the
            // pane, because it is about these two rings alone: the octave
            // sectors under them and the core answer immediately whatever
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
                    "How long a note has to stay the melody or the bass \
                     before its ring starts fading in. A note that loses the \
                     end again first never rings at all, which is what keeps \
                     fast playing from flickering rings across the band. 0 \
                     rings every note the moment it takes an end",
                );
            // The sweep, on the only layer that runs one — the sheet takes
            // both rings and the octave slice each one points at. Off is its
            // own option rather than a checkbox beside the row: there is no
            // separate bar the pattern would otherwise gray out, so one row
            // says both whether the marks shimmer and how.
            choice_row(ui, "Shimmer", &mut view.pulse_marks, SHIMMER_PATTERNS);
        });
    });
}

/// The patterns the mark rings' sheet can be laid in, for the row above.
///
/// A table beside the row rather than six arms written into it: a pattern is a
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
    (
        Pulse::Weave,
        "Weave",
        "The same crossed gratings as Checker with the brighter taken: a \
         lattice of light LINES rather than cells, brightest where two cross",
    ),
    (
        Pulse::Rings,
        "Rings",
        "Concentric rings travelling outward from the lattice's origin — the \
         one pattern with a center, so the light has somewhere it comes from",
    ),
];

/// Shimmer: how the sheet the Melody/bass row lays is sized and paced.
///
/// Its own section rather than four more bars under that row, because these
/// size ONE sheet crossing the whole lattice while the row picks its shape,
/// and because the row sits inside the marks' own gating where a bar about the
/// light itself does not belong. It stays at the foot of the pane for that
/// reason.
///
/// Grayed until something is actually shimmering, on the same grounds every
/// other gate in this pane uses: the bars do nothing at all with the pattern
/// Off.
///
/// "Actually" is the load-bearing word, and it is why this is not just a mode
/// check: `derive_scene` folds `pulse_marks` off with the ring layer itself
/// ([`ViewConfig::mark_rings_draw`]), so a view carrying a pattern with no end
/// marked, or no ring thickness, is not shimmering — and these bars would
/// otherwise be live with nothing to move.
fn shimmer_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Shimmer");
    let shimmering = view.pulse_marks.sweeps() && view.mark_rings_draw();
    ui.add_enabled_ui(shimmering, |ui| {
        ValueBar::new(&mut view.shimmer_speed, 0.0..=6.0, "Speed")
            .show(ui)
            .on_hover_text(
                "How fast the sheet travels, in lattice units a second -- so \
                 the plugin window and an exported video sweep at the same \
                 rate over the same nodes, whatever size either is drawn at. \
                 Which way it travels is the pattern's own. 0 freezes the \
                 sheet where it stands",
            );
        // Eased, because the range is three orders wide and the useful
        // settings are not spread evenly over it: the tight end is a
        // different picture every few hundredths (0.05 to 0.1 halves the
        // periods on a node), where the wide end changes little between 8 and
        // 15. Geometric travel gives each end the same share of the bar.
        ValueBar::new(&mut view.shimmer_width, 0.05..=15.0, "Width")
            .eased(true)
            .show(ui)
            .on_hover_text(
                "How wide the pattern is, in lattice units from one bright \
                 peak to the next -- about five nodes at the default spacing. \
                 Wider peaks are also further apart: it is one shape, sized. \
                 Around one node to a period the light reads as alternating \
                 nodes rather than as a sweep; below that several periods \
                 cross each node at once and it becomes a texture on them",
            );
        ValueBar::new(&mut view.shimmer_intensity, 0.0..=2.0, "Intensity")
            .show(ui)
            .on_hover_text(
                "How strong the light is: how much brightness a peak adds to \
                 the layer, and how far the layer dims between peaks, \
                 together. Near enough the same amount wherever it lands, so a \
                 peak reads about as strongly on a low note's dark color as on \
                 a high note's bright one. 0 draws the layer exactly as it is \
                 unshimmered; 1 is the tuned depth. From about 0.4 up a peak \
                 starts washing out the most saturated colors first -- an \
                 indicator under it says an octave sounds without saying which",
            );
        ValueBar::new(&mut view.shimmer_softness, 0.0..=1.0, "Softness")
            .show(ui)
            .on_hover_text(
                "How gradually the light arrives, where Intensity is how much \
                 of it there is. High, the brightest part fades into the \
                 clearest across the whole period and nothing is at rest; low, \
                 the peak narrows to a hard band with a dark field around it, \
                 which at a tight Width reads as stripes laid on the layer \
                 rather than as light crossing it",
            );
    });
}

/// A MIDI note as a key name and octave — "C1", "C8" — so a range's ends read
/// as pitches rather than bare numbers. Shared by the octave Center and the
/// color range.
///
/// It ROUNDS, which is exact for the octave Center (its bar lands on whole
/// semitones) and a reading for the color range (whose ends are a continuous
/// gradient, where a tenth of a semitone changes nothing anyone can see). A
/// caller wanting finer steps than a semitone needs its own readout, not a
/// looser one here: this one would then name two visibly different settings
/// the same note.
fn pitch_readout(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = super::KEY_NAMES[n.rem_euclid(12) as usize];
    format!("{name}{}", harmonigraph_core::notes::display_octave_of(n))
}

/// The pitch gradient on three bars, one per stretch: the arc on the spectrum
/// bar, the brightness pair on one of its own, and the chroma pair on another.
/// Each bar is a picture of what its numbers COMPOSE rather than a row per
/// number, which is what keeps a six-number gradient down to three rows.
///
/// One column of full-width bars, like every other settings group —
/// which here is a budget as much as a habit, and the reason the spectrum is a
/// bar rather than the hue WHEEL a circular value naturally asks for.
///
/// Two measured limits rule the wheel out. A wheel large enough to grab is
/// 148pt — six bars of height — and this pane has none to spare: it already
/// runs past the column at the window this UI was dialled against (1512x886),
/// so a wheel would add 140pt on top of a list that has started to scroll,
/// and every knob under it would be that much further down. Setting the wheel
/// BESIDE the bars below recovers the height, and breaks the other rule
/// instead — the one still pinned by a test: a bar in a settings pane is the
/// width of its column, so that dragging the column narrower narrows all of
/// them together (`every_bar_in_a_settings_pane_is_the_width_of_the_pane`).
/// The spectrum bar is the one exception the test allows, and the size of the
/// exception is the point: it gives up 20pt of a 400pt column to the flip
/// button and narrows with the column for the rest, where knobs beside a wheel
/// would be 284pt of 400 and would not.
///
/// A bar costs one row and says the same thing — see [`SpectrumBar`] for how a
/// circle fits on one, and for why the flip and the arc share that row rather
/// than taking two. The same budget is why the bar carries no name of its own,
/// and it is the one bar in the dock that does not carry one: the section
/// heading above it names the group, the spread bars below it are the rest of
/// the gradient, and the only rainbow in the pane needs no caption to be found.
/// What a name would say the tooltip says at more length.
///
/// It is the odd one out on purpose rather than by omission. Every other bar
/// here writes its name along its own track, over a well or an accent fill
/// that a word can be read on; this one's track is saturated color end to end
/// at every setting, so a name laid on it is legible at some gradients and not
/// at others. A label on a row of its own is what that would cost, and the row
/// is the thing this control does not have.
fn spectrum_group(ui: &mut egui::Ui, view: &mut ViewConfig) {
    SpectrumBar::new(&mut view.pitch_gradient).show(ui).on_hover_text(
        "The pitch->color spectrum: how far round the color circle the pitch \
         range walks, out of the whole turn the bar stands for. The colors it \
         takes fill from the left, low note first; the ones it does not are \
         dimmed. Drag the handle to widen or narrow it, drag the track to turn \
         the circle under it, double-click to reset. The strip beneath is the \
         same gradient at full width, in pitch order; the button at the left \
         runs it the other way round the circle.",
    );
    SpreadBar::brightness(&mut view.pitch_gradient).show(ui).on_hover_text(
        "The stretch of brightness the pitch range spends, in CIELab L*: the \
         two numbers are the bottom of the pitch range and the top, in that \
         order, so a picture with its bright end at the bottom reads out \
         backwards. Drag either end to move it, drag between them to slide the \
         whole stretch brighter or darker, drag one end past the other to swap \
         which end is bright, double-click to reset. Closing the two together \
         makes every note exactly as bright as every other and leaves hue to \
         carry the pitch alone.",
    );
    SpreadBar::chroma(&mut view.pitch_gradient).show(ui).on_hover_text(
        "The stretch of color the pitch range spends, each end as a share of \
         the most that note's own brightness and hue can hold — 100% is as \
         vivid as the screen goes without distorting the color, 0 is grey. The \
         two numbers are the bottom of the pitch range and the top, in that \
         order, so a picture with its vivid end at the bottom reads out \
         backwards. Drag either end to move it, drag between them to slide the \
         whole stretch, drag one end past the other to swap which end is \
         vivid, double-click to reset. Closing the two together gives every \
         note the same share of the color available to it.",
    );
}

/// What every layer of the node shares: the pitch->color gradient it is
/// tinted through, the time it takes to fade on release, the halo it carries
/// while lit, and the gap it clears around the whole of itself.
///
/// One section rather than a heading apiece, because they are one idea — none
/// of them is about the core, the octave glyphs or the melody/bass rings in
/// particular, and all of them apply to whichever of those happen to be drawn.
/// Fade especially: one time for the node rather than one per layer, so a
/// release reads as a single gesture instead of pieces of the node going dark
/// at different moments.
fn every_layer_section(
    ui: &mut egui::Ui,
    view: &mut ViewConfig,
    params: &dyn ParamBackend,
) {
    section(ui, "Every layer");
    // The gradient above the range because it is the coarser of the two: it
    // says what the colors ARE, the range says which pitches they are spread
    // over. Both feed the one table every pitch-colored shape reads, so a
    // change here repaints the discs, the octave glyphs, the trail and the
    // piano roll together.
    spectrum_group(ui, view);
    super::param_range_bar(
        ui,
        params,
        (ParamKey::DarkestPitch, ParamKey::BrightestPitch),
        0.0..=120.0,
        crate::COLOR_RANGE_MIN_SPAN,
        "Color range",
        pitch_readout,
    )
    .on_hover_text(
        "The pitch span the color gradient covers: the low end takes the \
         gradient's first color, the high end its last. Drag either end, or \
         drag between them to slide the whole range.",
    );
    // The note's timing and the curve it runs on, in that order. Fade is an
    // automatable param and Shape a view setting, so the two are stored apart
    // (`ViewConfig::envelope` is where they are put back together); the pane
    // is where they have to LOOK like the one setting they are.
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a note takes to arrive, and to leave once it's released — \
         the pitch class core, its glow, the octave glyphs and the \
         melody/bass rings together. A short note is not dimmed by it: the \
         node reaches full brightness whatever the key did, and starts \
         leaving from there. (A ring waits out the Delay above first, so it \
         still comes in graded on notes shorter than the two put together.) \
         0 switches the note on and off outright",
    );
    // Linear like every bar around it, and for the same reason: the whole
    // range is one unit, so every hundredth of it — the readout's own
    // resolution — is already a couple of pixels of travel, and there is no
    // fine end for an ease to rescue. The one bar in the group that is NOT a
    // duration, hence no seconds on the readout — it is the shape the Fade
    // above it is drawn with.
    ValueBar::new(&mut view.fade_shape, 0.0..=1.0, "Shape")
        .show(ui)
        .on_hover_text(
            "The curve both ends of the Fade run on. 0 is a straight \
             line — the same change every frame. Higher leaves and arrives \
             fast and settles slowly, the way a struck note decays; at the \
             top most of the travel is over in the first quarter of the \
             time. The trail keeps its own straight fade whatever this says",
        );
    // 0 = off (the renderer skips the whole post-process chain), so the bar
    // doubles as the toggle.
    ValueBar::new(&mut view.bloom_strength, 0.0..=1.5, "Bloom")
        .show(ui)
        .on_hover_text(
            "Soft halo around bright notes; 0 turns the post-process off",
        );
    // Here rather than with the sevens-layer controls, where a "Sevenths" name
    // misreads what it does: the gutter is cleared by every sounding node on
    // every sheet, the home one included and at any sevenths extent, so it
    // belongs to the node and not to the depth axis. The `sevens_` field names
    // stay — they are what saved projects spell.
    edge_bar(
        ui,
        (&mut view.sevens_gutter, &mut view.sevens_gutter_soft),
        0.5,
        "Gutter",
        |v| format!("{v:.2}"),
    )
    .on_hover_text(
        "The dark gap a sounding node clears around itself, so it reads over \
         whatever it crosses rather than needing room of its own: the grid \
         lines under it, and the sheets behind it once the lattice has depth. \
         Measured past the node's own edge, and the same width on screen \
         whatever size the node draws at. 0 draws none.\n\nThe bar is the gap \
         itself, read outward from the node: solid to the first handle, then \
         fading, and gone by the second. Drag between them to widen the gap \
         without softening it, the inner handle to soften it (together they \
         close for a hard edge), the outer one to reach further out from where \
         it already starts to soften",
    );
}
