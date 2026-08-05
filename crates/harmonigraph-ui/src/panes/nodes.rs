//! The Nodes pane: how a sounding note is drawn — its core mark, the octave
//! ring around it, the melody/bass marks on the outer held notes, and the
//! color, fade, halo and cleared gutter the whole node wears. Everything here
//! is the *played note*; the surrounding structure and overlays live in
//! [`super::scene`].

use super::{param_bar, section};
use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, OctaveStrip, RangeBar, SpectrumBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{Pulse, ViewConfig, MIN_EXTRA_SIZE, PITCH_CEIL, PITCH_FLOOR};

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
    ui.label("Band");
    RangeBar::new(&mut view.outer_inner, &mut view.outer_outer, 0.0..=1.0)
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
        // The sweep, on the only layer that runs one — the sheet takes both
        // rings and the octave slice each one points at. Off is its own option
        // rather than a checkbox beside the row: there is no separate bar the
        // pattern would otherwise gray out, so one row says both whether the
        // marks shimmer and how.
        //
        // Gated, because a ring has to be DRAWN for a sheet over it to mean
        // anything. Thickness 0 is the documented off position, where
        // `mark_ring` returns no coverage (the Core section gates its own
        // Solidity and Style on a radius of 0 the same way). The enclosing
        // block already grays this on both marks being off, so what the row is
        // gated on either way is `mark_rings_draw` — the same predicate
        // `derive_scene` folds the pattern on, and the one the Shimmer section
        // reads. Written as the thickness alone because that is the half this
        // row adds; the pair is what has to agree.
        ui.add_enabled_ui(view.mark_thickness > 0.0, |ui| {
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
                "How strong the light is: how far a peak pulls the layer \
                 toward white, and how far the layer dims between peaks, \
                 together. 0 draws the layer exactly as it is unshimmered; 1 \
                 is the tuned depth. Past 1 a peak reaches full white -- an \
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

/// The pitch gradient: the spectrum bar, and the three knobs that shape what
/// it draws. One column of full-width bars, like every other settings group —
/// which here is a budget as much as a habit, and the reason the spectrum is a
/// bar rather than the hue WHEEL a circular value naturally asks for.
///
/// Two measured limits rule the wheel out. A wheel large enough to grab is
/// 148pt — six bars of height — and this pane has none to spare: it already
/// runs past the column at the window this UI was dialled against (1512x886),
/// so a wheel would add 140pt on top of a list that has started to scroll,
/// and every knob under it would be that much further down. Setting the wheel
/// BESIDE these three knobs recovers the height, and breaks the other rule
/// instead — the one still pinned by a test: every bar in a
/// settings pane is the width of its column, so that dragging the column
/// narrower narrows all of them together
/// (`every_bar_in_a_settings_pane_is_the_width_of_the_pane`), and knobs beside
/// a wheel are 284pt of a 400pt column.
///
/// A bar costs one row and says the same thing — see [`SpectrumBar`] for how a
/// circle fits on one.
fn spectrum_group(ui: &mut egui::Ui, view: &mut ViewConfig) {
    button_row(ui, |ui| {
        ui.label("Spectrum");
        // Direction is a button rather than a gesture on the bar, and the bar
        // is the reason: it lays the arc out from its own start, so both
        // directions draw the same stretch of color in the same place and
        // there is nothing on it to drag the other way. Which is the right
        // division — a flip changes only the direction, and the arc it lands
        // on is exactly the arc it left, read backwards.
        if ui
            .button("Flip")
            .on_hover_text(
                "Run the spectrum the other way round the circle — the same \
                 colors, low and high swapped",
            )
            .clicked()
        {
            // The arithmetic lives on the gradient rather than here: what a
            // flip IS — the far end becoming the near one, so the arc keeps its
            // place on the circle — is a property of the gradient that the bar
            // beside this button previews and a test pins, and a second
            // spelling of it here is the one that would drift.
            view.pitch_gradient = view.pitch_gradient.flipped();
        }
    });
    SpectrumBar::new(&mut view.pitch_gradient).show(ui).on_hover_text(
        "How far round the color circle the pitch range walks, out of the \
         whole turn the bar stands for: the colors it takes fill from the \
         left, the ones it does not are dimmed. Drag the handle to widen or \
         narrow it, drag the track to turn the circle under it, double-click \
         to reset. The strip beneath is the same gradient in pitch order, low \
         note on the left.",
    );
    ValueBar::new(&mut view.pitch_gradient.lightness, 0.0..=100.0, "Brightness")
        .integer()
        .show(ui)
        .on_hover_text(
            "Brightness at the MIDDLE of the pitch range, in CIELab L*. The \
             ramp opens either side of it, so this stays the picture's overall \
             brightness at any ramp",
        );
    ValueBar::new(&mut view.pitch_gradient.lightness_ramp, -100.0..=100.0, "Brightness ramp")
        .integer()
        .show(ui)
        .on_hover_text(
            "How much brightness separates the bottom of the pitch range from \
             the top. 0 makes every note exactly as bright as every other and \
             leaves hue to carry the pitch alone; negative puts the bright end \
             at the bottom",
        );
    ValueBar::new(&mut view.pitch_gradient.chroma, 0.0..=1.0, "Chroma")
        .display(|v| format!("{:.0}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "How much color, as a share of the most this brightness and hue \
             can hold. 100% is as vivid as the screen goes without distorting \
             the color; 0 is grey",
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
    ui.label("Color range");
    super::param_range_bar(
        ui,
        params,
        ParamKey::DarkestPitch,
        ParamKey::BrightestPitch,
        0.0..=120.0,
        crate::COLOR_RANGE_MIN_SPAN,
        pitch_readout,
    )
    .on_hover_text(
        "The pitch span the color gradient covers: the low end takes the \
         gradient's first color, the high end its last. Drag either end, or \
         drag between them to slide the whole range.",
    );
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a released note keeps fading — the pitch class core, \
         the octave glyphs, and the melody/bass marks together. 0 cuts \
         notes off the moment they're released",
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
    ValueBar::new(&mut view.sevens_gutter, 0.0..=0.5, "Gutter")
        .show(ui)
        .on_hover_text(
            "The dark gap a sounding node clears around itself, so it reads \
             over whatever it crosses rather than needing room of its own: \
             the grid lines under it, and the sheets behind it once the \
             lattice has depth. Measured past the node's own edge, and the \
             same width on screen whatever size the node draws at. 0 draws \
             none",
        );
    ValueBar::new(&mut view.sevens_gutter_soft, 0.0..=0.5, "Gutter fade")
        .show(ui)
        .on_hover_text(
            "How gradually the gap ends, independent of how wide it is. \
             0 is a hard edge; past the gutter's own width it softens \
             outward rather than eating into the node",
        );
}
