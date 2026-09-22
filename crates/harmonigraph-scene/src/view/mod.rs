//! [`ViewConfig`] — the persisted, non-automatable visual settings — plus the
//! per-frame [`FrameParams`] mirror of the host-automatable appearance
//! parameters.
//!
//! The pieces that are their own shapes rather than fields of the settings
//! struct live in the submodules beside this one and are re-exported here, so
//! every path into them is the one it always was.

mod animation;
mod drawn_window;
mod frame_params;
mod glow_curve;
mod ring_stack;
mod windowing;

pub use animation::{AnimationOrder, NoteAnimationConfig};
pub use drawn_window::DrawnWindow;
pub use frame_params::FrameParams;
pub use glow_curve::GlowCurve;
pub use ring_stack::RingStack;

use ring_stack::{slot_start, Stack};

use crate::spectral::SpectralReading;
use crate::style::{Gradient, NoteNames, SevensLabel};
use crate::{
    AtmosphereSettings, Camera, ShadowSettings, GAP_MAX, GLOW_BALLISTICS_MAX, GLOW_CURVE_SHAPE_MAX,
    GLOW_CURVE_SHAPE_MIN, GLOW_REACH_MAX, GLOW_STRENGTH_MAX, MARK_THICKNESS_MAX, MAX_DRAWN_NODES,
    NODE_RADIUS_FACTOR, PLUS_SIZE_MAX, RING_INNER_MAX, RING_WIDTH_MAX,
};
use harmonigraph_core::{coords, Comma, Envelope, LatticePos, Tempered};

/// What every text-size bar offers, and so what a persisted scale is fit to.
/// One range for the three of them: they are the same control over three kinds
/// of text, and a reader comparing two of them should not have to check
/// whether they mean the same thing by 2.
///
/// The bar and the load clamp are the same two numbers BY REFERENCE, which is
/// the whole of the guarantee and the reason no test asserts it: an assertion
/// that clamping to this constant lands inside it only restates `clamp`. What
/// it buys is that widening the bar cannot leave a saved view loading at the
/// old ceiling — a setting that will not stay where it is put, silently.
///
/// It used to be able to. This constant lived a crate UP, in
/// `harmonigraph-ui`, which `ViewConfig` cannot see from here — so
/// [`ViewConfig::sanitize`] clamped `label_scale` to a written-out copy of
/// 0.3 and 3.0 with nothing tying the copy to the original, and the lattice's
/// was the one of the three bars that could drift. A test watched that gap
/// and is gone with it. Keep the reference: reintroduce a literal and the
/// drift comes back with nothing left watching for it.
pub const SCALE_BAR_RANGE: std::ops::RangeInclusive<f32> = 0.3..=3.0;

/// Arithmetic guard on a derived extent, not a picture-shaping limit:
/// [`MAX_DRAWN_NODES`] is what bounds the work, and it is reached long before
/// this. What this stops is the step before that bound is even computable — a
/// degenerate camera can hand [`ViewConfig::scrolled`] a world rectangle wider
/// than `i32`, and `2 * extent + 1` on a saturated extent overflows on the way
/// to counting the nodes it names.
const MAX_DRAWN_EXTENT: i32 = 4096;

/// How far from C the window's center may sit. Nothing musical is out here —
/// a billion fifths is not a pitch anyone reaches by scrolling — so this is
/// the bound that keeps `center + extent` inside `i32` for every reader of
/// [`ViewConfig::reach`], with room to spare for the widest extent
/// [`MAX_DRAWN_EXTENT`] allows.
const MAX_CENTER: i32 = 1 << 30;

/// How far from C a septimal sheet may sit — the axis the layer strip offers,
/// and so the only sheets a sanitized view holds.
///
/// A PICTURE limit rather than an arithmetic one, and the only one of the
/// three axes to have one: the strip draws a cell per sheet, so its axis is
/// what a reader can actually take hold of, and the cells have to stay wide
/// enough to grab. The bound it replaced was twenty steps of home travel
/// against a count of at most four each side — two numbers that could not be
/// shown on one axis without either the ends being unreachable or the cells
/// being a few points wide.
///
/// FOUR, and the nine sheets it allows, because one axis now answers what
/// those two numbers used to answer separately: this bounds the depth and the
/// parking at once, and the depth is the half with a picture behind it. #896
/// set it at six for the cell width alone, and the thirteen sheets that bought
/// put an ORDINARY pane over the node budget — a 16:9 lattice fully zoomed out
/// asks 21021 against [`MAX_DRAWN_NODES`]'s 20480 and is trimmed at the edges,
/// where nine sheets asks 12555 and is not (#916). Nine was also the old pair's
/// own maximum, so what this gives up against #896 is parking range, not depth
/// anyone had before.
///
/// Fewer cells are also wider ones, so the strip's own constraint pushes the
/// same way: nine cells on a settings column grab more easily than thirteen.
/// [`DrawnWindow::fit_to_node_budget`] still bounds the work, but it is a
/// backstop here rather than the thing that holds the picture together.
pub const SEVENS_LAYER_LIMIT: i32 = 4;

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
// Every field falls back to `impl Default` below, so a blob missing one costs
// that key alone rather than failing to parse and taking the whole persist —
// the camera, the dock and every other setting — down with it.
#[serde(default)]
pub struct ViewConfig {
    /// How far out along the fifths and thirds axes a played pitch is looked
    /// for a name and a node — the REACH, not a boundary, and not what is
    /// drawn: see [`reach`](Self::reach) for who reads them, and
    /// [`scrolled`](Self::scrolled) for the window the picture uses instead.
    /// The lattice itself has no end along these two.
    ///
    /// No bars: the drawn window answers to the camera now, so a bar here
    /// would look like it sets how much lattice there is while setting only
    /// how hard a spelling is hunted for. They stay generous, which is the
    /// whole of what the search wants.
    pub extent_threes: i32,
    pub extent_fives: i32,
    /// The lowest and highest sheet the lattice draws, in the same units
    /// [`center_sevens`](Self::center_sevens) is counted in: steps from the
    /// sheet containing C. Unlike the two extents above this IS the drawn
    /// window, and it keeps its bar: how deep the lattice runs is a question
    /// about the music, where how wide it runs is a question about the pane,
    /// and only the pane can be read off the screen.
    ///
    /// Absolute ends rather than a count each side of home, which is what the
    /// pair before them was. A count each side cannot say *two sheets above
    /// home and none below* — every stack it can describe is symmetric about
    /// the sheet it is read against — and the septimal axis is the one axis
    /// where that is a thing to ask for: the sheets above home and the ones
    /// below carry different spellings, so wanting one direction and not the
    /// other is an ordinary request rather than an odd one.
    ///
    /// The invariant is `min_sevens <= center_sevens <= max_sevens`, held by
    /// [`sanitize`](Self::sanitize) and by the one control that writes all
    /// three. Nothing downstream re-derives it: a reader wanting the count
    /// either side subtracts, and gets an honest asymmetric answer.
    pub min_sevens: i32,
    pub max_sevens: i32,
    /// Center of the window, in lattice steps from C (v1's Grid X/Y/Z). The
    /// center node renders at the world origin, so panning the window doesn't
    /// walk the content away from the camera.
    ///
    /// The fifths and thirds centers are driven by the camera rather than by a
    /// bar ([`follow_camera`](Self::follow_camera)): they are where the reach
    /// above is centered, and it has to stay under what is on screen. The
    /// sevens center is the home sheet, which is a choice, and keeps a control
    /// — the middle handle of the strip whose ends are
    /// [`min_sevens`](Self::min_sevens) and [`max_sevens`](Self::max_sevens),
    /// since which sheet is home is only meaningful among the sheets drawn.
    pub center_threes: i32,
    pub center_fives: i32,
    pub center_sevens: i32,
    // ---- The sevens layer ------------------------------------------------
    // How the sheets other than the home one draw. Both settings go inert
    // while the strip holds a single sheet (`min_sevens == max_sevens`), which
    // is where a fresh view starts. What
    // makes a small node legible over a large one is the Shadow
    // ([`shadow`](Self::shadow)) — each item multiplying the frame under it by
    // what its own ink casts, at any extent and on every sheet.
    //
    // The problem all three settings answer: the 5-limit sheet wants its
    // pitch classes as large as they will go, and at one world unit per step a
    // node's visible edge already reaches 0.376 of the way to its neighbor.
    // Turning depth on asks the same rectangle to hold three times the
    // nodes. Something has to give, and it must not be the home sheet — that
    // is the picture.
    /// How much smaller a node draws for each step it sits off the home
    /// sheet: the factor is `sevens_size^|sevens - center_sevens|`. 1 keeps
    /// every sheet the same size.
    ///
    /// Smaller in BOTH directions, deliberately, even though a positive
    /// sevens step is the one nearer the camera — this is not perspective.
    /// Size here says *how far from the home sheet*, because that is the
    /// thing worth reading: the home sheet is the ground the music is heard
    /// against, so it stays the largest thing on screen whichever way the
    /// sevens axis runs.
    pub sevens_size: f32,
    /// What text an off-sheet node's label carries (see [`SevensLabel`]).
    pub sevens_label: SevensLabel,
    /// Overall size of a node's label, as a multiple of its built-in sizes —
    /// the note name, the marks stacked beside it and the cents line under it
    /// together, so the label keeps its proportions and only the whole of it
    /// grows.
    ///
    /// It trims what the CAMERA decides rather than replacing it. A label
    /// tracks the on-screen size of the lattice it sits on (see
    /// [`Camera::screen_scale`](crate::Camera::screen_scale)), which is what
    /// keeps a name the same size on its node at every zoom; this says what
    /// that size is.
    pub label_scale: f32,
    /// Under each note-name label, also show the node's pitch class in
    /// cents.
    pub show_cents: bool,
    /// WHICH nodes carry a label: every one, every one the music has
    /// visited, or only what is sounding (see [`NoteNames`]).
    ///
    /// The one setting here that reaches into the PAST, and what it reaches
    /// with is drawn in TYPE alone -- see the [`trail`](crate::trail) module
    /// for why that is the whole design and not an implementation detail.
    /// [`NoteNames::Past`] is the only mode that fills
    /// [`NodeInstance::trail`](crate::NodeInstance::trail); a name under
    /// [`NoteNames::All`] is the label layer's own answer and carries no
    /// memory at all.
    pub note_names: NoteNames,
    /// Whether the editor draws the active lattice map's assignment rings and
    /// MIDI note labels over the lattice. The map and its tuning remain active
    /// when these annotations are hidden.
    pub show_map_indicators: bool,
    // How a sounding node's middle is painted has no field here: what lights
    // it is the node glow, and nothing switches that glow's paint. The field
    // styles (Vortex, Checker and Spiral) are gone with the core disc they
    // painted, and with them the `node_style` key and the per-node seed that
    // animated them. Saved blobs still carry that key, naming any of the
    // SEVENTEEN the enum answered to — those three, the Steady it defaulted
    // to, and the thirteen trimmed before them that its serde aliases went on
    // loading (Breathe, Sparks, Wire, Corona, Plasma, Aurora, Marble, Lava,
    // Filament, Stripes, Rings, Tiles, Pinwheel). Serde ignores unknown keys,
    // so such a blob loads intact and drops the key on the next save. This is
    // the only surviving record of that set, which is why it names it in full.
    /// The curve the low-to-high pitch gradient follows, as its six knobs
    /// (see [`Gradient`]). Every pitch-colored shape in the scene reads
    /// it through one table, so this is the only place the gradient is set.
    pub pitch_gradient: Gradient,
    /// How thick the octave band is — the MIDI ring, in quad UV units, whose
    /// inner edge is wherever the layer inside it ended plus one
    /// [`ring_gap`](Self::ring_gap) (see [`rings`](Self::rings)). Every outer
    /// style fits its glyphs' radial footprint to it, so the band IS the
    /// glyph set's radial extent.
    ///
    /// **0 turns the octave layer off**, as a width of 0 turns any layer off,
    /// and the layers outside it close up over the slot it leaves. That is a
    /// picture worth having rather than a degenerate one: with the band gone
    /// the audio ring and the melody/bass marks are what the node is made of,
    /// which is the lattice read as a spectrum with the keys marking only its
    /// outer voices.
    pub band_width: f32,
    // The octave layer's backdrop and solidity are fixed on in the shader and
    // have no fields of their own. The backdrop — the silent octaves standing
    // in the rings' own ground behind the sounding sectors — is what makes
    // the annulus complete, so a lone octave still reads as a whole note; and
    // the glyphs are always the crisp classic shapes. How bright that backdrop
    // is IS a field, and it is [`lattice_ground`](Self::lattice_ground) below, one
    // number under this layer and the audio ring together. Saved blobs may
    // still carry the keys the pair rode on (`outer_backdrop`, first a bool
    // and then an opacity under `outer_backdrop_alpha`, and `outer_solidity`);
    // serde ignores unknown keys, so such a blob loads intact and simply
    // drops them on the next save.
    /// Where the stack BEGINS, in quad UV units: the radius the innermost layer
    /// left on puts its inner edge on (see [`rings`](Self::rings)), and so the
    /// size of the empty middle a node carries.
    ///
    /// The one bar of the stack that is a POSITION rather than a width, and
    /// what it sizes is not a layer: nothing is drawn in there. What fills a
    /// node's middle is its own light ([`glow_reach`](Self::glow_reach)), which
    /// is laid OVER whatever the node draws rather than taking a slot in the
    /// stack, so the middle is at once empty and the brightest part of the
    /// node.
    ///
    /// The handle names the radius the innermost ring starts at directly, with
    /// no [`ring_gap`](Self::ring_gap) in front of it: a gap is padding between
    /// two DRAWN layers, and there is no layer inside this one to stand off.
    ///
    /// 0 seats the stack on the node's own center — the innermost ring reaches
    /// it and its sectors close into pie wedges rather than reading as an
    /// annulus. That is the bottom of the bar's travel rather than an off
    /// switch, this being the one size on the node that switches nothing off,
    /// and it is a picture worth having: the node as one solid reading.
    ///
    /// Widening it pushes every layer outward, and the quad runs out from the
    /// outside in — one refused layer at a time (see [`Stack::take`]), the same
    /// way it does when a ring is widened.
    pub ring_inner: f32,
    /// The node's shared padding, in quad UV units: radially, the gap between
    /// one ring of the stack and the next (see [`rings`](Self::rings)), which
    /// is also what stands a melody/bass mark off the band it continues;
    /// angularly, the constant-thickness cut between one octave sector and the
    /// next (see [`octave_gap_width`](Self::octave_gap_width)).
    ///
    /// One number on both axes so the concentric layers and the sectors within
    /// them carry the same rhythm of empty space. The radial use spends room
    /// from the stack while the angular use cuts slices out of a ring already
    /// placed, but both read as the same padding on the node.
    ///
    /// 0 closes the stack up and the sectors round: every ring meets the one
    /// inside it, a mark seats against the band, and adjacent octave slices
    /// meet. Radially, a gap is only ever spent between two DRAWN layers, so a
    /// ring dialled to 0 costs its own slot and the gap that would have stood it
    /// off together.
    pub ring_gap: f32,
    /// A node's RINGS where nothing is sounding, as an `L*` 0..100 — one
    /// neutral grey under both of the surfaces the node itself draws empty:
    ///
    /// - the **audio ring** wherever it reads silence, which its ramp is
    ///   re-anchored to open on ([`ring_gradient`](crate::ring_gradient));
    /// - the **MIDI ring**'s octave slices that are not sounding, which ARE
    ///   this colour, with a sounding octave's pitch painted over them.
    ///
    /// One number under both, like [`ring_gap`](Self::ring_gap) above it,
    /// and for the same reason: they are one picture read together — two annuli
    /// a gap apart on a single node — so a ground that differed between them
    /// says the two are different KINDS of thing when the only thing they have
    /// in common is being empty. Each deriving its own ground instead — the
    /// audio ring off the analyzer's gradient (whose dark end carries that
    /// gradient's own hue), the MIDI ring off the note's colour whitened and
    /// laid on at a fixed opacity — lands two near-greys a hair apart in tint,
    /// and no bar can dial two routes onto one value.
    ///
    /// The lattice's third at-rest surface, the markers standing at the
    /// positions, is on [`marker_ink`](Self::marker_ink) below and is free of
    /// this: it is not part of a node, and what it is dialled against is the
    /// light behind the nodes rather than the ring a gap away.
    ///
    /// **Neutral**, because the ground is what the two have in common rather
    /// than a colour either owns: it carries no hue, and each surface's light
    /// is added over it.
    ///
    /// Stated in `L*` because that is the axis the ask is on: perceived
    /// brightness, the same units [`Gradient::lightness`](crate::Gradient) is
    /// authored in, so a ground and a gradient can be compared by their
    /// numbers. There is no off position and it needs none — each ring has a
    /// width. What the bottom of the bar reaches is black, which against this
    /// skin's panel reads as holes punched through the lattice; a little above
    /// it, at the panel's own `L*` (8.8 on the fresh skin), a quiet node
    /// vanishes into the pane and only sounding ones draw.
    pub lattice_ground: f32,
    /// The resting MARKERS, as an `L*` 0..100 on the same axis as
    /// [`lattice_ground`](Self::lattice_ground) above: the neutral grey every
    /// cross standing at a home-sheet position is drawn in
    /// ([`derive_pluses`](crate::derive::derive_pluses)).
    ///
    /// And the grey a label on a node NOTHING is sounding under is drawn in
    /// with it, before the note's activation carries that label toward white.
    ///
    /// A bar of its own rather than a share of the ground, so the two are read
    /// against each other by their numbers and set independently: equal numbers
    /// are one grey under the whole resting picture, and every other pairing is
    /// reachable from there. What wants the freedom is a picture with the glow
    /// in it — a ground dialled down far enough for the light behind the notes
    /// to read takes the whole resting field with it, and the field is what
    /// says where the positions ARE. That structure is the thing a person
    /// navigates by, and it has nothing to do with how loud an empty ring
    /// should look.
    ///
    /// Absolute, not an offset. An offset would keep one master brightness for
    /// the resting picture, which is exactly the coupling this bar exists to
    /// cut: dragging the ground would still move the markers, just by a
    /// remembered amount.
    ///
    /// **Neutral**, for [`lattice_ground`](Self::lattice_ground)'s reason —
    /// hue in this picture is the music's. There is no off position and none is
    /// needed — [`plus_arm`](Self::plus_arm) at 0 takes the field away.
    pub marker_ink: f32,
    /// How many octaves one turn of a node covers at FULL SIZE (see
    /// [`octaves`](crate::octaves)), 1..=11 — not how many it draws, which is
    /// this plus twice [`octave_extras`](Self::octave_extras). Each is exactly
    /// one octave and they all share whatever the extras leave, so this says
    /// how many degrees an octave of the main register is worth. Notes past
    /// either end of the whole wheel light the outermost indicator on their
    /// side.
    pub octave_count: u32,
    /// The MIDI pitch at the TOP of the wheel — on every node, whatever its
    /// pitch class: a node's ring is turned so that its own octaves land on
    /// their pitches, by up to half a slice either way.
    /// [`sanitize`](Self::sanitize) holds it to the settable limits.
    pub octave_center: f32,
    /// Extra octaves at EACH end of the wheel, drawn small: 0..=5, and never
    /// so many that the whole wheel passes eleven slices. Each one reaches an
    /// octave further up AND down the keyboard for a sliver of the turn, where
    /// an octave of count is paid for by every full-size octave at once.
    pub octave_extras: u32,
    /// How wide one extra is, as a fraction of an EVEN slice (the turn over
    /// the whole wheel, extras included), 0.1..=1. Under 1 an extra is always
    /// narrower than a full-size octave, whatever the count and however many
    /// extras there are, and 1 is an even wheel.
    pub octave_extra_size: f32,
    /// How much the extras GRADE from the outermost inward, 0..1: 0 is a flat
    /// fringe of equal slivers and 1 is a ramp that meets the full-size
    /// octaves in a step the size of its own. The outermost extra is the size
    /// above whatever this is, so it is a shape rather than a second
    /// strength — and it is inert without two extras to differ.
    pub octave_extra_blend: f32,
    // ---- What the audio ring says ----------------------------------------
    // Which notes are HELD, or which sine waves are SOUNDING. The two are
    // different questions about the same music, and the lattice answers both
    // at once: the keys keep everything they draw, and the measurement gets a
    // ring of its own inside the octave band. What is settled here is which of
    // two readings that ring carries, how thick it is — which is also whether
    // it is there at all — and how each of the two is measured. See the fold in
    // `harmonigraph-ui`, which is where all of the analysis lives — nothing in
    // this crate reads audio.
    /// Which reading of the analyzer the audio ring carries.
    ///
    /// The one control that says what the spectrum indicator IS: both readings
    /// fill the same annulus in the same colours, and neither touches the MIDI
    /// picture around it. [`SpectralReading`] is where
    /// the two are described and where the case for one selector over two
    /// boxes lives.
    ///
    /// It does NOT say whether the ring is drawn — that is
    /// [`spectral_ring_width`](Self::spectral_ring_width), the ring's own size,
    /// exactly as a thickness of 0 is what turns the marks off. One off switch per
    /// layer, in the same place on every layer: a selector that also carried an
    /// Off would be a second one for this layer alone, and the two would then
    /// have to agree about what a ring of some width carrying no reading is.
    pub spectral_reading: SpectralReading,
    /// How far off a node's own pitch a partial may sit and still light it, in
    /// cents: the standard deviation of the Gaussian the fold weights power by,
    /// 1..=50.
    ///
    /// The FOLD's kernel, so it is
    /// [`SpectralReading::Fold`](crate::SpectralReading)'s alone.
    /// [`Spectrum`](crate::SpectralReading::Spectrum) reads the analyzer raw
    /// and shows a whole window of it per wedge
    /// ([`spectral_ring_range`](Self::spectral_ring_range)), where a kernel
    /// would be a blur over a picture whose whole subject is where a partial
    /// sits.
    ///
    /// A WEIGHT and not a gate, which is the whole of why it is a width in
    /// cents rather than a tolerance: distance maps to dimness, so a detuned
    /// partial fades rather than switching off, and ±15¢ of vibrato reads as
    /// breathing instead of flicker.
    ///
    /// Independent of [`Tuning::tolerance`](harmonigraph_core::Tuning), which
    /// answers a different question — whether a PLAYED pitch class counts as
    /// this node's — and is a hard threshold because a MIDI note either is that
    /// node or is not.
    ///
    /// Narrow fresh, at 2.1¢, because just intonation is what this is for:
    /// partials of just-tuned notes land dead on nodes, so a narrow kernel
    /// draws them crisp and rejects everything between. 12-TET material is
    /// rejected along with the rest at that width and wants the bar dragged
    /// right — a tempered major third's 5th harmonic sits 13.7¢ off the node
    /// it belongs to and a harmonic seventh 31¢ off, both several times the
    /// fresh kernel wide, so neither reaches its node until the bar is opened
    /// to the order of the miss.
    pub spectral_width: f32,
    /// How thick the audio ring is, in the same quad UV units as the octave
    /// band's own width ([`band_width`](Self::band_width)) — and **0 turns the
    /// ring off**, which is the only switch the LAYER has. Which nodes wear it
    /// is the gate's ([`spectral_ring_gate`](Self::spectral_ring_gate)), and
    /// the two are asked in that order: no width is no ring anywhere, and the
    /// gate never runs.
    ///
    /// It is the INNERMOST layer of the stack ([`rings`](Self::rings)), so its
    /// inner edge is where the stack begins ([`ring_inner`](Self::ring_inner))
    /// and everything outside it moves when it is dragged. INSIDE the band
    /// rather than outside it because of which disagreement between the two
    /// pictures is common: energy at a pitch class with no note held — every
    /// partial above a played chord's roots — happens constantly, and a held
    /// note with nothing sounding at it is rare. The common case is the one
    /// that gets the inner ring, where a busy ring of small wedges is contained
    /// by the band around it rather than fringing the node.
    ///
    /// One annulus for both readings, at this width either way: the reading
    /// changes what is measured and where in a wedge it is sampled, never where
    /// the ring is.
    pub spectral_ring_width: f32,
    /// How much of the spectrum one wedge of the audio ring shows, in cents,
    /// centred on that wedge's own octave — the ZOOM of the segment, and
    /// [`SpectralReading::Spectrum`](crate::SpectralReading)'s alone.
    /// [`Fold`](crate::SpectralReading::Fold) answers one number for a whole
    /// wedge, so it has no window to size; its own setting is
    /// [`spectral_width`](Self::spectral_width).
    ///
    /// At the ceiling ([`SPECTRAL_RANGE_MAX`](crate::SPECTRAL_RANGE_MAX), an
    /// octave) a wedge stands for exactly the octave it names, so neighbouring
    /// wedges meet at the pitch they share and the ring is one continuous
    /// reading — the wheel's own pitch map, painted. It is also the setting at
    /// which the ring says nothing about the NODE: with no extras the wheel's
    /// map is shared by every node, so every ring on screen is then the same
    /// picture turned, and what is worth looking at is the disagreement
    /// between a node's own pitch and where the energy near it actually sits.
    ///
    /// Fresh at 10¢, a wedge that is very nearly one pitch: what it shows is
    /// energy AT the node rather than energy somewhere near it, which is the
    /// reading that says where a partial actually landed.
    ///
    /// Every miss the material makes is wider than that — a tempered major
    /// third's 5th harmonic sits 13.7¢ off its node, a harmonic seventh 31¢,
    /// and the syntonic comma between two just spellings is 21.5¢ — so at the
    /// fresh width a detuned partial falls outside its own wedge and reads as
    /// that node going quiet rather than as a ring off centre. Seeing WHERE it
    /// went is what the bar is dragged right for, out to the order of the miss;
    /// a whole tone across the wedge holds all three at once, at the cost of a
    /// wedge that answers for a range rather than for a pitch.
    pub spectral_ring_range: f32,
    /// How loud the loudest thing a node's ring shows has to read before that
    /// node draws a ring at all, as a level on the analyzer's own Level window
    /// (0..=1, the axis the ring's colours are read off — see
    /// [`SPECTRAL_GATE_MIN`](crate::SPECTRAL_GATE_MIN)).
    ///
    /// The ring is a window onto ONE grid the whole lattice shares, so without
    /// this every node in view wears one whatever is sounding, and a stretch of
    /// spectrum with nothing in it draws as a ring at the ramp's floor rather
    /// than as no ring. That is an honest reading and a poor picture: the
    /// lattice is hundreds of nodes, and a reading every one of them carries
    /// says only where the nodes are. This is what buys back the other half —
    /// a ring is then a node with something sounding at it, and where the rings
    /// ARE is the picture.
    ///
    /// Per node and not per wedge, and the two readings answer it differently
    /// only in what a wedge reaches: the fold's wedge is one level at that
    /// octave's own pitch, and the spectrum's is the loudest bucket in the
    /// window it spreads across its arc
    /// ([`spectral_ring_range`](Self::spectral_ring_range)).
    ///
    /// Both ends are usable settings rather than guard rails. 0 is the gate off
    /// — every node rings, which is the picture to go back to when what is
    /// wanted is the analyzer's whole reading at once. The top asks for a
    /// full-scale wedge, where a ring is a rare event on the loudest node in a
    /// phrase.
    ///
    /// **It selects far more sharply under the fold than under the spectrum**,
    /// and that is the two readings rather than anything here. A fold wedge is
    /// energy concentrated AT its octave's pitch over a local noise floor, so
    /// most nodes read near nothing and a gate picks out the constellation; a
    /// spectrum wedge is a whole window of the raw grid, and in dense material
    /// there is something loud within a hundred cents of nearly every pitch
    /// class, so the nodes' levels sit close together and the bar tips from
    /// most rings to none over a short stretch of its travel. Measured on a
    /// sawtooth 24 dB down over a 200¢ window: the fold rings 601 nodes of
    /// 1025 at 0.1 and 177 at 0.4, where the spectrum rings all 1025 at both
    /// and none by 0.6.
    ///
    /// That contrast is a function of the WINDOW, which is why the width it was
    /// taken at is named rather than called the fresh one: the fold has no
    /// window at all and the spectrum's is
    /// [`spectral_ring_range`](Self::spectral_ring_range), so at the fresh 10¢
    /// the spectrum spreads ±6.25¢ across a wedge rather than ±100¢ and selects
    /// very nearly as sharply as the fold. The sentence above is what the two
    /// readings do as that bar is opened, and the numbers are one point on it.
    pub spectral_ring_gate: f32,
    /// How far the gate DROPS for a bucket already open, as a share of the
    /// Level window — a Schmitt trigger's lower threshold.
    ///
    /// A gate is a threshold on a live measurement, so a level sitting near it
    /// crosses repeatedly and the node's whole annulus answers each crossing.
    /// [`RingFade`](crate::RingFade) makes each of those a slow transition, and
    /// this makes them rare: what a fade fixes is the SPEED of a crossing, and
    /// what a second threshold fixes is how often there is one. Two different
    /// halves of one complaint, which is why both are here rather than one
    /// being tuned until it covers for the other.
    ///
    /// 0 is one threshold, exactly the picture with no hysteresis in it. The
    /// useful settings are small: the whole point is a band narrower than the
    /// gap between a partial and the haze, so it swallows the wobble without
    /// swallowing a real change.
    pub spectral_ring_hysteresis: f32,
    /// How long the ring's READING takes to rise toward a louder measurement,
    /// in seconds, and [`spectral_ring_release`](Self::spectral_ring_release)
    /// how long to fall.
    ///
    /// Its own times rather than the analyzer's, because the ring is asked a
    /// different question from the Spectral pane: the pane is a measurement
    /// instrument and wants to show what is there, and the ring is a legibility
    /// device on a lattice of hundreds of nodes and wants to show whether a
    /// harmonic is PRESENT. A filter long enough to settle the second is longer
    /// than the first should ever be.
    ///
    /// Its own times rather than the note Fade, too, because that parameter's
    /// default is a judgement about MIDI transients. Riding the ring's
    /// steadiness on it means one cannot be tuned without detuning the other.
    /// The Fade still carries the ring's ARRIVAL and DEPARTURE
    /// ([`RingFade`](crate::RingFade)) — a layer of a node comes and goes with
    /// the node. What is here is how fast the reading INSIDE it moves.
    pub spectral_ring_attack: f32,
    /// See [`spectral_ring_attack`](Self::spectral_ring_attack).
    pub spectral_ring_release: f32,
    // ---- Note envelope ---------------------------------------------------
    // How a note ARRIVES and how it LEAVES, for every layer of the node at
    // once. The DURATION of both is the host-automatable Fade param and lives
    // in [`FrameParams`]; the shape here is the other half of it, and
    // [`ViewConfig::envelope`] is where the two are put back together.
    /// How curved both ends of the note envelope are, 0..=1: 0 the straight
    /// line every layer has always faded on, 1 the sharpest curve on offer.
    /// It walks the exponent of an ease-out — see
    /// [`Envelope::approach`](harmonigraph_core::Envelope), which is also
    /// where the case for a power over an exponential is written.
    ///
    /// One number for the arrival and the departure together, which is the
    /// point of it — a curve is a house style for how things move, and a
    /// lattice that answered the keys one way and let go another would read
    /// as two instruments. It does NOT reach the trail, whose fade is a
    /// memory decaying over tens of seconds rather than a note's own
    /// envelope, and which stays deliberately linear (see
    /// [`trail`](mod@crate::trail)).
    ///
    /// About 0.31 fresh, which is a gentle power rather than the straight line 0
    /// draws — a note that leaves quickly at first and then lingers reads as
    /// decaying rather than as being wound down. A blob with no `fade_shape`
    /// key gets that, like every other missing key: the container-level
    /// `#[serde(default)]` makes `impl Default` the one fallback, and
    /// `a_view_missing_any_one_key_reloads_at_the_fresh_value` holds it.
    pub fade_shape: f32,
    /// Appearance of note arrivals and departures, on the shared Note fade clock.
    pub note_animation: NoteAnimationConfig,
    // An unlit node has no mark of its own: the marker standing at a node
    // position is the whole of what says the position is there, and it stands
    // on the home sheet alone (see `derive_pluses`) — off it, a position at
    // rest is unmarked, which is the same reason it is not hoverable. So a
    // resting lattice is its own drawing rather than a field of
    // placeholders, and every disc on screen is a note.
    // ---- Melody / bass highlight -----------------------------------------
    // Mark the outer held notes, so the melody and/or bass line reads at a
    // glance out of a chord. "Outer" is by sounding pitch (`Voice::pitch`,
    // which includes MPE/tuning bends), over HELD voices only: a released
    // note is on its way out and shouldn't keep the mark from the note that
    // replaced it.
    //
    // A mark rides the OUTER EDGE of that note's octave indicator and
    // nothing else — no layer inside it is repainted. That also makes it the
    // layer that survives a chord voiced within
    // a single pitch class: every octave of one note lands on the same node,
    // differing only by slot.
    /// The highest and lowest held notes share one strip: a mark is its own
    /// octave's slice continued outward, so what
    /// tells the two apart is WHICH slice each one extends — the slices are
    /// ordered by pitch round the node, and the higher marked one is
    /// ordinarily the melody. A note that is at once the highest and the
    /// lowest — a lone held note, or a chord whose top and bottom share a
    /// pitch class — is one slice extended once, which is the whole of what
    /// there is to say about it.
    ///
    /// That ordering is the usual case rather than a guarantee. A mark
    /// outlives its key and a released voice claims each end from its own
    /// stamp, so through one release a fading melody can sit on a LOWER slice
    /// than the live bass beside it, with nothing in the picture to say which
    /// is which — the radius that used to say it is what the shared strip
    /// spends. `a_released_end_can_mark_a_lower_slice_than_the_live_one`
    /// builds that state and is where the window is measured.
    /// How thick the melody/bass mark strip is, in quad UV units — the same
    /// units as the ring widths and [`ring_gap`](Self::ring_gap), so the whole
    /// stack reads against itself directly. One depth for both ends: they are
    /// one kind of mark, and letting them differ would say something that isn't
    /// true.
    ///
    /// A mark is an annular sector on exactly the angles of the octave
    /// responsible for it, and it takes the LAST slot of the stack
    /// ([`rings`](Self::rings)): one [`ring_gap`](Self::ring_gap) out from
    /// whatever ring the node ends with, ordinarily the octave band whose slice
    /// it is continuing. Its SIDES are cut by
    /// [`ring_gap`](Self::ring_gap), the same padding that separates one
    /// indicator from the next, so the mark reads as that indicator continued
    /// however far out the stack stands it; a `ring_gap` of 0 closes the
    /// stand-off, and the mark meets its slice.
    ///
    /// 0 turns the marks off, as a width of 0 turns any layer off. Absolute
    /// rather than a fraction of the band's width, which would move the marks
    /// every time the band is resized.
    pub mark_thickness: f32,
    /// Seconds a new melody/bass target waits before its carried mark eases in.
    /// [`crate::NodeMotion`] starts this wait when an end changes hands, including
    /// inheritance after another note is released. A target lost during the
    /// wait cancels its arrival; an already visible mark fades from its current
    /// level instead of restarting at full brightness.
    ///
    /// Independent of the note Fade: the delay filters brief handoffs, while
    /// [`envelope`](Self::envelope) controls the subsequent arrival and release.
    /// Zero begins the arrival immediately. Motion retains the handoff state
    /// even after the tracker prunes the voice that gave up the end.
    pub mark_delay: f32,
    // ---- Home markers ----------------------------------------------------
    // The cross standing at each home-sheet node position (see
    // `derive_pluses`), and the whole of what an unplayed lattice draws. Its
    // three lengths are what is set here — how far an arm reaches, how thick
    // it is, and how much of its end fades out — while the EDGE is not one of
    // them: it is a ring's edge, carrying the shader's one screen-constant
    // soft band rather than a softness of its own.
    //
    // Its colour is not here either. That is
    // [`marker_ink`](Self::marker_ink), up with the ground it is dialled
    // against, because the two are one question — how bright the lattice is
    // where nothing sounds — asked of the markers and of a node's unlit rings,
    // and a brightness read against the wrong neighbour is read against
    // nothing.
    /// How far one arm reaches, crossing to tip, in the quad UV a node's ring
    /// radii are dialled in ([`RING_INNER_MAX`] and the widths around it) — so
    /// a marker and the middle a node's rings stand around are two readings on
    /// ONE axis, and a marker that fits inside `ring_inner` can be read off the
    /// two numbers rather than by eye. 0 takes the markers away and with them
    /// the lattice's resting picture.
    ///
    /// The one length that reaches the renderer as a WORLD distance:
    /// `derive_pluses` spends the uv against the scene's node radius, and the
    /// other two travel as shares of this, so nothing downstream carries a
    /// second copy of the convention.
    pub plus_arm: f32,
    /// How thick an arm is, ACROSS it and all the way across — the whole bar,
    /// not half of one — in the same quad UV [`plus_arm`](Self::plus_arm) is
    /// in.
    ///
    /// A length of its own rather than a share of the arm, which is what lets
    /// a long arm be a hairline and a short one a block: tied to the arm, the
    /// shape would have one proportion and the arm bar would be the only
    /// control the marker has. Past twice the arm the cross has filled its own
    /// square, and every width above that draws that same square.
    ///
    /// 0 is not off. An arm with no thickness is still cut with the same
    /// screen-constant band as one with, so the bottom of the bar is the
    /// thinnest cross this screen can draw rather than no cross —
    /// [`plus_arm`](Self::plus_arm) at 0 is what takes the field away.
    pub plus_width: f32,
    /// How far the tapered END of an arm runs, in the same quad UV
    /// [`plus_arm`](Self::plus_arm) is in: each arm is solid out to
    /// `plus_arm - plus_taper` and fades to nothing by its tip. 0 is a square
    /// end; a taper equal to the arm fades the whole of it, from full at the
    /// crossing to nothing at the tip.
    ///
    /// A WIDTH beside the reach rather than a share of it, and paired with
    /// `plus_arm` on one two-handle bar, for the reason every soft edge here
    /// is a pair: a taper tied to the arm as a fraction would make a longer
    /// arm always a softer one, and there would be no way to ask for a long
    /// crisp arm or a short misty one.
    ///
    /// The four ends taper and the arms' SIDES do not. What is being softened
    /// is where the marker STOPS; a cross faded along its sides as well is a
    /// blurred plus rather than one reaching out of its crossing.
    ///
    /// A cross's blur shadow follows the coverage this fades. Its distance
    /// shadow treats the half-alpha contour as the end of the exact field, the
    /// same contour every other caster's distance cell holds.
    pub plus_taper: f32,
    /// Meantone mode: lock the major-third tuning to four perfect fifths
    /// (temper out the syntonic comma, 81/80). While on, the third-tuning
    /// value is derived from the fifth (in `begin_frame`) and note names are
    /// respelled without their comma marks.
    ///
    /// One of two comma switches, and the pattern for both: the flag is named
    /// after the temperament that tempers its comma out, [`Self::marvel`] is
    /// the same switch for 225/224, and [`ViewConfig::tempers`] is how the UI
    /// reaches either by [`Comma`] rather than by name.
    ///
    /// Whether this engages by itself is [`Self::meantone_auto`]'s business;
    /// releasing it is always an edit of the major third (or this switch,
    /// while the auto-detect is off).
    pub meantone: bool,
    /// Auto-detect meantone: engage [`Self::meantone`] whenever the tuning
    /// params land within `TEMPER_TOLERANCE` of the meantone identity —
    /// however they got there (a learned chord, the 12-TET preset, a drag
    /// of either bar). The major third then snaps to four perfect fifths
    /// and the comma marks go.
    ///
    /// Engage-only, deliberately: the lock has to survive dragging the
    /// FIFTH, which moves the derived third out from under a third param
    /// that is inert while the lock holds. So the release is the one edit
    /// that can mean nothing else — pulling the major third itself more
    /// than the tolerance away from the derived value.
    ///
    /// On by default: a project at 12-TET (400 = 4·700 − 2400) is meantone
    /// whether or not anyone said so, and its E and E- name one pitch, so
    /// the detect has something to say about most tunings without being
    /// asked. Switching this off leaves the mode wherever it is and hands
    /// the switch back.
    pub meantone_auto: bool,
    /// Marvel mode: lock the harmonic-seventh tuning to two fifths plus two
    /// thirds (temper out the septimal kleisma, 225/224). The same switch as
    /// [`Self::meantone`] one prime up — while on, the seventh-tuning value
    /// is derived in `begin_frame` and the sevens sheet is respelled onto the
    /// home sheet, where a harmonic seventh reads `A♯-2` (two fifths plus two
    /// thirds) instead of `B♭↓`.
    ///
    /// The third it derives from is the one in USE, so with meantone on too
    /// the pair composes into septimal meantone (a seventh of ten fifths) and
    /// every name on the lattice comes out a plain letter.
    pub marvel: bool,
    /// Auto-detect marvel: [`Self::meantone_auto`]'s twin, engage-only for
    /// the same reason — the lock has to survive dragging the fifth or the
    /// third, either of which moves the derived seventh out from under a
    /// seventh param that is inert while the lock holds.
    ///
    /// On by default, on the same grounds as the meantone detect: 12-TET
    /// tempers 225/224 out as well (1000 = 2·700 + 2·400 − 1200), so a
    /// project there has one pitch under `B♭↓` and `A♯` whether or not
    /// anyone said "marvel", and the detect respelling the sevens sheet is
    /// the tuning's own arithmetic showing up in the names.
    pub marvel_auto: bool,
    /// Hide every tab bar so adjacent panes — lattice above spectrum, in the
    /// default layout — record as one seamless surface. Tab toggles it.
    ///
    /// The separators keep their regular width, so the spacing between panes
    /// is the same in both modes and a take framed in one is framed in the
    /// other.
    pub frameless: bool,
    /// Show the performance overlay (a small draggable HUD with frame rate,
    /// memory and workload counts; per-stage CPU time waits for
    /// [`Self::show_perf_detail`]). Interactive shells only — the offline
    /// renderer never draws it, keeping its frames deterministic.
    ///
    /// Off by default: the HUD is a development instrument, and it sits over
    /// the picture the plugin exists to draw. The Display tab's System page,
    /// under Performance, is where it gets switched on.
    ///
    /// It was ON in the DAW when the 2026-09-07 look was captured, and stayed
    /// out of that capture for the reason above — the overlay is what the
    /// picture is read AGAINST while it is dialled, not part of the picture.
    /// `the_performance_overlay_ships_off` holds it.
    ///
    pub show_perf: bool,
    /// Expand the overlay from the headline numbers into the full per-stage
    /// breakdown of where a frame goes.
    ///
    /// Off by default: the breakdown exists to answer "which stage is eating
    /// the frame", and once it has, a dozen rows of scaffolding is not what
    /// you want sitting over the picture. Inert while `show_perf` is off.
    pub show_perf_detail: bool,
    /// Offscreen render resolution as a multiple of the pane's native pixel
    /// size: >1 supersamples (crisper glyph edges), <1 renders coarse and
    /// upscales. 1.0 reproduces the pre-offscreen-pass output exactly.
    pub render_scale: f32,
    /// Lattice and spiral bloom: how much blurred brightness gets added back
    /// as a halo around bright notes. The spectrogram's MIDI ribbons use
    /// [`crate::SpectralAtmosphere::note_glow`] instead. 0 disables this bloom chain.
    pub bloom_strength: f32,
    /// The node halo: how far past a node's outermost drawn edge its light
    /// spreads, in the quad UV units the layer sizes are in. 0 turns it off —
    /// nothing is drawn at all — so the glow's other fields need no toggle of
    /// their own, and the Glow section greys them under it.
    ///
    /// What it draws is every sounding octave's hue laid round the node by
    /// angle, over a falloff sized to the whole node: the node's outermost
    /// drawn edge plus this reach is the falloff's domain and outer limit.
    ///
    /// [`glow_curve`](Self::glow_curve) says how much light is left at each
    /// distance inside that span. Keeping the two separate makes a wide Reach
    /// useful both as a larger accent and as a faint field carried across the
    /// gaps between nodes. The ceiling ([`GLOW_REACH_MAX`]) is sized for the
    /// second picture — several lattice steps, where every node's light
    /// overlaps its neighbourhood's.
    ///
    /// The glow switches off at 0: the view draws exactly the
    /// ink the ring stack describes and nothing around it.
    ///
    /// Every node's glow is drawn into a target of its own, with SCREEN
    /// blending, so two neighbours' halos meld like light rather than summing
    /// to white and neither one's draw order is readable in the overlap. That
    /// target is one field across every sheet, laid down UNDER the lattice: the
    /// rings, the markers and the names are all drawn over it, so the middle of
    /// a node keeps the light its neighbours put there and every shadow in the
    /// frame lands on it. The node's own INK takes that field too, so a ring
    /// reads as a shape inside
    /// its light rather than a silhouette cut out of it — whole where the ink
    /// is unlit, and on [`glow_wash`](Self::glow_wash)'s share where it is a
    /// sounding slice.
    ///
    /// Distinct from [`bloom_strength`](Self::bloom_strength) in what it
    /// measures: the bloom thresholds a finished PICTURE, so only the bright
    /// end of the gradient blooms and it is one number over every picture the
    /// plugin draws. This is a layer of the lattice's nodes, drawn from the
    /// same octave colours their discs are.
    pub glow_reach: f32,
    /// Experimental glow texture and breathing, shared by editor and exports.
    pub atmosphere: AtmosphereSettings,
    /// How much light the node glow lays down. Inert while
    /// [`glow_reach`](Self::glow_reach) is 0.
    pub glow_strength: f32,
    /// How the close halo's brightness falls inside [`glow_reach`](Self::glow_reach).
    /// The endpoints stay fixed: full at the node's centre and zero where the
    /// reach ends.
    pub glow_curve: GlowCurve,
    /// The Shadow, dialled per GROUP of casters ([`ShadowSettings`]): the
    /// lattice's geometry and text, and the spectral pictures' geometry and
    /// text. The spiral inherits the matching spectral pair rather than
    /// carrying settings of its own.
    ///
    /// Each group says which renderer draws it, how wide its shadow is and how
    /// dark it lands ([`crate::ShadowStyle`]). A lattice group's width is a
    /// share of node radius; a spectral group's uses the renderer's fixed
    /// four-point edge unit so roll, axis and spiral shadows stay constant on
    /// screen. Half the resolved width is σ (`shadow::sigma_points` in
    /// harmonigraph-render).
    ///
    /// FOUR groups and not one bar because node geometry and the notation that
    /// names its positions want independent shadows. Geometry and text repeat
    /// for the spectral pictures because their compositors use different
    /// colours and paint order; the renderer and the three settings stay the
    /// same.
    ///
    /// Each item multiplies whatever is already in the frame under it in the
    /// painter's order the pass already walks — so what it casts is read off
    /// its own ink rather than off which draw it belongs to, and a nearer item
    /// darkens a farther one wherever they overlap.
    ///
    /// A shadow of the INK and not of a circle around it: a node reaching a
    /// melody mark on one octave casts from that wedge and hugs its rings
    /// everywhere else, the empty middle its rings stand around casts nothing,
    /// and a hairline ring casts a fainter shadow than a wide band does under
    /// the Gaussian. Each layer's own envelope is already in what the cell
    /// holds, so a releasing layer's shadow fades with its ink.
    ///
    /// It is spent on the FRAME rather than on the light alone, and the light
    /// takes it by being under everything: the halos are composited at the
    /// bottom of the scene pass, so a shadow lands on a neighbour's halo, on a
    /// ring behind, and on a name.
    ///
    /// Without it the light is at its brightest exactly where the rings are —
    /// the falloff is measured from the node's centre, so both sides of a ring
    /// sit near the peak — and a ring drawn over that is a flat grey silhouette
    /// on a bright field. A ring standing in a pool that brightens outward is
    /// what the eye reads as the ring being the source of the light.
    ///
    /// It is spent on the frame and never on a caster's own ink, which the draw
    /// leaves unmultiplied — so an item is the one thing its own shadow never
    /// darkens.
    ///
    /// Independent of [`glow_reach`](Self::glow_reach): an item casts with no
    /// light in the picture at all, onto the ground and onto whatever ink
    /// stands behind it.
    pub shadow: ShadowSettings,
    /// How much of the light standing at a LIT slice washes over that slice's
    /// own ink, 0..=1 — a sounding octave indicator, a wedge the analyzer is
    /// reading, and the melody/bass mark that continues one.
    ///
    /// The LIT ink alone. Every other piece of the lattice — a silent slice's
    /// grey, a wedge at the analyzer's pinned silent end, the resting markers
    /// ([`plus_arm`](Self::plus_arm)) between the nodes — takes the field whole
    /// whatever this says, and is not on a bar at all.
    ///
    /// The two halves want opposite things of one field, which is the whole
    /// reason only one of them is dialled. Unlit ink is ground laid over ground
    /// the light is already under, so unwashed it comes out DARKER inside a halo
    /// than beside it and the resting lattice reads as holes punched exactly
    /// where the light is brightest: it wants all of the light, always. A lit
    /// slice is already the colour its own halo is made of, so the field over it
    /// buys no colour and spends the edge between the slice and its light —
    /// dialled up, a node melts into its own glow.
    ///
    /// 1 is one field over the whole node, which is the picture with no bar in
    /// it. Down from there the sounding slices come back out of the light while
    /// the grey around them stays in it.
    ///
    /// The RAW field: an item's own shadow does not darken the light it is
    /// washed with, so a lit slice reads the same whatever the Shadow bars are
    /// doing around it.
    ///
    /// Laid over the ink as a SCREEN, so it can only ever brighten whoever laid
    /// the light down; see `node_paint` in lattice.wgsl for why an over is
    /// wrong over a field several nodes light at once.
    ///
    /// Inert while [`glow_reach`](Self::glow_reach) is 0.
    pub glow_wash: f32,
    /// How widely a node's own ink is averaged into the colour of its light.
    ///
    /// The glow's colour is not a formula naming its sources — it is what the
    /// node is DRAWING, blurred round the node (`ink_at` in lattice.wgsl):
    /// every layer's colour at each angle, weighted by that layer's level there
    /// and by the radial width the stack handed it. So a lit band sector, an
    /// audio wedge and a mark each light the halo in their own colour and in
    /// the proportion they occupy the node, and widening a layer on the Layers
    /// bar moves the light toward its colour with no knob of its own.
    ///
    /// This is how far round that average is taken: at 0 each layer's sectors
    /// stay distinct arcs of colour, and at 1 the whole node's ink averages
    /// into one tint. Inert while [`glow_reach`](Self::glow_reach) is 0.
    ///
    /// A BLEND and not a spread, in the name as on the bar: under the Glow
    /// heading, beside a Reach that is a distance, a "spread" reads as how far
    /// the light goes, and this moves no light at all — only what colour it is.
    pub glow_blend: f32,
    /// Blend from the fixed-peak glow (0) to the original per-channel screen
    /// accumulation (1). Only overlapping halos change; a lone glow is identical.
    pub glow_accumulation: f32,
    /// How fast a node's light follows the node, in seconds: the time constant
    /// of the exponential its LEVEL and its COLOUR are both carried on — this
    /// one while the light is coming up, [`glow_release`](Self::glow_release)
    /// once it is going down.
    ///
    /// Its own pair rather than the note Fade every drawn layer rides, because
    /// the light is not one of those layers. A halo is the slow part of the
    /// picture; stepped with the marks' and the audio ring's own fast
    /// envelopes it flickers with them. On a pair of its own a node's light
    /// LINGERS past the ink that lit it — the node keeps being drawn into the
    /// ink strip while its level is above nothing at all — and its hue morphs
    /// toward a new octave's rather than cutting to it.
    ///
    /// One pair for both halves, which is what keeps the two from disagreeing:
    /// the same coefficient `1 - exp(-dt/tau)` carries the level on the CPU and
    /// the node's own ink on the GPU (`panes::glow_fade` in harmonigraph-ui).
    /// Inert while [`glow_reach`](Self::glow_reach) is 0 — with no light there
    /// is nothing to carry.
    pub glow_attack: f32,
    /// See [`glow_attack`](Self::glow_attack). The slow half, and the one the
    /// look is in: what makes a halo read as light rather than as a layer of a
    /// node is how long it takes to leave.
    pub glow_release: f32,
}

/// A size bar's value as the picture may use it: inside `0..=high`, and 0 —
/// the off position every one of them has — where a hand-edited blob holds a
/// NaN or an infinity, which no clamp of its own would catch.
///
/// Reached from [`derive`](crate::derive) as well as from the stack here,
/// because the answer a size gets when it is not a number has to be one
/// answer: a marker's arm read as NaN in one derivation and as 0 in the next
/// is a picture assembled out of two readings of one bar.
pub(crate) fn size(value: f32, high: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, high)
    } else {
        0.0
    }
}

impl ViewConfig {
    /// The note envelope, assembled from the two halves it is stored in: the
    /// shape is a LOOK and lives here, the duration is host-automatable and
    /// lives in [`FrameParams`].
    ///
    /// ONE duration at both ends, so a note comes up on the time it goes down
    /// on and there is a single number to say how quick the lattice is. It
    /// costs the staccato end nothing, because the two ends are sequenced
    /// rather than overlaid — see
    /// [`release_level`](harmonigraph_core::Voice::release_level), where that
    /// is written and argued.
    ///
    /// One assembly point for every envelope a NOTE runs on, so the split is
    /// invisible past this line and no caller can pair a duration with the
    /// wrong shape. The shape is clamped to the range its bar offers — a
    /// hand-edited blob can hold anything, and `sanitize` only repairs the
    /// non-finite (a finite 40 would be a curve no bar can undo).
    ///
    /// The Note section's Fade curve bar builds one of its own, and it is the single
    /// exception rather than a second assembly point: it is drawing a PICTURE
    /// of the curve, over a unit duration that is nothing a note ever fades
    /// on, so it has no note's seconds to pair with and could not reach for
    /// them here. What it must not do is re-derive the SHAPE, and
    /// `the_shape_bars_preview_is_the_curve_the_notes_run_on` is what holds
    /// it to this function's answer.
    pub fn envelope(&self, frame: &FrameParams) -> Envelope {
        Envelope {
            attack_time: frame.fade_time,
            fade_time: frame.fade_time,
            shape: self.fade_shape.clamp(0.0, 1.0),
        }
    }

    /// Whether a melody/bass mark can be drawn at all: the depth has to leave
    /// it something to draw with.
    ///
    /// Says nothing about whether a mark is drawn NOW — that is a held note's
    /// business, per node. This is whether the layer is switched on, which is
    /// what the pane grays its Delay bar on: a mark that cannot appear has
    /// nothing for a delay to time.
    pub fn marks_draw(&self) -> bool {
        self.mark_thickness > 0.0
    }

    /// Where every layer of a node lands, read outward from its center: the
    /// four size bars turned into the radii everything that draws a node wants
    /// (see [`RingStack`]).
    ///
    /// One function because two callers have to agree on it and they are in
    /// different crates' reach — `derive_scene` builds the MIDI picture's
    /// radii, [`SpectralPaint::new`](crate::SpectralPaint) the audio ring's,
    /// and the audio ring's inner edge is a sum over the layers inside it. Two
    /// copies of that sum is how a ring comes to sit a gap off a band that
    /// moved.
    ///
    /// Every clamp the picture needs is here rather than in
    /// [`sanitize`](Self::sanitize), for the reason every other geometry clamp
    /// is: the drawing code is reached by more routes than the persist door — a
    /// take replay, the offline renderer's layout, a standalone harness — so a
    /// hand-edited blob has to come out as a node somebody can see rather than
    /// as one that silently is not there.
    pub fn rings(&self) -> RingStack {
        let gap = size(self.ring_gap, GAP_MAX);
        let inner = size(self.ring_inner, RING_INNER_MAX);
        // The cursor is the outer edge of the last layer DRAWN, and 0 until one
        // is — which is what makes a ring dialled to 0 cost its gap as well as
        // its slot: nothing moved the cursor, so the next ring starts where it
        // would have. The start is carried beside the cursor rather than as its
        // opening value, because the two answer different questions: what a
        // layer stands off, and where the stack sits.
        let mut stack = Stack { inner, cursor: 0.0, reach: 0.0, full: false };
        let audio = stack.take(gap, size(self.spectral_ring_width, RING_WIDTH_MAX));
        let band = stack.take(gap, size(self.band_width, RING_WIDTH_MAX));
        RingStack {
            inner,
            audio,
            band,
            outer: stack.cursor,
            // The strip's own slot, on the stack's terms — a gap out from how
            // far the stack REACHED, or the stack's start when it reached
            // nowhere. Only the outer edge is left to the renderer, because
            // that is the one the billboard's margin lets run past the quad.
            //
            // The reach and not the cursor, which is the whole of what keeps
            // the strip travelling the same way as the handle. The marks are
            // the one layer never refused — their slot is allowed past the
            // quad — so `Stack::full` cannot stop them the way it stops a
            // ring, and seating them on the cursor handed them the slot a
            // refused band had just given up. That is the gift `full` exists
            // to refuse, arriving by the one door it does not cover: the strip
            // jumped a fifth of a node INWARD as the Inner handle moved out.
            mark_inner: slot_start(stack.reach, inner, gap),
            mark_thickness: size(self.mark_thickness, MARK_THICKNESS_MAX),
            gap,
        }
    }

    /// [`ring_gap`](Self::ring_gap) as the angular width the shader can cut
    /// with: on the axis, and a real number.
    ///
    /// Unlike the radial use through [`rings`](Self::rings), this reaches the
    /// picture as a bare uniform rather than through a radius, so this is the
    /// one place its clamp can live, and it is here rather than in
    /// [`sanitize`](Self::sanitize) for the reason every other geometry clamp
    /// is: the drawing code is reached by more routes than the persist door. A
    /// non-finite width would threshold every fragment of every sector to
    /// false, taking the whole octave layer off the node with nothing on screen
    /// to say why.
    pub fn octave_gap_width(&self) -> f32 {
        size(self.ring_gap, GAP_MAX)
    }

    /// [`lattice_ground`](Self::lattice_ground) as an `L*` the colour path can
    /// actually solve for: on the axis, and a real number.
    ///
    /// One function for the same reason [`rings`](Self::rings) is one: the two
    /// layers standing on this ground resolve it in different crates' reach —
    /// `derive_scene` for the octave band, [`SpectralPaint::new`](crate::SpectralPaint)
    /// for the audio ring's table — and a ground repaired two ways is two
    /// grounds. The repair
    /// is here rather than in [`sanitize`](Self::sanitize) alone for that
    /// function's own reason: the drawing code is reached by more routes than
    /// the persist door, and a NaN walks through a `clamp` untouched into a
    /// Newton solve that answers with whatever its guard parks on.
    pub fn lattice_ground_lightness(&self) -> f32 {
        if self.lattice_ground.is_finite() {
            self.lattice_ground.clamp(0.0, 100.0)
        } else {
            DEFAULT_RING_GROUND
        }
    }

    /// [`marker_ink`](Self::marker_ink) as an `L*` the colour path can actually
    /// solve for: on the axis, and a real number.
    ///
    /// Its own function rather than the one above with a field swapped in,
    /// because the two numbers are independent and a repair that read the wrong
    /// one would be silent: both answers are drawable neutral greys, so only a
    /// comparison against the bar exposes the swap. The reason the repair
    /// exists at all is
    /// [`lattice_ground_lightness`](Self::lattice_ground_lightness)'s: a NaN
    /// walks through a `clamp` untouched into a Newton solve that answers with
    /// whatever its guard parks on, and the drawing code is reached by more
    /// routes than the persist door.
    pub fn marker_ink_lightness(&self) -> f32 {
        if self.marker_ink.is_finite() {
            self.marker_ink.clamp(0.0, 100.0)
        } else {
            DEFAULT_MARKER_INK
        }
    }

    /// Active labels are always neutral white.
    pub fn active_label_lightness(&self) -> f32 {
        100.0
    }

    /// Whether the audio ring is drawn at all: a width to draw it with, and
    /// room left inside the quad to draw it in.
    ///
    /// The LAYER's own switch, and the whole of it —
    /// [`spectral_reading`](Self::spectral_reading) says which of two readings
    /// fills the annulus, never whether there is one, and
    /// [`spectral_ring_gate`](Self::spectral_ring_gate) says which nodes wear
    /// what this turns on.
    pub fn spectral_ring_draws(&self) -> bool {
        let (inner, outer) = self.rings().audio;
        outer > inner
    }

    /// The window's center as a lattice position: the node drawn at the world
    /// origin, and what the reach is centered on.
    pub fn center(&self) -> LatticePos {
        LatticePos::new(self.center_threes, self.center_fives, self.center_sevens)
    }

    /// The commas being tempered out, as the set a name is spelled against
    /// ([`LatticePos::respell`]). The flags are stored one per comma so a
    /// saved project keeps reading, and this is where they become the one
    /// value every naming path takes.
    pub fn tempered(&self) -> Tempered {
        Tempered { syntonic: self.meantone, septimal_kleisma: self.marvel }
    }

    /// Whether one comma is being tempered out.
    pub fn tempers(&self, comma: Comma) -> bool {
        match comma {
            Comma::Syntonic => self.meantone,
            Comma::SeptimalKleisma => self.marvel,
        }
    }

    /// Whether one comma's auto-detect is running.
    pub fn temper_auto(&self, comma: Comma) -> bool {
        match comma {
            Comma::Syntonic => self.meantone_auto,
            Comma::SeptimalKleisma => self.marvel_auto,
        }
    }

    /// The switch for one comma's tempering, to read or set. Together with
    /// [`Self::temper_auto_mut`] this is what lets the tempering section be a
    /// loop over [`Comma::ALL`] instead of a block per comma.
    ///
    /// A third comma is then additive rather than another special case, but
    /// it is not free: the variant and its arms on [`Comma`], two fields and
    /// four arms here, one in `LatticePos::respell`, one in the UI's
    /// `judged_axes`, and one in its `derived_key` — which lives there
    /// because a `ParamKey` is the UI's to name, not core's.
    pub fn temper_mut(&mut self, comma: Comma) -> &mut bool {
        match comma {
            Comma::Syntonic => &mut self.meantone,
            Comma::SeptimalKleisma => &mut self.marvel,
        }
    }

    /// The auto-detect switch for one comma.
    pub fn temper_auto_mut(&mut self, comma: Comma) -> &mut bool {
        match comma {
            Comma::Syntonic => &mut self.meantone_auto,
            Comma::SeptimalKleisma => &mut self.marvel_auto,
        }
    }

    /// Fit a deserialized view to what its controls can actually produce.
    ///
    /// A bar cannot produce a nonsense value but a hand-edited RON can, and
    /// these feed a rasterizer.
    ///
    /// This repairs a value that is PRESENT and unusable — a NaN, an infinity,
    /// something past what its bar can reach. A key that is missing outright
    /// never arrives here at all: the container-level `#[serde(default)]`
    /// has already filled it from `impl Default`, which is the fresh view's
    /// value and the whole of that arrangement.
    ///
    /// Most repairs fall back to the fresh view's own value, which is the only
    /// other value in the file known to be drawable. `fade_shape` and
    /// `mark_delay` are the exception and land on 0 — both have a 0 that MEANS
    /// something (straight, no wait), so a blob carrying a nonsense number for
    /// one gets the inert setting rather than a look nobody asked for. It
    /// reads as a feature switched off, which is what a broken number should
    /// look like. Fresh, neither is 0.
    pub fn sanitize(&mut self) {
        let fresh = ViewConfig::default();

        // The window's own integers, which are the one group here that is not
        // a float. `DrawnWindow::count` multiplies the three spans together
        // and `reach` adds each center to its extent, so a blob carrying a
        // billion sheets overflows both — and the derived window now counts
        // nodes on every draw, which puts that arithmetic in the frame rather
        // than at the edge of it. The sevens ends are held to what their strip
        // offers.
        //
        // The other two are the naming REACH, and they are floored at the
        // fresh sizing rather than at zero. Nothing structural keeps the reach
        // and the drawn window in step — one is a setting, the other the
        // camera's — so a reach from a build that sized it smaller makes every
        // pitch out past it fall to the slower of the two naming paths, and in
        // a tuning that collapses, to a spelling chosen out of a window it was
        // never tuned for. No bar sets these, so there is no dialled-down
        // value to respect and no readout for a floor to contradict: raising
        // it is free. `a_loaded_view_never_draws_a_node_its_reach_cannot_name`
        // holds the cabinet case, which is the one the sizing is FOR.
        // The two ends onto the strip's axis, low end first so an INVERTED
        // pair comes out closed rather than silently swapped: a blob holding
        // `max < min` is one nobody dragged, and the sheet it agrees to draw
        // is the one its low end names.
        self.min_sevens = self.min_sevens.clamp(-SEVENS_LAYER_LIMIT, SEVENS_LAYER_LIMIT);
        self.max_sevens = self.max_sevens.clamp(self.min_sevens, SEVENS_LAYER_LIMIT);
        self.extent_threes = self.extent_threes.clamp(fresh.extent_threes, MAX_DRAWN_EXTENT);
        self.extent_fives = self.extent_fives.clamp(fresh.extent_fives, MAX_DRAWN_EXTENT);
        // The seventh center is the one center exposed as a setting, and it
        // names a sheet the picture DRAWS — so it is held inside the pair
        // above rather than merely on the same axis. A home sheet outside the
        // stack is one no node is on, and `sevens_size` measures every sheet
        // against it: nothing would come out at full size, and the sheet the
        // music is heard against would be off screen.
        //
        // The camera-owned fifths/thirds centers retain the arithmetic guard.
        self.center_sevens = self.center_sevens.clamp(self.min_sevens, self.max_sevens);
        self.center_threes = self.center_threes.clamp(-MAX_CENTER, MAX_CENTER);
        self.center_fives = self.center_fives.clamp(-MAX_CENTER, MAX_CENTER);

        // Fit the label scale to what its bar offers. It multiplies a FONT
        // SIZE, and the bar cannot produce a nonsense value where a
        // hand-edited blob can: a non-finite one reaches egui as a glyph with
        // no image, so every label silently vanishes, and a huge one asks the
        // rasterizer for a glyph wider than the texture atlas can hold.
        self.label_scale = if self.label_scale.is_finite() {
            self.label_scale.clamp(*SCALE_BAR_RANGE.start(), *SCALE_BAR_RANGE.end())
        } else {
            fresh.label_scale
        };

        // The pitch gradient's six knobs, for the same reason as the label
        // scale above and one more: they are the memo key of the color table
        // every pitch-colored shape reads, so a non-finite one would miss the
        // cache on every lookup as well as drawing a NaN.
        self.pitch_gradient = self.pitch_gradient.sanitized();

        // Together, because the pair is what has to fit the boundary table and
        // either one alone can be legal in a wheel that isn't.
        (self.octave_count, self.octave_extras) =
            crate::octaves::clamp_wheel(self.octave_count, self.octave_extras);
        self.octave_center = crate::octaves::clamp_center(self.octave_center);

        // The fringe feeds the wheel's boundary angles, and a non-finite size
        // or blend poisons every one of them: the widths come out NaN, so does
        // each `cos`/`sin` in the shader, and the whole octave layer vanishes
        // with nothing to say why. `clamp` alone does not catch it — NaN is
        // its own answer — hence the finite check either side of it.
        self.octave_extra_size = if self.octave_extra_size.is_finite() {
            self.octave_extra_size.clamp(crate::octaves::MIN_EXTRA_SIZE, 1.0)
        } else {
            fresh.octave_extra_size
        };
        self.octave_extra_blend = if self.octave_extra_blend.is_finite() {
            self.octave_extra_blend.clamp(0.0, 1.0)
        } else {
            fresh.octave_extra_blend
        };

        // Each off-sheet step multiplies geometry by this value. The draw path
        // stays defensive, while load owns making the stored reading fit the
        // bar that edits it.
        self.sevens_size = finite_or(self.sevens_size, fresh.sevens_size).clamp(0.15, 1.0);

        // The mark delay, against that same hole: it is added to a timestamp
        // and the sum divided by the attack, so a non-finite one poisons the
        // ease of every ring. The symptom is the rings VANISHING, not drawing
        // wrong — a NaN level fails `Mark::add`'s `>=` (NaN answers no to
        // every comparison), so the level stays at 0 while the slot bit is
        // still set, and the shader multiplies the ring's coverage away to
        // nothing. Silent, and it takes the whole layer wherever the marks
        // are on. `derive_scene` retains the RANGE clamp for callers that do
        // not pass through this load boundary; this door also catches NaN.
        self.mark_delay = finite_or(self.mark_delay, 0.0).clamp(0.0, crate::MARK_DELAY_MAX);

        // The envelope's shape, against the same hole. `Envelope::approach`
        // guards its own arithmetic against a non-finite duration or shape —
        // it has to, being reachable from a shell that never went through
        // this door — but it guards by treating the transition as already
        // OVER, so a NaN shape would show as a curve that silently
        // straightens: the picture quietly drawing something other than what
        // the bar reads out, which is exactly what this door is for. The
        // duration beside it is the Fade param rather than a blob field, and
        // has no door here to need.
        self.note_animation = self.note_animation.sanitized();
        self.fade_shape = finite_or(self.fade_shape, 0.0).clamp(0.0, 1.0);

        // The spectral kernel's width, against that same hole and one more: it
        // is a DIVISOR in the fold's Gaussian, so a 0 from a hand-edited blob
        // makes every weight a NaN and the whole lattice reads as silent —
        // dark, with nothing to say why. The clamp is what keeps that
        // impossible; the finite check is what a clamp cannot be.
        self.spectral_width = finite_or(self.spectral_width, fresh.spectral_width)
            .clamp(crate::SPECTRAL_WIDTH_MIN, crate::SPECTRAL_WIDTH_MAX);

        // The audio ring's width, against that same hole. A non-finite one
        // costs less than the width above — [`ViewConfig::rings`] reads a NaN
        // as the off position rather than letting it through as a radius — but
        // what it costs is the SETTING: the ring is not drawn while the bar
        // reads out a number, and dragging the bar is then the only way to find
        // out that the number was never a size. Repaired to the fresh width, so
        // the field and the picture agree about which it is.
        //
        // The fresh width is now 0 — the DAW capture at `64f7d41e` dialled the
        // ring off — so this repair and `rings`' own reading of a NaN land in
        // the same place today, and the sentence that used to stand here (a
        // blob through this door "holds a ring somebody can see") stopped being
        // true then. What the repair still buys is the agreement rather than
        // the ring: the stored field stops being a number no layer matches.
        // Should the fresh ring ever come back on, this line follows it.
        //
        // Where the ring SITS is not repaired here, because it is not stored: a
        // width is a width whatever is inside it, and the stack is what turns
        // the four of them into radii (`rings`).
        self.spectral_ring_width = finite_or(self.spectral_ring_width, fresh.spectral_ring_width)
            .clamp(0.0, RING_WIDTH_MAX);
        // The three handles BESIDE it on the same bar, against the same hole
        // and repaired for the same reason. Each is held to its own ceiling in
        // [`rings`](Self::rings) for the picture, and the Layers bar reads the
        // stored field back raw, so a blob past a ceiling leaves one handle out
        // on the axis at a width no layer of the node matches — the ring's own
        // case, three more times, and the one the bar is least able to report
        // since the stack it draws under the handles is already the clamped one.
        //
        // To the fresh value rather than to 0, which is the ring's rule and not
        // the delay's: 0 is a legal width on all four handles, but a layer
        // silently absent is no safer a reading of a broken number than a layer
        // at the wrong size, and the fresh stack is the one arrangement in the
        // file known to seat all four.
        self.ring_inner = finite_or(self.ring_inner, fresh.ring_inner).clamp(0.0, RING_INNER_MAX);
        self.band_width = finite_or(self.band_width, fresh.band_width).clamp(0.0, RING_WIDTH_MAX);
        self.mark_thickness =
            finite_or(self.mark_thickness, fresh.mark_thickness).clamp(0.0, MARK_THICKNESS_MAX);
        // The shared padding, against the same hole and for the reason the width
        // above is repaired rather than left to the picture: [`GAP_MAX`] is a
        // ceiling the bar is BUILT from, so a blob written when it stood
        // higher carries a number no bar can reach, and `rings` holds it to the
        // ceiling for the picture while the field keeps what the bar reads out.
        // The stack under the bar then draws one padding and the bar names
        // another, which is a value read out one way and drawn another.
        //
        // The clamp in [`rings`](Self::rings) stays where it is — the picture is
        // reached by more routes than this door — and this one makes the number
        // the door lets through a number the picture agrees with.
        self.ring_gap = finite_or(self.ring_gap, fresh.ring_gap).clamp(0.0, GAP_MAX);
        // How wide a window each wedge shows. A MULTIPLIER in the shader — a
        // fragment's across-the-wedge fraction scales by it into a cents
        // offset — so a zero from a hand-edited blob is finite but degenerate:
        // every fragment of a wedge reads the slot's own pitch and the ring
        // collapses to one flat reading per wedge. The floor forbids that
        // zoom; [`SPECTRAL_RANGE_MIN`](crate::SPECTRAL_RANGE_MIN) says where
        // it sits and why.
        self.spectral_ring_range = finite_or(self.spectral_ring_range, fresh.spectral_ring_range)
            .clamp(crate::SPECTRAL_RANGE_MIN, crate::SPECTRAL_RANGE_MAX);
        // The gate, repaired to its OFF position rather than to the fresh value
        // the two above take: a level nobody can read is a reason to draw every
        // ring, never to hide one, and a blob holding a NaN here would
        // otherwise open on a lattice with no rings and no way to tell that
        // from an analyzer with nothing to say. `SpectralPaint::new` repairs
        // the same way for the shells that never come through this door.
        self.spectral_ring_gate = finite_or(self.spectral_ring_gate, crate::SPECTRAL_GATE_MIN)
            .clamp(crate::SPECTRAL_GATE_MIN, crate::SPECTRAL_GATE_MAX);
        // The hysteresis repairs to 0 — one threshold — on the same argument
        // the gate repairs to its floor: a band nobody can read is a reason to
        // fall back to the simpler rule, never to hold rings open on a number
        // that came out of a corrupt blob.
        self.spectral_ring_hysteresis = finite_or(self.spectral_ring_hysteresis, 0.0)
            .clamp(0.0, crate::SPECTRAL_HYSTERESIS_MAX);
        // Times repair to the fresh pair rather than to zero: zero is a legal
        // setting (no smoothing) but it is not the safe reading of a broken
        // one, since it puts the flicker back with nothing on screen saying so.
        self.spectral_ring_attack =
            finite_or(self.spectral_ring_attack, fresh.spectral_ring_attack)
                .clamp(0.0, crate::SPECTRAL_BALLISTICS_MAX);
        self.spectral_ring_release =
            finite_or(self.spectral_ring_release, fresh.spectral_ring_release)
                .clamp(0.0, crate::SPECTRAL_BALLISTICS_MAX);

        // The ground both rings stand on, against that same hole. It is an
        // `L*`, so the clamp is the axis itself: off either end the Newton
        // solve behind a neutral grey is asked for a luminance sRGB does not
        // hold, and a non-finite one takes the ANALYZER's ramp with it — the
        // audio ring's table is re-anchored to open here, so a NaN ground is a
        // NaN gradient and the whole ring goes to whatever the clamp in
        // `oklab_srgb` lands on. Both rings read the repaired number, which is
        // what keeps the bar's readout and the grey on screen the same value.
        self.lattice_ground =
            finite_or(self.lattice_ground, fresh.lattice_ground).clamp(0.0, 100.0);
        // The markers' own grey, on the same axis and repaired for the same
        // reason: it is solved for a neutral by the same Newton solve, and it
        // reaches no gradient, so a broken one costs the resting field and
        // nothing else.
        self.marker_ink = finite_or(self.marker_ink, fresh.marker_ink).clamp(0.0, 100.0);
        // The node glow's pair. The reach repairs to the fresh value — 0, the
        // off position — on the same argument the ring's gate does: a number
        // nobody can read is a reason to draw no halo, never to open one over
        // the whole lattice out of a corrupt blob. The strength rides with it
        // and repairs to its own fresh value, being inert while the reach is 0.
        //
        // The reach is what the billboard is SIZED on (`quad_margin` in
        // lattice.wgsl), so a non-finite one is not merely a wrong halo: it is
        // a NaN quad, and every node's glow vanishes with nothing on screen to
        // say why.
        self.glow_reach = finite_or(self.glow_reach, fresh.glow_reach).clamp(0.0, GLOW_REACH_MAX);
        self.glow_strength =
            finite_or(self.glow_strength, fresh.glow_strength).clamp(0.0, GLOW_STRENGTH_MAX);
        self.glow_curve = self.glow_curve.sanitized();
        self.atmosphere = self.atmosphere.sanitized();
        // Every Shadow group, over the groups the settings enumerate rather
        // than by name, so a group added at step 7 arrives sanitized. The width
        // is what every caster's quad is grown by — a number from outside the
        // bar is a quad nothing can fill — and the depth is a SHARE of the
        // frame, so the unit interval. Falloff is a signed bend around zero
        // (linear), with the same fixed reach throughout its range.
        for (style, fresh) in self.shadow.groups_mut().into_iter().zip(fresh.shadow.groups()) {
            style.width = finite_or(style.width, fresh.width);
            style.depth = finite_or(style.depth, fresh.depth);
            style.falloff = finite_or(style.falloff, fresh.falloff);
        }
        self.shadow = self.shadow.clamped();
        // The SHARES — of the light a lit slice stands in, of the light's own
        // peak, of a whole turn — so their range is the unit interval.
        self.glow_wash = finite_or(self.glow_wash, fresh.glow_wash).clamp(0.0, 1.0);
        self.glow_blend = finite_or(self.glow_blend, fresh.glow_blend).clamp(0.0, 1.0);
        self.glow_accumulation =
            finite_or(self.glow_accumulation, fresh.glow_accumulation).clamp(0.0, 1.0);
        // The light's own pair, in seconds, on the ring's rule: a bar's range,
        // and a poisoned number repaired to the fresh value rather than left
        // to make a coefficient nothing can carry.
        self.glow_attack =
            finite_or(self.glow_attack, fresh.glow_attack).clamp(0.0, GLOW_BALLISTICS_MAX);
        self.glow_release =
            finite_or(self.glow_release, fresh.glow_release).clamp(0.0, GLOW_BALLISTICS_MAX);

        // These post-process controls are stored beside the view. Renderer
        // clamps remain wider defensive boundaries for callers that do not
        // load an AppearanceDocument through this sanitizer.
        self.render_scale = finite_or(self.render_scale, fresh.render_scale).clamp(0.5, 2.0);
        self.bloom_strength = finite_or(self.bloom_strength, fresh.bloom_strength).clamp(0.0, 2.0);

        // The resting marker's three lengths. The arm and its taper are a
        // reach-and-fade PAIR, held the way every such pair here is — the fade
        // clamped to its own reach — because `edge_bar` puts `reach - taper` on
        // the axis and a taper wider than its arm would show a low end the
        // value does not say. `derive_pluses` clamps the reach again for the
        // PICTURE, which is a separate job.
        //
        // The width is clamped to the axis and NOT to the arm: past twice the
        // arm it draws a filled square, which is a picture rather than an
        // error, and holding it to the arm here would drag a dialled width down
        // whenever the arm bar was pulled in.
        self.plus_arm = finite_or(self.plus_arm, fresh.plus_arm).clamp(0.0, PLUS_SIZE_MAX);
        self.plus_width = finite_or(self.plus_width, fresh.plus_width).clamp(0.0, PLUS_SIZE_MAX);
        self.plus_taper = finite_or(self.plus_taper, fresh.plus_taper).clamp(0.0, self.plus_arm);
    }
}

/// `value` if it is a real number, and `fallback` if it is a NaN or an
/// infinity — the guard `clamp` cannot be, NaN being its own answer to every
/// comparison a clamp makes.
///
/// No range: the caller's own clamp is the range, and this only has to hand
/// it something a clamp can act on.
///
/// Reached from [`derive`](crate::derive) and [`style`](crate::style) as well
/// as from the door below, for the same reason [`size`] is: the drawing code
/// is reached by more routes than the persist door — the offline layout, take
/// replay and the harness each build a view in code — and one answer to "this
/// is not a number" is what keeps the picture from being assembled out of two
/// readings of one bar.
pub(crate) fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The `L*` a fresh [`ViewConfig::lattice_ground`] opens on. Named because the
/// `_lightness` accessor needs it without building a whole fresh view to read
/// one field off. Named, and not a second value: the `Default` below is written
/// in terms of it, the way it is written in terms of `octaves::DEFAULT_COUNT`.
///
/// Where on the chrome's ladder this grey sits is said at the `Default` below.
const DEFAULT_RING_GROUND: f32 = 6.0;

/// The `L*` a fresh [`ViewConfig::marker_ink`] opens on. Kept beside the ring
/// ground because the accessors repair the two independently without building
/// a fresh view to read either field.
const DEFAULT_MARKER_INK: f32 = 32.0;

/// The look a fresh view starts in, and the single source of every field's
/// fallback: the container-level `#[serde(default)]` on the struct means a
/// blob missing a key picks its value up from here.
impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            // The naming reach: how far out a played pitch is hunted for a
            // spelling before it counts as off the lattice. Oblong, like the
            // panes it has to cover — `lattice_to_world` puts the FIFTHS axis
            // on world x, so that is the one running across the screen and the
            // one the width is spent on.
            //
            // Sized to hold the whole of what a CABINET pane shows, at every
            // zoom, up to a 16:9 frame — which is the projection this matters
            // under and the shape a render is. It does NOT hold what the other
            // two draw: a flat 16:9 perspective pane at the zoom limit draws
            // 3825 nodes against this window's 1025, so most of that picture
            // is out here. Covering it would mean naming a pitch sixty fifths
            // out, which is a spelling nobody wants, on a walk this wide per
            // played pitch per frame. So the reach stays a reach, and the
            // readers that describe the PICTURE ask the picture's own window
            // instead — see `SharedState::shown`.
            extent_threes: 12,
            extent_fives: 20,
            // The home sevens sheet alone — the strip closed on its one cell.
            // A sheet either side shows the septimal axis without anyone
            // having to go find it; the tradeoff is that nothing tells the eye
            // which sheet a node is on until the sevens layer settings below
            // are turned down to read as an annotation rather than a second
            // sheet (see sevens_size).
            min_sevens: 0,
            max_sevens: 0,
            center_threes: 0,
            center_fives: 1,
            center_sevens: 0,
            // Sevens sheets at full size, which rides along inert while the
            // axis is collapsed above. At full size a sheet rivals the home
            // one rather than annotating it — nothing says which sheet a node
            // is on and an off-sheet label lands on its neighbours — so
            // opening depth is also the cue to bring this down (around 0.55
            // reads as an annotation). Its label keeps the name, which the
            // septimal mark spells apart from the node two fifths down (see
            // SevensLabel) rather than repeating it.
            sevens_size: 1.0,
            sevens_label: SevensLabel::Name,
            // Where the music has been is most of what the lattice is for, so
            // the fresh view opens naming it: every visited node keeps its
            // name, and none of the unvisited ones carry text yet.
            note_names: NoteNames::Past,
            // The selected map reads directly from the lattice by default;
            // the Tuning pane can hide this editor annotation without
            // changing which map tunes new notes.
            show_map_indicators: true,
            // Effectively the built-in size (1) — where the marks and the
            // cents line are proportioned against, so this bar sizes the
            // whole label together.
            label_scale: 1.002_336,
            show_cents: true,
            // Written out rather than taken from `Gradient::default()`,
            // which is the gradient TYPE's own default — the CIELAB arc
            // converted, which `the_defaults_are_the_retired_arc_converted`
            // holds it to, and what a gradient assembled in code opens on.
            // The composed look is free to differ, and does: a shorter arc,
            // a brighter middle over a steeper brightness ramp, and a little
            // less chroma.
            pitch_gradient: Gradient {
                hue_start: 257.842_65,
                hue_span: 170.496,
                // Brighter in the middle and over a much steeper ramp than
                // the arc opened on (53.0 over 31.0): the low end stays dark
                // enough to sit back while the top of the range carries real
                // light, which is what separates octaves at a glance.
                lightness: 60.5,
                lightness_ramp: 65.0,
                // Denominated in the floor every hue can hold (see
                // `chroma_of`), which is a tighter axis than the per-hue
                // ceiling. Retuning the type's own `default_chroma` does not
                // reach here — the two are independent numbers, which is the
                // point of writing this out.
                chroma: 0.825_000_05,
                // A slight ramp, no longer flat: the brightness ramp above it
                // is steep enough that the bright end would read washed
                // without a little more color under it. Small on purpose —
                // the hue arc is still what spends color on pitch, and a
                // ramp that competed with it would take one end of the range
                // grey.
                chroma_ramp: 0.069_999_99,
            },
            // A narrow octave band, stopping short of the quad edge, with a
            // tight gap everywhere: the octaves read as a ring of distinct
            // marks rather than a solid annulus, and every layer keeps clear
            // space around it. (The backdrop that holds the whole ring's shape
            // behind them is fixed on.)
            band_width: 0.188_422_56,
            // The stack sits about seven tenths of the way out, leaving a broad
            // middle for the glow's field while the octave band reads as the
            // node's perimeter. Dialled to 0 the stack seats on the center and
            // its wedges close into pie slices, which is the same node read as
            // one solid measurement.
            ring_inner: 0.703_436_8,
            // One gap on both axes: the radial padding is what puts the band at
            // its outer edge, and the same width cut angularly is the slicing
            // that reads as distinct marks.
            ring_gap: 0.05,
            // Dark ground captured from the DAW on 2026-09-13. The ring and
            // resting marker ink stay separate from the editor chrome.
            lattice_ground: DEFAULT_RING_GROUND,
            // Well above the ring ground — 26 `L*` clear of it — so the
            // resting positions stay legible through the broad glow without
            // competing with a sounding node's white name.
            marker_ink: DEFAULT_MARKER_INK,
            // Seven full-size octaves to the turn with middle C straight up —
            // the keyboard's C0..C6 span in the DAW's numbering, with no
            // smaller fringe at either end.
            octave_count: crate::octaves::DEFAULT_COUNT,
            octave_center: crate::octaves::DEFAULT_CENTER,
            octave_extras: 0,
            octave_extra_size: 0.387_534_47,
            octave_extra_blend: 0.562_241_4,
            // The fold, which is the reading to look at a screenful of nodes
            // with (see [`SpectralReading`]) — and the one to meet the ring on
            // first, a lattice of constellations being what the whole layer is
            // for. The zoomed one is a drag away, and it is about a single
            // node.
            spectral_reading: SpectralReading::Fold,
            // Narrow, for the just-tuned material this is aimed at — see the
            // field.
            spectral_width: 2.088_490_2,
            // Parked at the floor so the center and octave band spend the
            // node's radial budget; Fold still feeds the node glow through the
            // analyzer reading independently of this width.
            spectral_ring_width: 0.0,
            // A narrow wedge — see the field for why a window this size and
            // not the octave that makes the ring continuous. It only zooms the
            // Spectrum reading's wedge, so it rides inert while Fold is
            // selected.
            spectral_ring_range: 10.0,
            // A share of the Level window rather than a dB deliberately, so
            // what it says is "no ring dimmer than this much of the ramp" and
            // moving the window moves the gate with the colours it is
            // judging. Permissive at the top end on purpose: hiding a ring
            // that had something to show is the failure a person cannot see,
            // where too many rings is one they can, and the bar is right
            // there.
            spectral_ring_gate: 0.299_119_53,
            spectral_ring_hysteresis: 0.096_396_71,
            // Fast up, slow down. A quarter second of release is long against
            // the 8 ms the analyzer measures on and short against a phrase, so
            // a partial reads as present for as long as it is sounding and the
            // haze between partials stops twinkling.
            spectral_ring_attack: 0.030,
            spectral_ring_release: 0.250,
            // Near enough a square law (the exponent lands at 1.94): enough
            // that a release leaves promptly and settles instead of sliding
            // out at one rate, and not so much that the tail is over before
            // the ear has finished the note. The straight line is still one
            // drag away.
            fade_shape: 0.313_509_55,
            note_animation: NoteAnimationConfig::default(),
            // A shallow step past the band — about a third of the band's own
            // width, so a mark reads as its slice carrying on rather than as a
            // second ring around everything.
            mark_thickness: 0.067_234_814,
            // Just past a passing sixteenth (125ms at 120bpm), and well off
            // the bar's 0 floor: a mark outlives its key (see
            // `mark_delay`), and at 0 every momentary crowning fades its way
            // OUT over the whole Fade, so lifting a chord one key at a time
            // leaves a fading mark on nearly every note of it.
            mark_delay: 0.129_772_4,
            // Arms reaching about half way from the crossing to the ring stack
            // (`ring_inner`, in the same UV): the resting lattice reads as
            // separate crosses with ground between them rather than as a
            // near-continuous mesh. Captured from the DAW on 2026-09-07, with
            // the two below.
            plus_arm: 0.305_142_85,
            // A hairline stroke, so the crosses stay marks rather than blocks
            // through the node glow field.
            plus_width: 0.045_857_143,
            // About two thirds of each arm is taper, so the marker arrives at
            // a fine point rather than carrying its width to the tip.
            plus_taper: 0.195_142_84,
            // The fresh 12-TET tuning satisfies both comma identities, so the
            // spelling locks open on the tuning's own equivalences rather than
            // showing duplicate comma spellings.
            meantone: true,
            meantone_auto: true,
            marvel: true,
            marvel_auto: true,
            frameless: false,
            show_perf: false,
            show_perf_detail: false,
            render_scale: 1.0,
            // A halo at not quite two thirds strength, down from the four
            // fifths the view opened on: a node's rings are quiet shapes and
            // the bloom is what gives them presence, while the glow beside it
            // shares that job — see `glow_blend` and `glow_wash` below.
            bloom_strength: 0.633_927_7,
            // A reach spanning several lattice steps turns each node's light
            // into a shared field, at just over half strength as captured
            // from the DAW on 2026-09-13.
            glow_reach: 4.795_308,
            atmosphere: AtmosphereSettings::default(),
            glow_strength: 0.570_992_95,
            glow_curve: GlowCurve::default(),
            // Four groups at four styles, which is the picture as captured
            // from the DAW: the numbers themselves live in `impl Default for
            // ShadowSettings`, that being the one source of a persisted
            // group's fallback.
            shadow: ShadowSettings::default(),
            // About two fifths of the field, where the picture opened on the
            // whole of it: a sounding slice is pulled back out of its own halo
            // and reads as ink rather than as light, while the resting grey
            // around it still wears what it stands in. That is exactly what
            // the bar is for, and the fresh view now uses it.
            glow_wash: 0.395_398_86,
            // Two thirds of the way round: an octave's arc is softened well
            // into its neighbours rather than cut against them, which is what
            // turns the overlapping fields of a chord into one light instead
            // of a stack of coloured rings.
            glow_blend: 0.669_761_9,
            // The fixed-peak glow alone: with the blend above carrying a
            // chord's halos into each other, the per-channel screen
            // accumulation was adding brightness where they meet on top of a
            // union that already reads as one light.
            glow_accumulation: 0.0,
            // Slow and fluid, which is what the pair is for: a light that
            // arrives inside a third of a second and takes a couple of seconds
            // to leave, so a halo trails the notes that lit it instead of
            // stepping with them.
            glow_attack: 0.3,
            glow_release: 2.5,
        }
    }
}
