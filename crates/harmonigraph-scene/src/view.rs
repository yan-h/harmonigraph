//! [`ViewConfig`] — the persisted, non-automatable visual settings — plus the
//! per-frame [`FrameParams`] mirror of the host-automatable appearance
//! parameters.

use crate::spectral::SpectralReading;
use crate::style::{Gradient, NoteNames, Pulse, SevensLabel};
use crate::{
    Camera, ShadowKernel, GAP_MAX, GLOW_BALLISTICS_MAX, GLOW_CURVE_SHAPE_MAX, GLOW_CURVE_SHAPE_MIN,
    GLOW_REACH_MAX, GLOW_SHADOW_CURVE_MAX, GLOW_SHADOW_CURVE_MIN, GLOW_SHADOW_GAIN_MAX,
    GLOW_SHADOW_MAX, GLOW_SHADOW_NAME_MAX, GLOW_STRENGTH_MAX, MARK_THICKNESS_MAX, MAX_DRAWN_NODES,
    NODE_RADIUS_FACTOR, PLUS_SIZE_MAX, RING_INNER_MAX, RING_WIDTH_MAX,
};
use harmonigraph_core::{coords, Comma, Envelope, LatticePos, Tempered};

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

/// A block of lattice positions, as explicit inclusive bounds.
///
/// Two of these are in play and they answer different questions: the DRAWN
/// window one pane builds from its camera ([`ViewConfig::scrolled`]), and the
/// naming REACH the whole UI shares ([`ViewConfig::reach`]). A type of their
/// own is what keeps them apart — #357 passed them as two `ViewConfig`s, and
/// its own coverage test then projected through the reach while the renderer
/// drew through the derived window, where the two offsets cancelled and it
/// passed over a picture with sixty-four holes in it.
///
/// Bounds rather than `center ± extent`, because what a camera shows is not
/// centered on anything. Tilt a perspective camera and the sheet runs away
/// from the eye on one side and off the bottom of the pane on the other, so a
/// window forced symmetric about the origin draws a block that is never on
/// screen: at 40° on a 16:9 pane it asks for 25921 nodes where the camera can
/// see 9494. Orthographic and cabinet are symmetric already — their rectangles
/// are square with the view axis — so this axis of the fix is perspective's
/// alone, and it is worth about 2.7x there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawnWindow {
    /// Inclusive, in absolute lattice steps from C — not offsets from a
    /// center, so a reader needs nothing else to know what is in here.
    pub min: LatticePos,
    pub max: LatticePos,
}

/// The shape of the node glow's falloff through [`ViewConfig::glow_reach`].
/// The centre and the far edge stay fixed at 1 and 0 while one signed exponent
/// bends the curve between them without an interpolation join or an inflection.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// A curve nested in a persisted view still has its own fallback, so a
// hand-edited blob missing its shape costs that field alone.
#[serde(default)]
pub struct GlowCurve {
    /// Signed exponent, [`GLOW_CURVE_SHAPE_MIN`]..=[`GLOW_CURVE_SHAPE_MAX`].
    /// Zero is linear; positive falls early and negative falls late.
    pub shape: f32,
}

impl GlowCurve {
    /// The glow level `p` of the way from its centre to its edge.
    ///
    /// `level = expm1(k(1 - p)) / expm1(k)`. Positive `k` is a fast
    /// exponential-like decay, negative `k` holds then falls, and zero is the
    /// straight line.
    pub fn sample(self, p: f32) -> f32 {
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
        if p >= 1.0 {
            return 0.0;
        }
        exponential_level(p, self.shape())
    }

    /// The finite, bounded shape consumed by the renderer.
    pub fn shape(self) -> f32 {
        self.sanitized().shape
    }

    /// Repair the value into the finite range the editor can produce.
    pub fn sanitized(mut self) -> Self {
        let fresh = GlowCurve::default();
        self.shape =
            finite_or(self.shape, fresh.shape).clamp(GLOW_CURVE_SHAPE_MIN, GLOW_CURVE_SHAPE_MAX);
        self
    }
}

/// The normalized exponential at `p`. The series is the same one the shader
/// uses where direct subtraction of two almost-equal exponentials loses the
/// bend to float cancellation.
fn exponential_level(p: f32, shape: f32) -> f32 {
    let remaining = 1.0 - p;
    if shape.abs() < 0.05 {
        let shape2 = shape * shape;
        return remaining * (1.0 - shape * p * 0.5 + shape2 * p * (2.0 * p - 1.0) / 12.0);
    }
    (shape * remaining).exp_m1() / shape.exp_m1()
}

impl Default for GlowCurve {
    fn default() -> Self {
        // This exponent closely follows the compact accent the fresh Glow bars
        // are tuned around while leaving both slower and faster shapes in reach.
        GlowCurve { shape: 2.75 }
    }
}

impl DrawnWindow {
    /// Every position in the block, threes outer and sevens inner.
    ///
    /// The order is load-bearing: [`index_of`](Self::index_of) inverts it, so
    /// the renderer can find a neighbour by arithmetic instead of hashing.
    pub fn positions(&self) -> impl Iterator<Item = LatticePos> {
        coords::positions_within(
            self.min.threes..=self.max.threes,
            self.min.fives..=self.max.fives,
            self.min.sevens..=self.max.sevens,
        )
    }

    /// How many positions [`positions`](Self::positions) yields, so a
    /// per-frame buffer can preallocate instead of growing through
    /// reallocations.
    pub fn count(&self) -> usize {
        self.span(|p| p.threes) * self.span(|p| p.fives) * self.span(|p| p.sevens)
    }

    /// Where `pos` lands in [`positions`](Self::positions), or `None` when it
    /// is outside the block.
    ///
    /// Each axis is bounds-checked separately, which is the whole of what
    /// keeps an out-of-range step on one axis from aliasing onto a different
    /// node's slot.
    pub fn index_of(&self, pos: LatticePos) -> Option<usize> {
        let axis = |v: i32, lo: i32, hi: i32| (v >= lo && v <= hi).then(|| (v - lo) as usize);
        let t = axis(pos.threes, self.min.threes, self.max.threes)?;
        let f = axis(pos.fives, self.min.fives, self.max.fives)?;
        let s = axis(pos.sevens, self.min.sevens, self.max.sevens)?;
        Some((t * self.span(|p| p.fives) + f) * self.span(|p| p.sevens) + s)
    }

    pub fn contains(&self, pos: LatticePos) -> bool {
        self.index_of(pos).is_some()
    }

    /// One axis's inclusive span, floored at zero so a reversed pair counts as
    /// empty rather than wrapping a `usize`.
    fn span(&self, axis: fn(LatticePos) -> i32) -> usize {
        (axis(self.max) - axis(self.min) + 1).max(0) as usize
    }

    /// Clip the block into [`MAX_DRAWN_NODES`], taking what it loses off the
    /// end of each axis FARTHER from `center` — the horizon — and leaving the
    /// end nearer the center where it is.
    ///
    /// Which end is which is the whole of it, because the block is lopsided
    /// wherever the camera's view of the sheet is. A tilted perspective window
    /// runs a step or two below the eye and hundreds of steps out to the far
    /// field, so a trim that scaled both ends by one factor took the same
    /// FRACTION off each — and a fraction of a near edge already beside the
    /// center is the whole of it. What that drew is a bald wedge across the
    /// lower half of the pane with the lattice running on above it:
    /// `the_budget_trims_the_horizon_not_the_foreground` holds it.
    ///
    /// Toward the CENTER rather than toward the block's own middle.
    /// `follow_camera` keeps the camera's target within one cell of the
    /// center, so the center is what the eye is on; under the tilted camera
    /// that reaches this cap at all, the block's own middle is way out in the
    /// sub-pixel far field.
    ///
    /// Only [`ViewConfig::scrolled`] can reach a block this large, and only
    /// from a camera the picture is already degenerate under — see
    /// [`MAX_DRAWN_NODES`] for what the cap is protecting.
    fn fit_to_node_budget(&mut self, center: LatticePos) {
        let count = self.count();
        if count <= MAX_DRAWN_NODES {
            return;
        }
        // Both sheet axes at once, by the square root of how far over budget
        // the count is — the count goes as their product, so that is the
        // factor that lands near the cap in one step whatever shape the block
        // is. The sevens axis is left alone: it is a setting rather than a
        // consequence of the camera, and it is at most nine sheets.
        let shrink = (MAX_DRAWN_NODES as f32 / count as f32).sqrt();
        // Spent as a RADIUS about the center that the block is clipped into,
        // rather than as a scale on each bound: an end already inside the
        // radius is left alone, so a lopsided block loses its far side and
        // keeps its near one. On the symmetric block cabinet and orthographic
        // build, where both ends are the same distance out, the two are the
        // same arithmetic — which is why their figures did not move.
        //
        // It is also the closed form of the loop below: take a step off
        // whichever end is farther from the center, over and over, and what
        // you are left with is the block inside a radius.
        let clip = |lo: &mut i32, hi: &mut i32, c: i32| {
            let far = (*hi - c).max(c - *lo).max(0);
            let radius = (far as f32 * shrink) as i32;
            // Neither end is ever clipped PAST the other. A camera tilted to
            // the limit puts the whole visible block to one side of the center
            // (`threes 19..31` at the pitch limit), and clipping its near end
            // across its far one would leave a window with nothing in it.
            let near = (*lo).max(c - radius).min(*hi);
            *hi = (*hi).min(c + radius).max(near);
            *lo = near;
        };
        clip(&mut self.min.threes, &mut self.max.threes, center.threes);
        clip(&mut self.min.fives, &mut self.max.fives, center.fives);
        // The step above lands near the cap rather than exactly on it (each
        // axis carries a `+1` that does not scale, and a lopsided axis keeps
        // its near side whole), so the cap is finished by hand — a step at a
        // time off the longer axis, from whichever of its ends is farther from
        // the center, which is the end with less on it worth seeing.
        while self.count() > MAX_DRAWN_NODES {
            let (lo, hi, c) = if self.span(|p| p.fives) >= self.span(|p| p.threes) {
                (&mut self.min.fives, &mut self.max.fives, center.fives)
            } else {
                (&mut self.min.threes, &mut self.max.threes, center.threes)
            };
            if lo == hi {
                break;
            }
            if c - *lo >= *hi - c {
                *lo += 1;
            } else {
                *hi -= 1;
            }
        }
    }
}

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
// Every field falls back to `impl Default` below, so a blob missing one costs
// that key alone rather than failing to parse and taking the whole persist —
// the camera, the dock and every other setting — down with it.
#[serde(default)]
pub struct ViewConfig {
    /// World-space distance between adjacent nodes.
    pub spacing: f32,
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
    /// How many sheets either side of the home one the lattice draws. Unlike
    /// the two above this IS the drawn window, and it keeps its bar: how deep
    /// the lattice runs is a question about the music, where how wide it runs
    /// is a question about the pane, and only the pane can be read off the
    /// screen.
    pub extent_sevens: i32,
    /// Center of the window, in lattice steps from C (v1's Grid X/Y/Z). The
    /// center node renders at the world origin, so panning the window doesn't
    /// walk the content away from the camera.
    ///
    /// The fifths and thirds centers are driven by the camera rather than by a
    /// bar ([`follow_camera`](Self::follow_camera)): they are where the reach
    /// above is centered, and it has to stay under what is on screen. The
    /// sevens center is the home sheet, which is a choice, and keeps its bar.
    pub center_threes: i32,
    pub center_fives: i32,
    pub center_sevens: i32,
    // ---- The sevens layer ------------------------------------------------
    // How the sheets other than the home one draw. Both settings go inert
    // while `extent_sevens` is 0, which is where a fresh view starts. What
    // makes a small node legible over a large one is the Glow section's Shadow
    // ([`glow_shadow`](Self::glow_shadow)) — each item multiplying the frame
    // under it by its own blurred ink, at any extent and on every sheet.
    //
    // The problem all three settings answer: the 5-limit sheet wants its
    // pitch classes as large as they will go, and at the default spacing a
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
    /// Only meaningful while `show_labels` is on.
    pub sevens_label: SevensLabel,
    /// Draw note-name labels on hovered and sounding nodes.
    pub show_labels: bool,
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
    /// cents. Only meaningful while `show_labels` is on.
    pub show_cents: bool,
    /// WHICH nodes carry a label: every one, every one the music has
    /// visited, or only what is sounding (see [`NoteNames`]). Only
    /// meaningful while `show_labels` is on, which is the on/off for text on
    /// the lattice as a whole.
    ///
    /// The one setting here that reaches into the PAST, and what it reaches
    /// with is drawn in TYPE alone -- see the [`trail`](crate::trail) module
    /// for why that is the whole design and not an implementation detail.
    /// [`NoteNames::Past`] is the only mode that fills
    /// [`NodeInstance::trail`](crate::NodeInstance::trail); a name under
    /// [`NoteNames::All`] is the label layer's own answer and carries no
    /// memory at all.
    pub note_names: NoteNames,
    /// How bright the text on a SOUNDING node is — its name, the marks beside
    /// it and its cents line alike — as an `L*` 0..100 on the same axis as
    /// [`marker_ink`](Self::marker_ink), which is what that same text draws in
    /// once nothing is sounding under it.
    ///
    /// One END of a pair rather than a brightness on its own, and the pair is
    /// what the setting is: a label crosses between the two on its node's own
    /// [`activation`](crate::NodeInstance::activation), so the note Fade times
    /// the crossing and there is no second clock to set. Equal numbers put
    /// every label in the resting field's grey and the type answers to the
    /// music by nothing; every other pairing is reachable from there, a
    /// sounding name held BELOW a kept one included.
    ///
    /// A brightness and not an opacity, which is the whole of why this is a
    /// bar rather than a factor on a label's strength. Alpha over the
    /// lattice's dark ground is grey rather than a fainter white, so a rank
    /// spent there costs the quieter end its legibility as well as its rank;
    /// an `L*` names the grey each end lands on outright, and neither end is a
    /// fraction of the other.
    ///
    /// **Neutral**, for [`lattice_ground`](Self::lattice_ground)'s reason: hue
    /// in this picture is the music's, and a name is not where a note's colour
    /// lives. Only meaningful while `show_labels` is on.
    pub sounding_ink: f32,
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
    /// The node's RADIAL padding, in quad UV units: the gap between one ring of
    /// the stack and the next (see [`rings`](Self::rings)), which is also what
    /// stands a melody/bass mark off the band it continues.
    ///
    /// The other axis is [`octave_gap`](Self::octave_gap), and the two are
    /// separate because they are answers to different questions. This one is
    /// about the STACK: how far apart the annuli read, which is the same
    /// question the three widths above are asked and is settled against them —
    /// every unit spent here is a unit of quad the layers do not get, so the
    /// gap and the sizes are dialled together on one bar's worth of room. The
    /// angular gap spends nothing: it cuts the slices out of a ring already
    /// placed.
    ///
    /// 0 closes the stack up: every ring meets the one inside it and a mark
    /// seats against the band. A gap is only ever spent between two DRAWN
    /// layers, so a ring dialled to 0 costs its own slot and the gap that would
    /// have stood it off together.
    pub ring_gap: f32,
    /// The node's ANGULAR padding, in quad UV units: the gap between one octave
    /// sector and the next, cut as a constant-thickness band at every radius so
    /// it is the same width where a slice starts and where it ends.
    ///
    /// One number over every angular slice on the node, and that IS the whole
    /// of the layer-crossing claim here: the octave band's sectors, the audio
    /// ring's wedges and a melody/bass mark's own edges all run on it, so one
    /// rhythm of interruptions runs radially through the node and the rings
    /// read as one picture rather than three that happen to be concentric. What
    /// it does NOT set is how far apart those rings sit — see
    /// [`ring_gap`](Self::ring_gap).
    ///
    /// 0 closes the ring round: the sectors become a solid annulus, and a
    /// backdrop is what still says an octave is silent.
    pub octave_gap: f32,
    /// A node's RINGS where nothing is sounding, as an `L*` 0..100 — one
    /// neutral grey under both of the surfaces the node itself draws empty:
    ///
    /// - the **audio ring** wherever it reads silence, which its ramp is
    ///   re-anchored to open on ([`ring_gradient`](crate::ring_gradient));
    /// - the **MIDI ring**'s octave slices that are not sounding, which ARE
    ///   this colour, with a sounding octave's pitch painted over them.
    ///
    /// One number under both, like [`octave_gap`](Self::octave_gap) above it,
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
    /// with it, which is the resting end of the label pair — see
    /// [`sounding_ink`](Self::sounding_ink) for why the two share one number.
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
    // Which shimmer sweeps the octave glyphs has no field here: one pattern
    // (`pulse_marks`) sizes the sheet for every layer it reaches — the
    // glyphs of a currently sounding octave and a melody or bass mark's own
    // strip alike. Saved blobs still carry the `pulse_octaves` key, naming a
    // pattern nothing reads any more; serde ignores unknown keys, so such a
    // blob loads intact and drops the key on the next save.
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
    /// Its own times rather than the note Fade, too, for a reason the Fade's own
    /// default states: 0.15 s is where a note's arrival and its release agree,
    /// and that is a judgement about MIDI transients. Riding the ring's
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
    /// 0.35 fresh, which is a gentle power rather than the straight line 0
    /// draws — a note that leaves quickly at first and then lingers reads as
    /// decaying rather than as being wound down. A blob with no `fade_shape`
    /// key gets that, like every other missing key: the container-level
    /// `#[serde(default)]` makes `impl Default` the one fallback, and
    /// `a_view_missing_any_one_key_reloads_at_the_fresh_value` holds it.
    pub fade_shape: f32,
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
    /// Mark the highest held note.
    ///
    /// Independent of [`mark_bass`](Self::mark_bass), and they share one
    /// strip: a mark is its own octave's slice continued outward, so what
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
    pub mark_melody: bool,
    /// Mark the lowest held note. See [`mark_melody`](Self::mark_melody).
    pub mark_bass: bool,
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
    /// [`octave_gap`](Self::octave_gap), the same padding that separates one
    /// indicator from the next, so the mark reads as that indicator continued
    /// however far out the stack stands it; a `ring_gap` of 0 closes the
    /// stand-off, and the mark meets its slice.
    ///
    /// 0 turns the marks off, as a width of 0 turns any layer off. Absolute
    /// rather than a fraction of the band's width, which would move the marks
    /// every time the band is resized.
    pub mark_thickness: f32,
    /// How long a note must HOLD an end before its mark begins to ease in,
    /// in seconds. The wait sits in front of the ease rather than stretching
    /// it: the mark is at 0 for this long and then arrives on the same Fade
    /// ramp ([`envelope`](Self::envelope)) every other layer arrives on.
    ///
    /// The wait is also a THRESHOLD: an end that changes hands again before
    /// the delay is up never draws a mark at all. That is what the setting is
    /// for. Playing fast, the top and bottom of what is down change every few
    /// notes, and a mark easing in on each of them reads as flicker around the
    /// octave band rather than as the line it is tracing — so the delay is
    /// how long a note has to be the melody before it counts as the melody.
    ///
    /// A mark outlives its key (it fades out on the note's release), so the
    /// threshold is answered AT the key-up — `derive_scene`'s `ease` — and
    /// only the ramp runs on from there. Left to the ramp alone, an end
    /// dropped mid-delay would climb past the threshold while the note was
    /// already fading and mark a note that never was the melody, which is the
    /// very flicker this setting buys off.
    ///
    /// Not derived from the note Fade, which is the other end of the same
    /// note and reads as the natural pair: a fade is how long a note takes to
    /// LEAVE, and tying the two would mean a long release could not be paired
    /// with a mark that answers immediately. The delay is measured from the
    /// handoff the tracker stamps ([`HeldEnd`](harmonigraph_core::HeldEnd)),
    /// which is exactly why that stamp cannot come off the released voice:
    /// any delay past the Fade would outlive the note that handed the end
    /// over.
    ///
    /// 0 is the mark arriving with its note, and is deliberately not what a
    /// fresh view opens on — see `impl Default`, where the wait that stops
    /// the chord-release smear is written out.
    pub mark_delay: f32,
    /// Which shimmer sweeps the lattice (see [`Pulse`]): the sheet takes
    /// every octave slice a note currently lights, and a melody or bass
    /// mark's own strip past the band as well — a mark being one slice in
    /// two pieces, so light crossing one crosses the other. [`Pulse::Bands`]
    /// fresh, so the sweep is on out of the box; a blob with no `pulse_marks`
    /// key gets that same value, the container-level `#[serde(default)]`
    /// making `impl Default` the one fallback for this field as for every
    /// other.
    pub pulse_marks: Pulse,

    // ---- Shimmer ---------------------------------------------------------
    // The sweep's knobs: what the pattern above is sized and paced by, one
    // sheet of light crossing the whole lattice.
    //
    // All four are inert while the pattern is Off.
    /// How fast the shimmer travels, in world units per second — the
    /// lattice's own units, so the DAW window and an exported video sweep at
    /// the same rate across the same nodes, where a rate in screen pixels
    /// would not. It travels along the bands' own normal, every pattern here
    /// being gratings. 0 freezes the sheet where it stands, which is a look
    /// rather than an off switch (the mode is the switch).
    pub shimmer_speed: f32,
    /// How wide the pattern is, in the same world units: the distance from one
    /// bright peak to the next, which sizes the lit part and the dark
    /// between it and its neighbour together — the shimmer is one shape,
    /// scaled, rather than a width and a spacing that could disagree. Every
    /// pattern is built out of gratings of exactly this period, so the bar
    /// means the same thing in all of them (a hex cell comes out about 15%
    /// wider than this, three gratings at sixty degrees being what makes it).
    ///
    /// The range spans three ORDERS of it, and the two ends are different
    /// pictures rather than more and less of one:
    ///
    /// - Wide (around the default, several nodes to a band) is a sheet
    ///   crossing the lattice, each node lighting as it passes.
    /// - Around one node to a band the two read against each other worst:
    ///   neighbours land most of a cycle apart and the picture is alternating
    ///   NODES rather than a band passing over them, the lattice's own
    ///   spacing being irregular (the thirds and fifths axes both project
    ///   onto the screen's x).
    /// - Below that, several bands cross a single node at once and it is a
    ///   texture on the nodes rather than a sweep between them — which is a
    ///   look worth reaching, and why the floor is a small fraction of a node
    ///   rather than a stop above the awkward middle.
    ///
    /// A node is [`spacing`](Self::spacing) × 0.25 in world radius, so the
    /// count of bands across one is roughly its diameter over this.
    ///
    /// The tight end is a resolution trade as well as a look, and the shader
    /// spends it deliberately. A pattern is sines of a world coordinate
    /// sampled once per fragment, so a period approaching a pixel — a tight
    /// setting seen from far enough out — has no samples left to carry it and
    /// would alias into moire that crawls as the camera moves. Rather than
    /// draw that, `shimmer_terms` fades the sheet's amplitude out as its
    /// period closes on the pixel footprint, so the layer settles to its
    /// unshimmered self instead of to a shifting texture. The setting is
    /// still the size it says it is on the lattice; what runs out is the
    /// SAMPLING, and the fade is what makes running out look like an ending
    /// rather than a fault. Frame the shot at the zoom the tight end is
    /// chosen for.
    pub shimmer_width: f32,
    /// How strong the sweep is where it passes, 0..1 being none to the full
    /// tuned depth: the ratio of light between a band's crest and the trough
    /// beside it, which is ONE number for the whole of what a band does.
    ///
    /// The light is a MULTIPLY — an exposure — rather than an amount added or a
    /// mix toward white (`SHIMMER_EXPOSURE` in `lattice.wgsl`). That is what
    /// makes one setting mean one thing across the pitch ramp, and it means it
    /// in the currency the eye reads a moving texture in: the crest-to-trough
    /// ratio a setting is worth varies 3% from the ramp's dark end to its bright
    /// one, where an added light varies 28% and a mix toward white more still.
    /// A sheet that was uniform in the LIGHT it added — which the addition very
    /// nearly was — still read weaker on the ramp's bright half, because equal
    /// added light is not equal contrast up there.
    ///
    /// What it costs is CHROMA at a crest, and what it holds is hue. Scaling all
    /// three channels by one gain slides a color along its own chromaticity, and
    /// where the crest runs out of room it pales toward white rather than
    /// clipping — all three channels moving together, so the color keeps its
    /// hue while it loses some of its colorfulness. Across the ramp's two ends
    /// at 1 that is 0.7 and 5.0 degrees of hue, against 88% and 57% of the
    /// chroma. An addition clips in whichever channel is already highest, which
    /// holds more chroma (99.6% and 73%) and swings the hue three times as far
    /// (15.3 degrees); a mix toward white leaves 15% of the chroma everywhere,
    /// at a trough as much as at a crest.
    ///
    /// 0 is the layer drawing exactly as it does unshimmered, from a bar rather
    /// than from the mode. Where the display leaves the swing room, the whole
    /// of it goes upward: the troughs sit at the layer's own color and stay
    /// there. Where a color is too bright for that, the swing slides down to
    /// keep its crest a color rather than a white flash, and the troughs pay
    /// for the slide — nothing below the middle of the default ramp, about 15
    /// `L*` of standing shade at its bright end at 1.
    ///
    /// What the light costs is real at any setting, and it is the point of
    /// the bar: under a strong band an indicator says "an octave sounds here"
    /// without saying which.
    pub shimmer_intensity: f32,
    /// How the light is shared out ACROSS one period, 0..1 — where
    /// [`shimmer_intensity`](Self::shimmer_intensity) says how much light
    /// there is, this says how gradually it arrives.
    ///
    /// The pattern is a raised cosine raised to a power, and this is the
    /// power, log-spaced from 8 at 0 to 1 at 1:
    ///
    /// - Toward 0 the peak is a narrow crest on a layer that is otherwise at
    ///   rest — a hard white band with a dark field around it, which at a
    ///   tight width is a stripe pattern more than a sweep.
    /// - Toward 1 the exponent reaches 1 and the pattern IS the cosine: every
    ///   point of the period is on its way somewhere, so the brightest part
    ///   fades into the clearest across the whole of the gap rather than at
    ///   an edge. Nothing is at rest, which is the cost — the layer is lit
    ///   somewhere at every instant.
    ///
    /// One number for both halves of the shape, like Intensity: the bright
    /// part narrows exactly as the dark part widens, so a period always adds
    /// up to itself and no setting can leave the sheet mostly-lit and
    /// mostly-dark at once.
    pub shimmer_softness: f32,

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
    /// same contour the flood uses for every other caster.
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
    /// Bloom post-process: how much blurred brightness gets added back
    /// as a halo around bright notes. 0 disables the chain entirely — the
    /// composite is then exactly the plain scene, so there is deliberately
    /// no separate on/off toggle.
    pub bloom_strength: f32,
    /// The node glow: how far past a node's outermost drawn edge its own light
    /// spreads, in the quad UV units the layer sizes are in. 0 turns it off —
    /// nothing is drawn at all — so the glow's other fields need no toggle of
    /// their own, and the Glow section greys them under it.
    ///
    /// What it draws is every sounding octave's hue laid round the node by
    /// angle, over a falloff sized to the whole node: the node's outermost
    /// drawn edge plus this reach is both the falloff's domain and where its
    /// window shuts, so this bar is exactly where the light stops.
    ///
    /// [`glow_curve`](Self::glow_curve) says how much light is left at each
    /// distance inside that span. Keeping the two separate makes a wide Reach
    /// useful both as a larger accent and as a faint field carried across the
    /// gaps between nodes. The ceiling ([`GLOW_REACH_MAX`]) is sized for the
    /// second picture — several lattice steps, where every node's light
    /// overlaps its neighbourhood's.
    ///
    /// It is the ONLY light a node has: a view with this at 0 draws exactly the
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
    /// How much light the node glow lays down. Inert while
    /// [`glow_reach`](Self::glow_reach) is 0.
    pub glow_strength: f32,
    /// How the light's brightness falls inside [`glow_reach`](Self::glow_reach).
    /// The endpoints stay fixed: full at the node's centre and zero where the
    /// reach ends.
    pub glow_curve: GlowCurve,
    /// The Shadow: how wide every item's shadow is, in the same quad UV units
    /// [`ring_gap`](Self::ring_gap) reads in. It is HALF this in σ
    /// (`shadow::sigma_px` in harmonigraph-render), which puts a wide caster's
    /// shadow all but out at one bar width.
    ///
    /// ONE length for the whole picture, and that is what makes it one bar. A
    /// blur row convolves each caster at this σ; a distance row spends the same
    /// width through its profile. Each item multiplies whatever is already in
    /// the frame under it in the painter's order the pass already walks — so
    /// what it casts is read off its own ink rather than off which draw it
    /// belongs to, and a nearer item darkens a farther one wherever they overlap.
    ///
    /// A blur of the INK and not of a circle around it: a node reaching a
    /// melody mark on one octave casts from that wedge and hugs its rings
    /// everywhere else, the empty middle its rings stand around casts nothing,
    /// and a hairline ring casts a fainter shadow than a wide band does. Each
    /// layer's own envelope is already in the coverage that is blurred, so a
    /// releasing layer's shadow fades with its ink.
    ///
    /// It is spent on the FRAME rather than on the light alone, and the light
    /// takes it by being under everything: the halos are composited at the
    /// bottom of the scene pass, so a shadow lands on a neighbour's halo, on a
    /// ring behind, and on a name, all at the depth
    /// [`glow_shadow_depth`](Self::glow_shadow_depth) says.
    ///
    /// Without it the light is at its brightest exactly where the rings are —
    /// the falloff is measured from the node's centre, so both sides of a ring
    /// sit near the peak — and a ring drawn over that is a flat grey silhouette
    /// on a bright field. A ring standing in a pool that brightens outward is
    /// what the eye reads as the ring being the source of the light.
    ///
    /// The ceiling is [`GLOW_SHADOW_MAX`], a whole radius where the two
    /// paddings stop at [`GAP_MAX`]: what stops a shadow reading as a black RIM
    /// rather than as a lack of light is a blur broad enough to come off at the
    /// rate the skirt does, and that is a width a good deal past any padding's.
    /// Nothing in the picture bounds it short of that — every caster's own quad
    /// is grown by the blur's reach so that it holds the answer.
    ///
    /// Independent of [`glow_reach`](Self::glow_reach): an item casts with no
    /// light in the picture at all, onto the ground and onto whatever ink
    /// stands behind it.
    pub glow_shadow: f32,
    /// How dark a shadow lands where it is whole, 0..=1 — the factor the frame
    /// is left with under a caster's solid middle, spent in STOPS across that
    /// caster's own blur.
    ///
    /// In stops and not in proportion, because sight answers ratios: a factor
    /// walked evenly in VALUE spends most of what can be SEEN of it in the
    /// first fraction of the blur's width — at 0.85, half the visible swing is
    /// gone by a fifth of the way out — and the shadow then reads as a dark rim
    /// hugging the ink with an edge on it however wide the Shadow is dialled.
    /// See `shadow_through` in lattice.wgsl.
    ///
    /// A FLOOR rather than a scale: the exponent is capped at 1, so a caster
    /// wide against σ lands exactly here and a thin one lands short of it. 1
    /// takes the frame under a wide caster to black; 0 is the picture with no
    /// shadow in it at all, pixel for pixel, which is what makes this bar the
    /// A/B on the whole feature.
    ///
    /// It is spent on the frame and never on the caster's own ink, which the
    /// draw leaves unmultiplied — so an item is the one thing its own shadow
    /// never darkens, and a ring standing in a pool cleared to the bare ground
    /// still carries the colour of the halo around it.
    ///
    /// It is the atlas's second off switch: at 0 no cell is packed and every
    /// draw multiplies by 1. Independent of
    /// [`glow_reach`](Self::glow_reach).
    pub glow_shadow_depth: f32,
    /// What a caster's blurred ink is multiplied up by before it is spent as a
    /// shadow, 0..=[`GLOW_SHADOW_GAIN_MAX`].
    ///
    /// The blur of a caster's coverage is at most 1, and only deep inside a
    /// caster far wider than σ; a hairline ring or a stroke of type is a few
    /// pixels against a σ of several, so its blur peaks at a fraction of that
    /// and its shadow, spent as an exponent on the depth, would land at a
    /// fraction of the depth the bar names. This is the factor that fraction is
    /// multiplied up by, and the `min(…, 1)` under it keeps
    /// [`glow_shadow_depth`](Self::glow_shadow_depth) a true FLOOR: a caster
    /// wide against σ saturates there rather than overshooting, and the gain
    /// only deepens the thin ones.
    ///
    /// So it is the bar that says what the picture's THINNEST ink is worth
    /// against its widest, which is not a thing either width or depth can say
    /// on its own. At 0 nothing casts at all. At the top a hairline is as dark
    /// as a solid band, which is the whole lattice reading as one silhouette.
    pub glow_shadow_gain: f32,
    /// The exponent the gained blur is bent by on its way to the depth,
    /// [`GLOW_SHADOW_CURVE_MIN`]..=[`GLOW_SHADOW_CURVE_MAX`]. 1 is the straight
    /// line and is what a fresh view opens on.
    ///
    /// The gained blur is a number in 0..=1 and the depth is spent as its
    /// exponent, so bending it here bends where along the shadow's WIDTH the
    /// darkness sits without moving either end: the saturated middle stays at
    /// the depth and the far edge stays at nothing.
    ///
    /// Under 1 lifts the tail — the shadow reads as a broad even pool that
    /// stops abruptly, the sky-like look the blur family is otherwise short of.
    /// Over 1 presses the tail down and pulls the shadow in against the ink,
    /// which is a Gaussian read as a rim. That the two halves have different
    /// room to move is why the range is not symmetric about 1 (see
    /// [`GLOW_SHADOW_CURVE_MAX`]).
    ///
    /// It is free: one `pow` in the same function that already spends the gain
    /// (`shadow_transmittance`, common.wgsl), and no atlas, cell or quad moves
    /// with it.
    pub glow_shadow_curve: f32,
    /// What a note NAME's σ takes against the rest of the picture's,
    /// 0..=[`GLOW_SHADOW_NAME_MAX`]. 1 is one width across the whole lattice
    /// and is what a fresh view opens on.
    ///
    /// The one place the lattice's "one bar, one reach" is asked to bend, and
    /// it is asked because a name is the only ink in the picture whose SHAPE is
    /// meant to be read. A ring and a cross want a shadow that says how thick
    /// they are; a letterform wants one that does not fill its counters, and
    /// the width that does the first may not be the width that does the second.
    ///
    /// 0 is a name's ink cast with no blur at all — a hard-edged drop of the
    /// letterforms themselves, the cell being packed with no padding and the
    /// kernel collapsing to its centre tap.
    ///
    /// Nearly free of the atlas's cost model: σ is per CASTER where the cell is
    /// packed (`pack` in `harmonigraph-render`'s `shadow.rs`), and each cell is
    /// drawn at `min(1, SIGMA_CELL_MAX / σ)` of the target's pixels, so a name
    /// at three times the width is a cell a third the size rather than a kernel
    /// three times as wide. What it does cost is the name's own quad, which
    /// grows with its reach like every other caster's.
    pub glow_shadow_name: f32,
    /// Which mixture of Gaussians every caster's ink is blurred with — the
    /// SHAPE of a shadow's falloff, where [`glow_shadow`](Self::glow_shadow) is
    /// how far it reaches and [`glow_shadow_depth`](Self::glow_shadow_depth)
    /// how dark it lands.
    ///
    /// Every row is scaled to the same reach, so switching one does not move
    /// the Shadow bar under it: what changes is where the darkness sits between
    /// the ink and the edge. See [`ShadowKernel`] for why the shapes worth
    /// comparing arrive as mixtures rather than as kernels of their own.
    ///
    /// It is the one setting in the Shadow section that costs ATLAS: a row of
    /// N terms packs N cells for every caster, each at the resolution its own σ
    /// asks for. The blur chain over them is unchanged — every cell's σ is
    /// still capped at `SIGMA_CELL_MAX` texels by construction — so what grows
    /// is the area packed and the taps a caster's draw takes, not the kernel
    /// any one cell is blurred by.
    pub glow_shadow_kernel: ShadowKernel,
    /// The exponent a DISTANCE row's decay is taken over,
    /// [`SHADOW_SHAPE_MIN`](crate::SHADOW_SHAPE_MIN)..=[`SHADOW_SHAPE_MAX`](crate::SHADOW_SHAPE_MAX).
    /// 1 is the plain exponential and is what a fresh view opens on.
    ///
    /// The distance family's own profile bar, and it is NOT
    /// [`glow_shadow_curve`](Self::glow_shadow_curve) under another name — the
    /// two bend opposite things and would mean opposite things across the
    /// picker. γ below 1 lifts a blur's tail and fills its middle into a pool;
    /// `shape` below 1 steepens the decay where it leaves the ink and leaves a
    /// haze over the rest of the width, which is a skin rather than a pool. One
    /// bar meaning two things across a toggle is the kind of dial #520 deletes.
    ///
    /// The range runs entirely UNDER the exponential because that is where the
    /// family has no knee: see [`SHADOW_SHAPE_MAX`](crate::SHADOW_SHAPE_MAX).
    /// Inert on a blur row, and the bar is hidden there.
    pub glow_shadow_shape: f32,
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

/// Where each layer of a node lands, in quad UV units, read outward from its
/// center — what [`ViewConfig::rings`] turns the four size bars into.
///
/// The bars are WIDTHS, and a ring's inner edge is wherever the last drawn
/// layer ended plus one [`gap`](Self::gap) — or [`inner`](Self::inner), for the
/// innermost layer left on. That is the whole of what stacking
/// buys: widening one layer slides everything outside it out as far as the quad
/// edge, no bar can be dragged behind its neighbour, and a layer dialled to 0
/// hands its slot AND its gap back to the ones around it instead of leaving a
/// hole.
///
/// The quad edge is where sliding stops and DROPPING starts: a ring is the
/// width its bar reads or it is not drawn, so the first layer the stack can no
/// longer fit whole comes out as an empty pair. Widen the audio ring far enough
/// and the node loses the band, ending as that ring alone — rather than wearing
/// a hairline whose bar reads out a size nothing on screen matches (`stacked`).
///
/// An empty pair is also what makes `outer > inner` the one test for "this ring
/// draws": there is nothing else to ask, and no second flag that could
/// disagree with the geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingStack {
    /// Where the innermost layer left on begins, and so the empty middle a node
    /// carries (see [`ViewConfig::ring_inner`]). 0 seats the stack on the
    /// node's own center.
    ///
    /// A radius the stack is READ from rather than one of its layers: nothing
    /// is drawn inside it, so no pair of radii describes it and no `outer >
    /// inner` test asks whether it is there.
    pub inner: f32,
    /// The audio ring's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::spectral_ring_width`]).
    pub audio: (f32, f32),
    /// The octave band's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::band_width`]).
    pub band: (f32, f32),
    /// The outer edge of the outermost ring DRAWN, and 0 on a node with no
    /// ring at all.
    ///
    /// What the melody/bass marks stand off, and what a node's billboard is
    /// sized on, so neither has to know which of the rings inside it
    /// happened to be the last one on.
    pub outer: f32,
    /// Where the melody/bass mark strip STARTS: a gap out from
    /// [`outer`](Self::outer), or [`inner`](Self::inner) when the stack is
    /// empty and there is nothing to stand off.
    ///
    /// Settled here rather than left to the renderer because that second
    /// clause is [`Stack::take`]'s rule, and a layer deriving its own inner edge
    /// downstream is a layer that does not get it: the marks are the one slot
    /// `stacked` cannot hand out, since they alone may run past the quad edge
    /// into the billboard's margin instead of being refused there.
    pub mark_inner: f32,
    /// How deep the mark strip is from [`mark_inner`](Self::mark_inner) (see
    /// [`ViewConfig::mark_thickness`]); 0 = off. The OUTER edge is the one
    /// thing about the marks still left to the renderer, which eases the strip
    /// off its own billboard edge.
    pub mark_thickness: f32,
    /// The padding between two drawn layers (see [`ViewConfig::ring_gap`]) —
    /// the RADIAL one alone, this being the stack. What separates one octave
    /// sector from the next is [`ViewConfig::octave_gap`], which no radius on
    /// this struct depends on.
    pub gap: f32,
}

impl RingStack {
    /// The four boundaries the stack is laid out on, read outward: where the
    /// audio ring's slot begins, where the octave band's begins, where the
    /// melody/bass strip's begins, and where that strip ENDS.
    ///
    /// Each is the outer limit of what is INSIDE it — the middle two a
    /// [`gap`](Self::gap) past where a layer stopped, the last one flush, there
    /// being no layer after the marks to stand off, and the first one flush
    /// too, the empty middle being no layer to stand off either. So the four
    /// run middle, audio ring, band, marks: one boundary per handle, and moving
    /// one is that handle's own number changing.
    ///
    /// **A slot a layer WOULD take, where the layer is not drawn**, which is
    /// what makes this different from reading the radii above and the reason
    /// it exists: an off layer has no inner edge of its own, so a control that
    /// sized it by its radii would lose the handle the moment it was switched
    /// off and could never switch it back on. Two boundaries landing on one
    /// point is exactly what an off layer looks like here, and the Layers bar
    /// draws them piled up.
    ///
    /// A REFUSED layer comes out the same way, since the cursor did not move
    /// for it either — see [`Stack::take`]. The picture and the bar then agree:
    /// the layer is not on the node and its handle is not out on the axis.
    pub fn edges(&self) -> [f32; 4] {
        let after_audio = if self.audio.1 > self.audio.0 { self.audio.1 } else { 0.0 };
        [
            self.inner,
            slot_start(after_audio, self.inner, self.gap),
            self.mark_inner,
            self.mark_inner + self.mark_thickness,
        ]
    }
}

/// The stack a node's rings are handed out of, innermost first: a start radius
/// and a cursor at the outer edge of the last layer DRAWN.
///
/// A ring draws at exactly the width its bar reads or it is not drawn at all,
/// which is why a slot that does not fit is REFUSED rather than clipped to the
/// room left. Clipping keeps the layer alive at whatever width the stack
/// happened to leave it — a bar reading 0.19 drawing 0.0008, a hairline at the
/// node's rim that no setting asked for and nothing on screen explains.
///
/// The two ways a layer comes back empty are different questions, and only one
/// of them is about the room: a layer at width 0 is switched off by its own
/// BAR and gives its slot up, where a layer REFUSED is holding a width the
/// room cannot seat. What tells them apart is the width the layer still reads,
/// not the empty pair they share — see `resized` in the Layers bar, which is
/// the one caller that has to.
struct Stack {
    /// Where the innermost layer DRAWN begins, whichever layer that turns out
    /// to be (see [`ViewConfig::ring_inner`]).
    inner: f32,
    cursor: f32,
    /// How far out the stack REACHED, a refusal counting as far as the layer
    /// it could not seat would have gone.
    ///
    /// The cursor answers "what does the next layer stand off?", and stops at
    /// the last layer DRAWN so that a layer switched off gives its slot back.
    /// This answers "how far out is the stack spoken for?", which is a
    /// different question the moment a refusal is in play: the room ran out,
    /// nothing outside can be seated, and the slot is spent rather than free.
    /// The mark strip is the one layer that reads this instead of the cursor —
    /// see [`ViewConfig::rings`].
    reach: f32,
    /// Set by a refusal, and the reason it is not just a cursor: the two ways
    /// a layer comes back empty are different questions, and only one of them
    /// is about the room.
    ///
    /// A layer at width 0 is switched off by its own BAR and gives its slot
    /// up; the layer outside it closes over the space, which is what the bar's
    /// hover promises. A layer REFUSED is the room itself running out, and
    /// nothing outside it can fit either — so the stack drops from the outside
    /// in, one layer at a time, and a layer that has gone stays gone while the
    /// room keeps shrinking.
    ///
    /// Letting a refusal leave the cursor where it was makes the refused
    /// layer's slot a gift to the one outside it, which reads on screen as the
    /// stack coming apart in no order at all: push the stack's start past the
    /// audio ring's slot and the BAND takes it, so a band that had been gone
    /// for a quarter of the bar's travel reappears, and the ring's own width
    /// bar picks up a second off position at the TOP of its travel, where
    /// dragging it wider is what removes it.
    full: bool,
}

impl Stack {
    /// The next layer's slot: `width` thick, a `gap` out from the cursor,
    /// which it then advances to its own outer edge.
    ///
    /// The gap is skipped while nothing has been drawn, where there is nothing
    /// to stand off: the innermost ring left on seats its inner edge on the
    /// stack's own start, rather than opening a hole the size of a padding
    /// around nothing.
    fn take(&mut self, gap: f32, width: f32) -> (f32, f32) {
        if width <= 0.0 || self.full {
            return (0.0, 0.0);
        }
        let inner = slot_start(self.cursor, self.inner, gap);
        let outer = inner + width;
        if outer > 1.0 {
            self.full = true;
            // Spent, not free: the reach stands at the node's own edge, which
            // is where the room ran out. The mark strip then stands off that
            // rather than dropping into the slot, and it does so at the SAME
            // radius it had one step earlier, when the layer fitted exactly —
            // so the strip crosses a refusal without moving at all.
            //
            // The edge and not `outer`, which is where the layer would have
            // ended and is unbounded: `mark_inner` is what the shader sizes
            // every node's BILLBOARD on, so a strip seated on a refused width
            // asks for a quad several node radii across, on every node in the
            // window, to draw marks nobody can see.
            self.reach = 1.0;
            return (0.0, 0.0);
        }
        self.cursor = outer;
        self.reach = outer;
        (inner, outer)
    }
}

/// Where the next layer out begins, given the outer edge of the last one
/// DRAWN: a `gap` past it, or `inner` — the stack's own start — when nothing
/// has been drawn yet.
///
/// The second clause is the whole of why this is a function rather than a sum.
/// The innermost ring left on seats on the start with no padding in front of
/// it, there being no layer inside it to stand off — and three places have to
/// agree about that: [`Stack::take`] handing out a slot,
/// [`ViewConfig::rings`] placing the mark strip, and [`RingStack::edges`]
/// saying where a layer that is switched OFF would have started. A second copy
/// of the rule is how a handle comes to sit a gap off a ring that is not there.
fn slot_start(cursor: f32, inner: f32, gap: f32) -> f32 {
    if cursor > 0.0 {
        cursor + gap
    } else {
        inner
    }
}

/// A size bar's value as the picture may use it: inside `0..=high`, and 0 —
/// the off position every one of them has — where a hand-edited blob holds a
/// NaN or an infinity, which no clamp of its own would catch.
fn size(value: f32, high: f32) -> f32 {
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

    /// Whether a melody/bass mark can be drawn at all: an end has to be
    /// marked for there to BE a mark, and the depth has to leave it
    /// something to draw with.
    ///
    /// Says nothing about whether a mark is drawn NOW — that is a held note's
    /// business, per node. This is whether the layer is switched on, which is
    /// what the pane grays its Delay bar on: a mark that cannot appear has
    /// nothing for a delay to time.
    pub fn marks_draw(&self) -> bool {
        self.mark_thickness > 0.0 && (self.mark_melody || self.mark_bass)
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

    /// [`octave_gap`](Self::octave_gap) as a width the shader can cut with: on
    /// the axis, and a real number.
    ///
    /// The angular gap's [`rings`](Self::rings) — it reaches the picture as a
    /// bare uniform rather than through a radius, so this is the one place its
    /// clamp can live, and it is here rather than in
    /// [`sanitize`](Self::sanitize) for the reason every other geometry clamp
    /// is: the drawing code is reached by more routes than the persist door. A
    /// non-finite width would threshold every fragment of every sector to
    /// false, taking the whole octave layer off the node with nothing on screen
    /// to say why.
    pub fn octave_gap_width(&self) -> f32 {
        size(self.octave_gap, GAP_MAX)
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
    /// one would be silent — the markers and the rings open on one grey, so a
    /// fresh view draws the same picture either way and only a moved bar tells
    /// them apart. The reason the repair exists at all is
    /// [`lattice_ground_lightness`](Self::lattice_ground_lightness)'s: a NaN
    /// walks through a `clamp` untouched into a Newton solve that answers with
    /// whatever its guard parks on, and the drawing code is reached by more
    /// routes than the persist door.
    pub fn marker_ink_lightness(&self) -> f32 {
        if self.marker_ink.is_finite() {
            self.marker_ink.clamp(0.0, 100.0)
        } else {
            DEFAULT_RING_GROUND
        }
    }

    /// [`sounding_ink`](Self::sounding_ink) as an `L*` the colour path can
    /// actually solve for: on the axis, and a real number.
    ///
    /// Its own function for [`marker_ink_lightness`](Self::marker_ink_lightness)'s
    /// reason, with one more of its own: this end is read through a MIX against
    /// that one, and a mix carries a NaN whatever the other end holds — so a
    /// broken value here is not one label drawn the wrong grey but every label
    /// on the pane, including the ones on nodes nothing is sounding under.
    pub fn sounding_ink_lightness(&self) -> f32 {
        if self.sounding_ink.is_finite() {
            self.sounding_ink.clamp(0.0, 100.0)
        } else {
            DEFAULT_SOUNDING_INK
        }
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

    /// The block of lattice one pane's viewport shows: what the camera is
    /// actually looking at, at `aspect`. `derive_scene` is handed the result,
    /// and it is what makes the sheet scroll without end — pan far enough and
    /// the window has walked with you, so there is no edge to reach.
    ///
    /// A [`DrawnWindow`] and not a second [`ViewConfig`], because it is not a
    /// view: it says which nodes to build and nothing about how they look, and
    /// the settings it would otherwise carry a copy of are the ones every pane
    /// shares. The type is also what keeps it from being handed to a reader
    /// that wanted the naming [`reach`](Self::reach); see [`DrawnWindow`] for
    /// the picture that mix-up drew.
    ///
    /// The view's CENTER is not this function's to move: the block
    /// `derive_scene` builds is pinned to the world origin, so a center chosen
    /// here would relabel the pitches without moving the picture. What keeps
    /// the picture under the camera is
    /// [`follow_camera`](Self::follow_camera), which moves the center and the
    /// camera as one. The bounds here are absolute lattice positions about
    /// that center, and they are LOPSIDED wherever the camera's view of the
    /// sheet is — which under perspective is everywhere but dead flat.
    ///
    /// Per DRAW, never written back into the persisted view, and that is the
    /// contract rather than a style: two live copies of the lattice are drawn
    /// every frame — the docked pane and the Video tab's preview — off one
    /// camera at two different aspects, and each must build the window its own
    /// frame shows. A window stored in the shared view would be whichever copy
    /// drew last, leaving the other one drawing for a pane it is not in.
    ///
    /// Aspect, not pixels, is the whole of what it reads, which is what makes
    /// the preview honest: the preview is letterboxed to the render's aspect,
    /// so it derives the same window the mp4 will and shows exactly the nodes
    /// the export gets.
    ///
    /// The sevens window is untouched. How many sheets deep the lattice runs
    /// is a question about the music, not about the pane — the sevens axis has
    /// no screen extent of its own to be read off, only the offset each sheet
    /// is drawn at — so it keeps its bar.
    ///
    /// Falls back to this view's own window where the geometry gives no
    /// rectangle at all (see [`Camera::visible_world_bounds`], which names the
    /// one camera that does that — and it is a degenerate matrix, not a steep
    /// camera). Drawing the reach window there is a picture; an arbitrary huge
    /// number is a stall.
    pub fn scrolled(&self, camera: &Camera, aspect: f32) -> DrawnWindow {
        let center = self.center();
        let sevens = self.extent_sevens.max(0);
        let flat = |threes: i32, fives: i32| DrawnWindow {
            min: LatticePos::new(
                center.threes - threes,
                center.fives - fives,
                center.sevens - sevens,
            ),
            max: LatticePos::new(
                center.threes + threes,
                center.fives + fives,
                center.sevens + sevens,
            ),
        };
        let spacing = self.spacing;
        // A spacing of zero divides by nothing and a NaN one poisons the
        // rectangle; both leave every step at the same place, where one node
        // is the whole picture there is to draw.
        if spacing.is_nan() || spacing <= 0.0 {
            return flat(0, 0);
        }
        // The slab the sheets occupy, in world depth about the home sheet —
        // `lattice_to_world` puts the sevens axis on z, and the window's
        // center sheet is drawn at the origin.
        let depth = sevens as f32 * spacing;
        let Some(sheet) = camera.visible_world_bounds(aspect, -depth, depth) else {
            return self.reach();
        };
        // Where the pane shows the sheets all the way to the horizon there is
        // no far edge to take, and the rectangle's own is a corner's line
        // extrapolated BACKWARDS through the eye — so its two sides are not
        // the two sides of anything. Read as edges they say the picture starts
        // a step or two from the center, which is the foreground missing: at
        // 52° of pitch the rectangle runs from -3.4 to 557 while the pane is
        // showing nodes twelve steps the other way.
        //
        // Mirrored about the center instead, which has no near side to lose,
        // and the node budget rations what is left. That is the answer
        // [`MAX_DRAWN_NODES`] exists to give — there is no window that would
        // be right here — and it costs the lopsided window nothing, because
        // the tilts it wins at are all bounded (cabinet always, the other two
        // out past 45°).
        let (min, max) = if sheet.bounded {
            (sheet.min, sheet.max)
        } else {
            let mirror = sheet.min.abs().max(sheet.max.abs());
            (-mirror, mirror)
        };

        // Margin enough that a node arrives whole and off-pane rather than
        // growing at the edge: its own radius, plus a step for the NAME, which
        // is drawn beside the node and so reaches onto the pane from a node
        // that is not on it.
        let margin = spacing * (1.0 + NODE_RADIUS_FACTOR);
        // Each end of the rectangle taken on its own, in steps from the WORLD
        // ORIGIN — which is where `derive_scene` draws the center node, so a
        // world coordinate divided by the spacing IS an offset from the
        // center, and the block is anchored where the picture is.
        //
        // Taking each end separately is the whole of what keeps the block off
        // the far field. A single extent per axis has to cover the farther end
        // and then mirrors it onto the nearer one, drawing a second copy of
        // the far reach behind the camera that no pane ever shows; see
        // [`DrawnWindow`] for what that costs under perspective.
        //
        // Saturating, so an enormous rectangle lands on the extent bound
        // rather than wrapping. (A NaN casts to 0, which is the center node —
        // also drawable.)
        let offset = |world: f32, round: fn(f32) -> f32| {
            let steps = round(world / spacing);
            let steps = if steps.is_nan() { 0 } else { steps as i32 };
            steps.clamp(-MAX_DRAWN_EXTENT, MAX_DRAWN_EXTENT)
        };
        // World x is the fifths axis and world y the thirds one, which is the
        // one place that mapping has to be undone rather than applied.
        //
        // No center is derived here, and that is deliberate rather than an
        // omission: it would be a relabeling the picture cannot show, and it
        // is not free — under a tilted camera the rectangle's center is a
        // multi-step quantity that moves with every pan and zoom, so rounding
        // it to a step made the pitch drawn at a fixed point on screen JUMP a
        // cell, several times per drag. The center is
        // [`follow_camera`](Self::follow_camera)'s alone, which moves it and
        // the camera together. Only the BOUNDS move here.
        let mut window = DrawnWindow {
            min: LatticePos::new(
                center.threes + offset(min.y - margin, f32::floor),
                center.fives + offset(min.x - margin, f32::floor),
                center.sevens - sevens,
            ),
            max: LatticePos::new(
                center.threes + offset(max.y + margin, f32::ceil),
                center.fives + offset(max.x + margin, f32::ceil),
                center.sevens + sevens,
            ),
        };
        window.fit_to_node_budget(center);
        window
    }

    /// How far out a played pitch is hunted for a spelling and a node, as a
    /// block — the naming REACH, centered on the camera but sized by the
    /// setting rather than by any pane.
    ///
    /// Not what is drawn; [`scrolled`](Self::scrolled) is. This is one answer
    /// the whole UI shares, so that a name does not change under a pan, and it
    /// is the fallback for the readers that want the picture's window on a
    /// frame where no lattice pane drew one.
    pub fn reach(&self) -> DrawnWindow {
        let center = self.center();
        let extent = LatticePos::new(
            self.extent_threes.max(0),
            self.extent_fives.max(0),
            self.extent_sevens.max(0),
        );
        DrawnWindow { min: center - extent, max: center + extent }
    }

    /// Keep the window's center under the camera, moving both together so the
    /// picture does not stir: a whole step added to the center subtracts one
    /// spacing from every node's world position, and taking the same off the
    /// camera's target leaves each node exactly where it was on screen.
    ///
    /// This is what puts the picture under the camera at all, and both windows
    /// depend on it. The block `derive_scene` builds is pinned to the world
    /// origin, so the camera has to be brought back to the origin for there to
    /// be anything in front of it; and the reach the note names are chosen out
    /// of (see [`reach`](Self::reach)) is centered here, so it has to follow
    /// or scrolling away would leave every note on screen outside the set that
    /// can name it.
    ///
    /// The x and y axes are carried into the center, which is the pair the
    /// lattice's own sheet runs on. The DEPTH is zeroed instead, and that is
    /// not the same operation wearing a different hat: `pan` moves along the
    /// camera's right and up vectors, which under any projection but cabinet
    /// carry a z component, so dragging sideways walks the eye through the
    /// sheets as well as across them. There is nowhere for that to go — which
    /// sheet is home is [`center_sevens`](Self::center_sevens), a setting with
    /// a bar, not somewhere a sideways drag should arrive — and left to
    /// accumulate it is unbounded: 2500 pan gestures under perspective put the
    /// target 747 spacings off the sheet, with the lattice long gone from the
    /// pane and every frame still deriving twenty thousand nodes for it.
    /// Zeroing it makes a pan mean the same thing under all three
    /// projections, which is a slide ACROSS the sheet.
    ///
    /// So the target is left inside one cell of the origin, on every axis.
    /// That matters beyond the picture: the target is persisted, and it is the
    /// number a scroll without end would otherwise grow forever.
    ///
    /// Once a frame, from the docked lattice — which the offline renderer also
    /// reaches, drawing through the same pane function. It is idempotent on a
    /// target already inside its cell, so the extra call costs a render
    /// nothing and its frames stay reproducible.
    pub fn follow_camera(&mut self, camera: &mut Camera) {
        if self.spacing.is_nan() || self.spacing <= 0.0 || !camera.target.is_finite() {
            return;
        }
        // Bounded well inside `i32`, because the center is added to an extent
        // (`reach`) and that sum must not overflow. Saturating the center
        // instead leaves a target that cannot be reduced — it keeps
        // stepping and the center cannot take the step — so the two stop
        // agreeing and the reach comes out EMPTY, which reads as every note
        // being off the lattice, permanently. Clamping the step keeps them in
        // step: an absurd target walks back a bound's worth per frame and
        // arrives.
        let steps = |world: f32| {
            let steps = world / self.spacing;
            if steps.is_finite() {
                steps.round().clamp(-(MAX_CENTER as f32), MAX_CENTER as f32) as i32
            } else {
                0
            }
        };
        let (fives, threes) = (steps(camera.target.x), steps(camera.target.y));
        // Saturating BEFORE the clamp, not after: a center already at the
        // bound plus a step of the bound is `2^31`, one past what an `i32`
        // holds, so the clamp would be handed a number that had already
        // wrapped negative.
        self.center_fives = self.center_fives.saturating_add(fives).clamp(-MAX_CENTER, MAX_CENTER);
        self.center_threes =
            self.center_threes.saturating_add(threes).clamp(-MAX_CENTER, MAX_CENTER);
        camera.target.x -= fives as f32 * self.spacing;
        camera.target.y -= threes as f32 * self.spacing;
        camera.target.z = 0.0;
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
    /// look like. Fresh, they are 0.35 and 0.15, not 0.
    pub fn sanitize(&mut self) {
        let fresh = ViewConfig::default();

        // The window's own integers, which are the one group here that is not
        // a float. `DrawnWindow::count` multiplies the three spans together
        // and `reach` adds each center to its extent, so a blob carrying a
        // billion sheets overflows both — and the derived window now counts
        // nodes on every draw, which puts that arithmetic in the frame rather
        // than at the edge of it. The sevens extent is held to what its bar
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
        self.extent_sevens = self.extent_sevens.clamp(0, 4);
        self.extent_threes = self.extent_threes.clamp(fresh.extent_threes, MAX_DRAWN_EXTENT);
        self.extent_fives = self.extent_fives.clamp(fresh.extent_fives, MAX_DRAWN_EXTENT);
        self.center_sevens = self.center_sevens.clamp(-MAX_CENTER, MAX_CENTER);
        self.center_threes = self.center_threes.clamp(-MAX_CENTER, MAX_CENTER);
        self.center_fives = self.center_fives.clamp(-MAX_CENTER, MAX_CENTER);

        // Fit the label scale to what its bar offers. It multiplies a FONT
        // SIZE, and the bar cannot produce a nonsense value where a
        // hand-edited blob can: a non-finite one reaches egui as a glyph with
        // no image, so every label silently vanishes, and a huge one asks the
        // rasterizer for a glyph wider than the texture atlas can hold.
        self.label_scale = if self.label_scale.is_finite() {
            self.label_scale.clamp(0.3, 3.0)
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

        // The shimmer's four knobs, on the same grounds and against the same
        // hole in `clamp`. `derive_scene` clamps all four into their ranges
        // every frame, which is what the shader trusts — and a NaN walks
        // through a clamp untouched, because every comparison against it is
        // false. From there it is a divide (the period), a `pow` exponent
        // (the softness) and two mixes, so ONE non-finite number in a
        // hand-edited blob NaNs the sheet, and a NaN sheet takes the rings and
        // the slices they name with it wherever the mode is on. Repaired here rather
        // than in `derive_scene` because this is the blob's own door: the bars
        // cannot reach these values, so a view that holds one got it from a
        // file.
        // The mark delay, against that same hole: it is added to a timestamp
        // and the sum divided by the attack, so a non-finite one poisons the
        // ease of every ring. The symptom is the rings VANISHING, not drawing
        // wrong — a NaN level fails `Mark::add`'s `>=` (NaN answers no to
        // every comparison), so the level stays at 0 while the slot bit is
        // still set, and the shader multiplies the ring's coverage away to
        // nothing. Silent, and it takes the whole layer wherever the marks
        // are on. `derive_scene` clamps the RANGE (the bar cannot leave it; a
        // file can), which is again no guard against a NaN.
        self.mark_delay = finite_or(self.mark_delay, 0.0);

        // The envelope's shape, against the same hole. `Envelope::approach`
        // guards its own arithmetic against a non-finite duration or shape —
        // it has to, being reachable from a shell that never went through
        // this door — but it guards by treating the transition as already
        // OVER, so a NaN shape would show as a curve that silently
        // straightens: the picture quietly drawing something other than what
        // the bar reads out, which is exactly what this door is for. The
        // duration beside it is the Fade param rather than a blob field, and
        // has no door here to need.
        self.fade_shape = finite_or(self.fade_shape, 0.0);

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
        // a blob that has been through this door holds a ring somebody can see.
        //
        // Where the ring SITS is not repaired here, because it is not stored: a
        // width is a width whatever is inside it, and the stack is what turns
        // the four of them into radii (`rings`).
        self.spectral_ring_width = finite_or(self.spectral_ring_width, fresh.spectral_ring_width)
            .clamp(0.0, RING_WIDTH_MAX);
        // The two paddings, against the same hole and for the reason the width
        // above is repaired rather than left to the picture: [`GAP_MAX`] is a
        // ceiling the two bars are BUILT from, so a blob written when it stood
        // higher carries a number no bar can reach, and `rings` holds it to the
        // ceiling for the picture while the field keeps what the bar reads out.
        // The stack under the bar then draws one padding and the bar names
        // another, which is a value read out one way and drawn another.
        //
        // The clamp in [`rings`](Self::rings) stays where it is — the picture is
        // reached by more routes than this door — and this one makes the number
        // the door lets through a number the picture agrees with.
        self.ring_gap = finite_or(self.ring_gap, fresh.ring_gap).clamp(0.0, GAP_MAX);
        self.octave_gap = finite_or(self.octave_gap, fresh.octave_gap).clamp(0.0, GAP_MAX);
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
        // The lit end of the labels' pair, on that same axis and repaired for
        // the markers' reason. It reaches no gradient either, so a broken one
        // costs the type on a sounding node and nothing else — but it costs it
        // on every node at once, the value being one end of a mix rather than a
        // grey drawn straight.
        self.sounding_ink = finite_or(self.sounding_ink, fresh.sounding_ink).clamp(0.0, 100.0);

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
        // The Shadow, which every caster's quad is grown by: a number from
        // outside the bar is a quad nothing can fill.
        self.glow_shadow =
            finite_or(self.glow_shadow, fresh.glow_shadow).clamp(0.0, GLOW_SHADOW_MAX);
        // The SHARES — of the frame a shadow takes, of the light a lit slice
        // stands in, of the light's own peak, of a whole turn — so their range
        // is the unit interval.
        self.glow_shadow_depth =
            finite_or(self.glow_shadow_depth, fresh.glow_shadow_depth).clamp(0.0, 1.0);
        // The shadow's own three, none of which is a share: a gain, an exponent
        // and a ratio, each with a ceiling of its own beside the width's.
        self.glow_shadow_gain = finite_or(self.glow_shadow_gain, fresh.glow_shadow_gain)
            .clamp(0.0, GLOW_SHADOW_GAIN_MAX);
        self.glow_shadow_curve = finite_or(self.glow_shadow_curve, fresh.glow_shadow_curve)
            .clamp(GLOW_SHADOW_CURVE_MIN, GLOW_SHADOW_CURVE_MAX);
        self.glow_shadow_name = finite_or(self.glow_shadow_name, fresh.glow_shadow_name)
            .clamp(0.0, GLOW_SHADOW_NAME_MAX);
        // And the distance family's two, held to their bars whatever row is
        // picked: a blob switched back to a Gaussian still carries them, and a
        // number out of range would be waiting there when it is switched back.
        self.glow_shadow_shape = finite_or(self.glow_shadow_shape, fresh.glow_shadow_shape)
            .clamp(crate::SHADOW_SHAPE_MIN, crate::SHADOW_SHAPE_MAX);
        self.glow_wash = finite_or(self.glow_wash, fresh.glow_wash).clamp(0.0, 1.0);
        self.glow_blend = finite_or(self.glow_blend, fresh.glow_blend).clamp(0.0, 1.0);
        // The light's own pair, in seconds, on the ring's rule: a bar's range,
        // and a poisoned number repaired to the fresh value rather than left
        // to make a coefficient nothing can carry.
        self.glow_attack =
            finite_or(self.glow_attack, fresh.glow_attack).clamp(0.0, GLOW_BALLISTICS_MAX);
        self.glow_release =
            finite_or(self.glow_release, fresh.glow_release).clamp(0.0, GLOW_BALLISTICS_MAX);

        self.shimmer_speed = finite_or(self.shimmer_speed, fresh.shimmer_speed);
        self.shimmer_width = finite_or(self.shimmer_width, fresh.shimmer_width);
        self.shimmer_intensity = finite_or(self.shimmer_intensity, fresh.shimmer_intensity);
        self.shimmer_softness = finite_or(self.shimmer_softness, fresh.shimmer_softness);

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
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The `L*` a fresh [`ViewConfig::lattice_ground`] opens on — and a fresh
/// [`ViewConfig::marker_ink`] with it, so the resting picture opens as ONE grey
/// and the two bars start as a pair to be moved apart. Named because both
/// `_lightness` accessors need it without building a whole fresh view to read
/// one field off. Named, and not a second value: the `Default` below is written
/// in terms of it, the way it is written in terms of `octaves::DEFAULT_COUNT`.
///
/// Which grey this is, and why that rung, is at
/// [`skin::surface_faint_color`](crate::skin::surface_faint_color).
const DEFAULT_RING_GROUND: f32 = 20.0;

/// The `L*` a fresh [`ViewConfig::sounding_ink`] opens on: the top of the axis,
/// so a sounding name is white and the fresh distance between the two ends of
/// the label pair is the whole of it. Named for [`DEFAULT_RING_GROUND`]'s
/// reason — the `_lightness` accessor needs it without building a fresh view to
/// read one field off — and the `Default` below is written in terms of it.
///
/// White rather than a rung of the resting picture, because the two ends are
/// answering different questions: the resting one is dialled against the ground
/// the lattice's structure has to stay legible over, and this one against the
/// light a note is putting out under it.
const DEFAULT_SOUNDING_INK: f32 = 100.0;

/// The look a fresh view starts in, and the single source of every field's
/// fallback: the container-level `#[serde(default)]` on the struct means a
/// blob missing a key picks its value up from here.
impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
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
            // The home sevens sheet alone. A sheet either side (extent 1)
            // shows the septimal axis without anyone having to go find it;
            // the tradeoff is that nothing tells the eye which sheet a node
            // is on until the sevens layer settings below are turned down to
            // read as an annotation rather than a second sheet (see
            // sevens_size).
            extent_sevens: 0,
            center_threes: 0,
            center_fives: 0,
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
            show_labels: true,
            // Where the music has been is most of what the lattice is for, so
            // the fresh view opens naming it: every visited node keeps its
            // name, and none of the unvisited ones carry text yet.
            note_names: NoteNames::Past,
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
            // a dimmer middle over a shallower brightness ramp, and a little
            // less chroma.
            pitch_gradient: Gradient {
                hue_start: 257.842_65,
                hue_span: 190.0,
                lightness: 53.0,
                lightness_ramp: 31.0,
                // Denominated in the floor every hue can hold (see
                // `chroma_of`), which is a tighter axis than the per-hue
                // ceiling: this is the fraction holding the mean colorfulness
                // of THIS arc where 0.601_670_8 of the ceiling held it, so the
                // lattice a fresh install draws is as colored as it was and
                // only spends that color evenly. Retuning the type's own
                // `default_chroma` does not reach here — the two are
                // independent numbers, which is the point of writing this out.
                chroma: 0.793_2,
                // Flat, where the brightness ramp is not: the hue arc is
                // already spending color on pitch, and a chroma ramp over it
                // would say the same thing twice at the price of one end of
                // the range going grey. Dialled rather than opened on — see
                // `default_chroma_ramp`, which the type's own default takes
                // for the same reason.
                chroma_ramp: 0.0,
            },
            // A narrow octave band, stopping short of the quad edge, with a
            // tight gap everywhere: the octaves read as a ring of distinct
            // marks rather than a solid annulus, and every layer keeps clear
            // space around it. (The backdrop that holds the whole ring's shape
            // behind them is fixed on.) The stack is middle, audio ring, gap,
            // band, gap, marks — the node's own light fills the middle and the
            // two rings stand around it (see ring_inner).
            band_width: 0.163_084_63,
            // The stack seated just past halfway out, so a node reads as a lit
            // middle with its readings around it: the glow fills the whole of
            // the space this leaves (see glow_reach), the analyzer's ring is a
            // thin annulus at its edge, and the octaves stand outside that.
            // Dialled to 0 the stack seats on the center instead and the audio
            // ring's wedges close into pie slices, which is the same node read
            // as one solid measurement.
            ring_inner: 0.551_335_3,
            // The two gaps are one number here: the radial padding is what puts
            // the band at its outer edge, and the same width cut angularly is
            // the slicing that reads as distinct marks. They are two bars
            // because a node has two spacings to set, not because the fresh
            // one wants them apart — the picture this describes is one a
            // person can meet by dialling neither.
            ring_gap: 0.05,
            octave_gap: 0.05,
            // The rung of the chrome's own ladder the rings stand on: `L*` 20.0
            // is the skin's `surface_faint`, two rungs ABOVE the well grey
            // (4.7) the lattice pane stands on, and clear of the panel between
            // them (8.8), which is near enough the ground to read as a smudge
            // on it rather than as a raised surface. A quiet ring is therefore
            // a faintly raised backdrop that is plainly still a reading —
            // `the_fresh_ground_is_the_skins_faint_surface` holds the number to
            // the skin, so retuning that rung and leaving this behind is a test
            // failure rather than a drift.
            lattice_ground: DEFAULT_RING_GROUND,
            // The same rung, so the fresh lattice is one grey at rest and the
            // pair reads as a pair. Where they part is a picture a person dials
            // for — the glow on, the ground down, the markers held up to keep
            // the positions legible — and there is no fresh look that guesses
            // it, because it depends on how much light is behind the notes.
            marker_ink: DEFAULT_RING_GROUND,
            // The other end of the label pair, as far from that grey as the
            // axis goes: type on a sounding node is white, and what says a node
            // is sounding is its own light behind a name that has stepped out
            // of the resting field. A fresh view is therefore a picture where
            // the bar is doing something — it is a look to dial down from
            // rather than one to discover.
            sounding_ink: DEFAULT_SOUNDING_INK,
            // Five octaves to the turn with middle C straight up — C1..C5 in
            // the DAW's numbering, the register a keyboard part lives in, at
            // 72 degrees an octave, with a two-octave fringe either end (see
            // octave_extras) narrower than a full-size slice and graded from
            // the outer edge in.
            octave_count: crate::octaves::DEFAULT_COUNT,
            octave_center: crate::octaves::DEFAULT_CENTER,
            octave_extras: 2,
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
            // A thin ring, the middle ahead of it (see ring_inner) holding
            // most of the room inside the band. It still carries one wedge per
            // octave, each a level or a window of spectrum, and the gaps either
            // side still make it a ring rather than a thick edge on the node's
            // light.
            spectral_ring_width: 0.061_113_536,
            // A narrow wedge — see the field for why a window this size and
            // not the octave that makes the ring continuous. Dialled rather
            // than taken from the live session, which had it parked at the
            // disabled bar's floor while reading Fold: it only zooms the
            // Spectrum reading's wedge.
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
            // Near enough a square law (the exponent lands at 2.05): enough
            // that a release leaves promptly and settles instead of sliding
            // out at one rate, and not so much that the tail is over before
            // the ear has finished the note. The straight line is still one
            // drag away.
            fade_shape: 0.35,
            // Both ends marked: the marks are subtle enough to live with
            // always on, and a chord's outer voices are worth seeing without
            // having to go turn something on first.
            mark_melody: true,
            mark_bass: true,
            // A shallow step past the band — about a third of the band's own
            // width, so a mark reads as its slice carrying on rather than as a
            // second ring around everything.
            mark_thickness: 0.062_530_935,
            // Short of a passing sixteenth (125ms at 120bpm), but still well
            // off the bar's 0 floor: a mark outlives its key (see
            // `mark_delay`), and at 0 every momentary crowning fades its way
            // OUT over the whole Fade, so lifting a chord one key at a time
            // leaves a fading mark on nearly every note of it.
            mark_delay: 0.102_448_754,
            pulse_marks: Pulse::Bands,
            // The sheet the marks above wear. A period well under one node's
            // spacing puts several of them across every mark, so this reads as
            // a fine texture ON the marks rather than as light crossing the
            // lattice — which is what keeps it off the reading of the octave
            // slice each mark extends, the one place the sheet touches the
            // glyph layer. Half depth and a slow pace hold it there; wider
            // and deeper it would be a sweep crossing the lattice instead.
            shimmer_speed: 0.335_761_5,
            shimmer_width: 0.639_271_56,
            shimmer_intensity: 0.517_033_16,
            shimmer_softness: 1.0,
            // A small marker: arms a fifth of the way out the quad, which is
            // well inside the middle a node's rings stand around (see
            // ring_inner, at 0.55), so a note arriving covers its own marker
            // rather than growing out of it.
            plus_arm: 0.2,
            // Just over half the arm's length across, so the fresh cross reads
            // as two strokes rather than as a blob with dents — at this
            // proportion it carries about 60% of the ink a disc of the same
            // reach would (8t - 4t^2 against pi), which is light enough to be
            // a ground for the music to arrive on.
            plus_width: 0.11,
            // A little under half the arm, so a fresh plus reads as reaching
            // out of its crossing rather than as a drawn glyph: the ends
            // arrive at nothing rather than stopping at something.
            plus_taper: 0.09,
            meantone: false,
            meantone_auto: true,
            marvel: false,
            marvel_auto: true,
            frameless: false,
            show_perf: false,
            show_perf_detail: false,
            render_scale: 1.0,
            // A halo at about four fifths strength: a node's rings are quiet
            // shapes, and the bloom is what gives them presence.
            bloom_strength: 0.806_154_85,
            // The node glow ON, it being the only light a node has: the rings
            // are crisp shapes and the middle they stand around (see
            // ring_inner) is empty ink, so a fresh view with the reach at 0
            // would be a lattice of hollow annuli. A reach of about a third of
            // a node past its rim, which is a halo plainly there and well short
            // of the neighbour it would otherwise reach.
            glow_reach: 0.35,
            glow_strength: 1.0,
            glow_curve: GlowCurve::default(),
            // A sixth of a radius, so σ is a twelfth of one: the shadow and the
            // light either side of it read as one blur rather than as a cut
            // through it, which is what a band with a short edge, laid against
            // a node's own dark rings, does not.
            glow_shadow: 0.16,
            // Most of the frame taken under a ring, and not all of it: a ring
            // in a dim pool of its own halo reads as shade, where the whole of
            // it taken away reads as a black annulus drawn round the node.
            glow_shadow_depth: 0.85,
            // Calibrated by eye on a name at the fresh view (#498, PR B): at 1
            // a fresh name's shadow is a faint tint beside the ring's, at 4 a
            // hairline casts as a block. A ring and a cross take the same
            // number, which is what makes one Shadow bar one darkness across
            // the picture.
            glow_shadow_gain: 2.5,
            // The straight line, and 1 is the only value that is one: the
            // shadow's profile is the kernel's own, which is what every reading
            // of the Shadow bar is calibrated against.
            glow_shadow_curve: 1.0,
            // One width across the whole picture, a ring, a cross and a name
            // alike. The bar exists to ask whether a letterform wants
            // otherwise; the fresh view is the answer being no.
            glow_shadow_name: 1.0,
            // One Gaussian, which is one cell per caster and the picture the
            // rest of the Shadow section is calibrated on.
            glow_shadow_kernel: ShadowKernel::Gaussian,
            // The plain exponential, which is the ceiling of the shape bar and
            // the only value in its range with no knee anywhere in the decay.
            glow_shadow_shape: 1.0,
            // The whole field, which is the fresh picture with no bar in it:
            // every piece of the lattice's ink wears the light it stands in,
            // and the bar is there to pull a SOUNDING slice back out of its own
            // halo without the grey around it going with it.
            glow_wash: 1.0,
            // The colour averaged half way round, which keeps a chord's hues
            // as arcs while a lone wedge still tints the whole halo.
            glow_blend: 0.5,
            // Slow and fluid, which is what the pair is for: a light that
            // arrives inside a third of a second and takes a couple of seconds
            // to leave, so a halo trails the notes that lit it instead of
            // stepping with them.
            glow_attack: 0.3,
            glow_release: 2.5,
        }
    }
}

/// Per-frame mirrors of the host-automatable appearance parameters. The
/// shell copies these from its param backend every frame (see root_ui).
/// Deliberately NOT part of [`ViewConfig`] or the persist blob: the param
/// system owns these values, and persisting a copy would create a second
/// source of truth that's dead on arrival at load time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameParams {
    /// Seconds a note takes to arrive, and — once it is released — to leave,
    /// for EVERY layer of the node: the audio ring, the octave glyphs, and the
    /// melody/bass marks.
    ///
    /// One time per DIRECTION, so the lattice answers the keys the way it
    /// lets go of them; a note that came up quickly and left slowly would
    /// read as two instruments. It costs a short note no brightness, only
    /// time at full, because the arrival lands before the departure starts
    /// (see [`ViewConfig::envelope`]).
    ///
    /// One time per LAYER too, so an arrival or a release reads as a single
    /// gesture instead of pieces of the node moving at different rates. The
    /// octave sectors and the melody/bass marks arrive on the same ramp as
    /// each other, because a mark and the sector it links back to
    /// belong to one note — [`ViewConfig::mark_delay`] moves a ring's ramp
    /// LATER without changing its rate, which is the one thing that may
    /// differ.
    pub fade_time: f32,
    /// Pitch (MIDI note) mapped to the darkest gradient color.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        FrameParams { fade_time: 1.0, darkest_pitch: 24.0, brightest_pitch: 108.0 }
    }
}
