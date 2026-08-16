//! The Display tab's Nodes section: how a sounding note is drawn — its core
//! mark, the octave ring around it, the melody/bass marks on the outer held
//! notes, and the fade and cleared gutter the whole node wears. Everything
//! here is a layer of the *played note*, or the whole of one.
//!
//! The pitch gradient and Bloom are NOT here, though a node wears both: the
//! Analyzer's ribbons read the one and its piano roll blooms off the other, so
//! filing them under a node's own layers would name them for the narrower of
//! the two pictures they paint. They are [`super::color`]. The text a node
//! carries is [`super::labels`], kept out for the same reason in miniature — a
//! label rides a hovered and a remembered node as readily as a sounding one.

use super::{edge_bar, param_bar, section};
use crate::params::{seconds, ParamBackend, ParamKey};
use crate::widgets::{button_row, choice_row, OctaveStrip, RangeBar, ValueBar};
use crate::SharedState;
use harmonigraph_scene::{
    Pulse, SpectralReading, ViewConfig, MARK_DELAY_MAX, MIN_EXTRA_SIZE, PITCH_CEIL, PITCH_FLOOR,
    SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN, SPECTRAL_RING_MIN_SPAN, SPECTRAL_WIDTH_MAX,
    SPECTRAL_WIDTH_MIN,
};

/// The sounding-note controls: what the audio ring measures, then the whole
/// note — the time it takes to arrive and leave, and the gutter it clears —
/// then each layer of it reading outward from the center, and last the sweep
/// the outermost layer can be set to run.
///
/// Whole-note before the layers, because those settings are the ones reached
/// for most and because none of them belongs to a layer. Ordering them after
/// Core, Octaves and the marks would read more consistently outward, and would
/// put the section's most-used controls under the ones reached for least.
/// Reading outward is what orders the rest. Audio comes above even those,
/// being the one control that adds a layer rather than sizing one.
pub(super) fn nodes_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    audio_section(ui, &mut state.view);
    note_section(ui, &mut state.view, params);
    core_section(ui, &mut state.view);
    octaves_section(ui, &mut state.view);
    melody_bass_section(ui, &mut state.view);
    shimmer_section(ui, &mut state.view);
}

/// Core: the mark at a sounding node's center. One continuous shape sized by
/// the radius (0 = off, like Bloom) and morphed by Solidity from a soft glow
/// (0) to the classic solid orb (1), painted as one calm disc that blends the
/// sounding octaves' colors. Two bars and no style row: the paint is not a
/// choice. Independent of the Octaves layer.
fn core_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Core");
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
        .display(super::pitch_readout)
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
    // between the band and the melody/bass marks.
    ValueBar::new(&mut view.outer_gap, 0.0..=0.4, "Gap")
        .show(ui)
        .on_hover_text(
            "Padding inside the octave layer: between one octave \
             and the next, and between the band and the \
             melody/bass marks. 0 closes the octaves into a solid \
             annulus and seats the marks against it",
        );
    // No Solidity and no Backdrop bar: both are fixed at 1. The glyphs are
    // always the crisp classic shapes, and the silent octaves always ghost
    // in behind the sounding ones — that backdrop is what completes the
    // ring, so a lone octave still reads as a whole note.
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
    // A mark is the marked octave's own slice continued outward, standing off
    // the band by Gap -- the same padding one sector stands off the next, so
    // it reads as that indicator's own piece rather than as a ring around
    // everything.
    ui.add_enabled_ui(view.mark_melody || view.mark_bass, |ui| {
        ValueBar::new(&mut view.mark_thickness, 0.0..=0.3, "Mark depth")
            .show(ui)
            .on_hover_text(
                "How far a mark reaches past the octave band, in the same \
                 units as the band radii and Gap. 0 turns the marks off; \
                 a band dialled right out with a wide Gap leaves less room \
                 than this asks for, and the mark stops at the node's edge",
            );
        // The Delay is about a mark that is DRAWN — when it arrives — so it is
        // gated on there being one. A depth of 0 is the documented off position,
        // where `mark_extension` returns no coverage (the Core section gates its
        // own Solidity on a radius of 0 the same way). The enclosing block already
        // grays this on both marks being off, so what it is gated on either way
        // is `marks_draw` — the same predicate `derive_scene` folds the
        // shimmer on, and the one the Shimmer section reads. Written as the
        // depth alone because that is the half this block adds; the pair is
        // what has to agree.
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
                    "How long a note has to stay the melody or the bass \
                     before its mark starts fading in. A note that loses the \
                     end again first is never marked at all, which is what \
                     keeps fast playing from flickering marks around the \
                     band. 0 marks every note the moment it takes an end",
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
/// carrying a pattern with no end marked, or no mark depth, is not
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
                    "How fast the sheet travels, in lattice units a second -- so \
                     the plugin window and an exported video sweep at the same \
                     rate over the same nodes, whatever size either is drawn at. \
                     Which way it travels is the pattern's own. 0 freezes the \
                     sheet where it stands",
                );
            // Eased, because the range is three orders wide and the useful
            // settings are not spread evenly over it: the tight end is a
            // different picture every few hundredths (0.05 to 0.1 halves the
            // periods on a node), where the wide end changes little between 8
            // and 15. Geometric travel gives each end the same share of the bar.
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
                    "How strong the light is: how far a peak stands above the \
                     trough beside it. The same RATIO wherever it lands, so a peak \
                     reads as strongly on a high note's bright color as on a low \
                     note's dark one. 0 draws the layer exactly as it is \
                     unshimmered; 1 is the tuned depth. Darker notes hold their \
                     color between peaks; the brightest have no room to swing up, \
                     so their peaks pale toward white and their troughs darken \
                     instead -- an indicator under a strong one says an octave \
                     sounds without saying which",
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
    });
}

/// Audio: what the ring inside the octave band measures — one reading of the
/// analyzer's spectrum, or none.
///
/// First in the pane, and above even the note-wide settings, because it is the
/// one control here that adds a LAYER where the rest size and colour the ones
/// already there. It is a plain heading for that reason too — it is the top of
/// the section body, where `section`'s leading rule would sit directly under
/// the Display pane's own Nodes header. (The View and Analyzer section bodies,
/// and the Tuning and System panes, open the same way.)
///
/// One choice row and not two boxes, because there is one indicator here and
/// two ways to fill it: both readings answer "what is sounding at this node",
/// both draw in the same annulus in the same colours, and neither touches the
/// MIDI picture. Two boxes would have to say what BOTH ticked means, and the
/// only honest answer — one drawn over the other in the same ring — is a
/// picture nobody can read.
///
/// Each reading's own setting sits under the row, greyed when the other is
/// chosen: Width is the fold's kernel and Range is the spectrum's zoom, and
/// neither means anything to the other. Both are shown either way rather than
/// swapped in and out, so the section keeps its height and the bars keep their
/// place as the row is clicked along.
fn audio_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    ui.heading("Audio");
    choice_row(
        ui,
        "Ring",
        &mut view.spectral_reading,
        &[
            (
                SpectralReading::Off,
                "Off",
                "No ring: the lattice is the MIDI picture, whole",
            ),
            (
                SpectralReading::Fold,
                "Fold",
                "Each wedge one reading, taken at that octave's own pitch: how \
                 much is sounding THERE, folded onto the node the way the \
                 lattice already folds a note. A partial sits at an exact \
                 ratio of its fundamental, so a timbre lights a constellation \
                 around its own node — a fifth up for the 3rd harmonic, a \
                 third up for the 5th, the sevenths axis for the 7th — and \
                 each node's wedges say which octaves those partials are in. \
                 The reading to look at a screenful of nodes with",
            ),
            (
                SpectralReading::Spectrum,
                "Spectrum",
                "Each wedge a window of the raw spectrum, bent into its arc — \
                 a segment of the spiral spectrogram. Angle across the wedge \
                 is a pitch window around that octave, so a partial dead on \
                 the node paints down the middle and one a comma sharp paints \
                 to the clockwise side: this one reads a DETUNING, which a \
                 number per octave cannot say. The reading to look at ONE node \
                 with",
            ),
        ],
    );
    // Where the ring sits, as one control over its two radii — the same shape
    // as the octave band's own Band bar, because it is the same question about
    // a second annulus. Fresh it lands in the gap the core and the octave band
    // leave; a dialled-up core or a band pulled inward closes that gap, and
    // this is what moves the ring out of the way. Both readings, since both
    // draw in this one annulus.
    ui.add_enabled_ui(view.spectral_reading.draws(), |ui| {
        RangeBar::new(
            &mut view.spectral_ring_inner,
            &mut view.spectral_ring_outer,
            0.0..=1.0,
            "Ring",
        )
        .min_span(SPECTRAL_RING_MIN_SPAN)
        .show(ui)
        .on_hover_text(
            "The audio ring's inner and outer radius, in the same units as the \
             octave Band. Drag between the handles to move the whole ring in \
             or out; fresh it sits in the clear space between the core and the \
             octave band, with a gap either side so the three read as separate \
             layers",
        );
    });
    // The FOLD's kernel, and so inert under Spectrum rather than merely
    // without audio: the spectrum reading shows a whole window of pitch per
    // wedge, and a kernel there would blur the one axis the window exists to
    // resolve. Its own setting is the Range bar below.
    ui.add_enabled_ui(view.spectral_reading == SpectralReading::Fold, |ui| {
        ValueBar::new(
            &mut view.spectral_width,
            SPECTRAL_WIDTH_MIN..=SPECTRAL_WIDTH_MAX,
            "Width",
        )
        .display(|cents| format!("{cents:.0}¢"))
        .show(ui)
        .on_hover_text(
            "How far off an octave's own pitch a partial may sit and still \
             light its wedge, in cents. A weight and not a cutoff: distance \
             reads as dimness, so a detuned partial fades rather than \
             switching off and vibrato breathes instead of flickering. Narrow \
             is right for just intonation, where partials land dead on the \
             nodes; equal-tempered material wants it wider, a tempered third's \
             5th harmonic sitting 13.7 ¢ off its node and a 7th harmonic 31 ¢. \
             The Fold reading's alone — Spectrum does not fold",
        );
    });
    // The SPECTRUM reading's zoom, under the Width it stands opposite: how much
    // pitch a wedge shows, where Width is how much of it counts as the node's.
    ui.add_enabled_ui(view.spectral_reading == SpectralReading::Spectrum, |ui| {
        ValueBar::new(
            &mut view.spectral_ring_range,
            SPECTRAL_RANGE_MIN..=SPECTRAL_RANGE_MAX,
            "Range",
        )
        // A decimal below ten cents: the bar's floor is 0.5¢, and "{:.0}"
        // would read it out as the zero the floor exists to forbid.
        .display(|cents| {
            if cents < 10.0 { format!("{cents:.1}¢") } else { format!("{cents:.0}¢") }
        })
        .show(ui)
        .on_hover_text(
            "How much of the spectrum one wedge of the audio ring shows, in \
             cents, centred on that octave's own pitch. Narrow zooms in on the \
             node's own neighbourhood, where a partial's detuning is a \
             readable fraction of the wedge; at the top of the bar a wedge \
             spans exactly its octave, so neighbouring wedges meet at the \
             pitch they share and the ring becomes one continuous reading — \
             the same picture on every node, turned. The Spectrum reading's \
             alone — a folded wedge is one number and has no window to size",
        );
    });
}

/// Note: what the whole node does rather than any one layer of it — the time
/// it takes to arrive and leave, the curve it runs on, and the gap it clears
/// around itself.
///
/// One section rather than a heading apiece, because they are one idea: none
/// is about the core, the octave glyphs or the melody/bass marks in
/// particular, and all apply to whichever of those happen to be drawn. Fade
/// especially — one time for the node rather than one per layer, so a release
/// reads as a single gesture instead of pieces of the node going dark at
/// different moments.
fn note_section(ui: &mut egui::Ui, view: &mut ViewConfig, params: &dyn ParamBackend) {
    section(ui, "Note");
    // The note's timing and the curve it runs on, in that order. Fade is an
    // automatable param and Fade curve a view setting, so the two are stored apart
    // (`ViewConfig::envelope` is where they are put back together); the pane
    // is where they have to LOOK like the one setting they are.
    param_bar(ui, params, ParamKey::Fade).on_hover_text(
        "Seconds a note takes to arrive, and to leave once it's released — \
         the pitch class core, its glow, the octave glyphs and the \
         melody/bass marks together. A short note is not dimmed by it: the \
         node reaches full brightness whatever the key did, and starts \
         leaving from there. (A mark waits out the Delay above first, so it \
         still comes in graded on notes shorter than the two put together.) \
         0 switches the note on and off outright",
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
            "The curve both ends of the Fade run on, drawn across the bar as \
             an arrival — a release is the same line upside down. 0 is a \
             straight line — the same change every frame. Higher leaves and \
             arrives fast and settles slowly, the way a struck note decays; at \
             the top most of the travel is over in the first quarter of the \
             time. The trail keeps its own straight fade whatever this says",
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
        {
            let fresh = ViewConfig::default();
            (fresh.sevens_gutter, fresh.sevens_gutter_soft)
        },
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
