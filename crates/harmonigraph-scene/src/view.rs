//! [`ViewConfig`] — the persisted, non-automatable visual settings — plus the
//! per-frame [`FrameParams`] mirror of the host-automatable appearance
//! parameters.

use crate::spectral::SpectralReading;
use crate::style::{Gradient, Pulse, SevensLabel};
use crate::{
    Camera, CORE_RADIUS_MAX, GAP_MAX, MARK_THICKNESS_MAX, MAX_DRAWN_NODES, NODE_RADIUS_FACTOR,
    RING_WIDTH_MAX,
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

impl DrawnWindow {
    /// Every position in the block, threes outer and sevens inner.
    ///
    /// The order is load-bearing: [`index_of`](Self::index_of) inverts it, so
    /// `derive_grid` can find a neighbour by arithmetic instead of hashing.
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
    // How the sheets other than the home one draw. `sevens_size` and
    // `sevens_label` go inert while `extent_sevens` is 0, which is where a
    // fresh view starts; `sevens_gutter` does NOT — it is named for this
    // layer but cuts the grid under a sounding node at any extent (see its
    // own doc below, and `a_flat_lattice_still_clears_its_grid`).
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
    /// Width of the dark gutter a node clears around itself, in quad UV
    /// units — the units a FULL-SIZE node uses, so the gap comes out the
    /// same width on screen whatever size the node it belongs to draws at.
    /// (The shader divides by the node's size factor to get there.) A gap
    /// that shrank with its node read as a property of the note rather than
    /// of the layer it sits on. 0 draws none.
    ///
    /// This is what lets the sevens layer OVERLAP the home sheet instead of
    /// needing room of its own: the node punches its own footprint out of
    /// whatever it crosses and sits in the hole, so a small node stays
    /// legible over a large one. It costs no layout space at all, which is
    /// the whole point — the alternative is shrinking the 5-limit sheet to
    /// open up clearance, and the 5-limit sheet is what you came to look at.
    ///
    /// FOOTPRINT means the node's own shape, this far out from it, and not a
    /// circle sized to hold it: a node reaching a melody mark on one octave
    /// bulges over that wedge and hugs its rings everywhere else. A circle
    /// answers with the widest thing the node reaches in ANY direction, which
    /// on a marked node is a gap wider than itself all the way round — so the
    /// hole stops reading as the node's and starts reading as empty space
    /// somebody cut out.
    ///
    /// The shape is the shader's (`node_clearing`), off the same radii that
    /// draw the layers, and it is built one LAYER at a time: each clears its
    /// own footprint at the level it is drawn at, and the hole is whichever of
    /// them claims a pixel most. Which is what lets a node wearing an audio
    /// ring and no note clear at all — its ring's level is the gate's, where
    /// the note's is nothing — and equally what stops that node clearing the
    /// band-sized hole a node-wide level would give it. A layer nobody is
    /// drawing clears nothing.
    ///
    /// It clears to the GROUND the pass is composited over, which the shell
    /// hands in (see [`Scene::background`](crate::Scene::background)). With
    /// no color of its own a premultiplied layer knocks out to black, and
    /// black is several shades darker than this skin's panel, so the
    /// clearing announced itself as a plate sitting on the picture rather
    /// than disappearing into the ground.
    ///
    /// It fades rather than ending at a rim, over a band of its own
    /// ([`sevens_gutter_soft`](Self::sevens_gutter_soft)) — a hard edge
    /// cutting across a lit ring reads as a bite taken out of it. And its
    /// STRENGTH is each layer's own envelope, so a clearing fades out exactly
    /// as the ink in it does while holding its width; a node drawing nothing
    /// clears nothing at all.
    ///
    /// Named for the sevens layer it was built for, but not confined to it:
    /// the home sheet clears too, and at any sevenths extent — with the
    /// lattice flat there is no sheet behind to hide, but the grid lines
    /// are still cut, which is a look worth having on its own.
    pub sevens_gutter: f32,
    /// How wide the clearing's fade is, same units — and deliberately NOT
    /// derived from the reach above. Tying the two into one number (fade
    /// pinned to twice the reach) means widening the gap also blurs it,
    /// with no way to ask for a wide crisp one or a narrow soft one.
    ///
    /// One CONTROL is a different question from one number, and the Nodes
    /// section's Clearance bar is one: both are distances from the node's rim, so
    /// they are two points on one axis and the bar carries a handle at each.
    ///
    /// The clearing is solid out to `reach - fade` past the node's own body
    /// and gone by `reach`, so the reach is exactly where it ends whatever the
    /// fade does — both measured from that body rather than from a rim, so
    /// they follow its shape (see [`sevens_gutter`](Self::sevens_gutter)).
    /// A fade wider than the reach eats outward rather than inward: the body
    /// itself is the one part that must always be cleared.
    pub sevens_gutter_soft: f32,
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
    // How a sounding node's core is painted has no field here: the core is
    // the steady disc-and-glow, and nothing switches it. The field styles
    // that stood beside it (Vortex, Checker and Spiral) are gone, and with
    // them the `node_style` key and the per-node seed that animated them.
    // Saved blobs still carry that key, naming any of the SEVENTEEN the enum
    // answered to — those three, the Steady it defaulted to, and the thirteen
    // trimmed before them that its serde aliases went on loading (Breathe,
    // Sparks, Wire, Corona, Plasma, Aurora, Marble, Lava, Filament, Stripes,
    // Rings, Tiles, Pinwheel). Serde ignores unknown keys, so such a blob
    // loads intact and drops the key on the next save. This is the only
    // surviving record of that set, which is why it names it in full.
    /// The curve the low-to-high pitch gradient follows, as its six knobs
    /// (see [`Gradient`]). Every pitch-colored shape in the scene reads
    /// it through one table, so this is the only place the gradient is set.
    pub pitch_gradient: Gradient,
    /// The core's solidity, 0..1: a soft glow at 0, morphing continuously
    /// to the classic solid orb at 1 (the disc fades in over its glow
    /// skirt and its edge crisps). Inert while the core is off.
    pub core_solidity: f32,
    /// The core's radius, in quad UV units (0 = node center, 1 = quad
    /// edge; the classic disc edge sits at 0.46). Sizes the disc and its
    /// glow together. **A radius of 0 turns the core off** — the outer
    /// octave glyphs then carry the note alone.
    ///
    /// The one size on the node measured from the CENTER, because it is the
    /// only layer that starts there: everything outside it is a ring, and a
    /// ring is sized by how thick it is (see [`rings`](Self::rings)). Widening
    /// the core therefore slides every ring outward with it rather than eating
    /// into the innermost one.
    pub core_radius: f32,
    /// How thick the octave band is — the MIDI ring, in quad UV units, whose
    /// inner edge is wherever the layer inside it ended plus one
    /// [`ring_gap`](Self::ring_gap) (see [`rings`](Self::rings)). Every outer
    /// style fits its glyphs' radial footprint to it, so the band IS the
    /// glyph set's radial extent.
    ///
    /// **0 turns the octave layer off**, as a radius of 0 turns the core off,
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
    /// The one padding on a node, in quad UV units: the RADIAL gap between one
    /// ring of the stack and the next (see [`rings`](Self::rings)), and the
    /// ANGULAR gap between one octave sector and the next, which is also what
    /// separates a melody/bass mark from the band it continues.
    ///
    /// One number for both axes, because a node reads as one rhythm of
    /// interruptions: a ring standing off its neighbour by more than a sector
    /// stands off the sector beside it makes two spacings out of one idea, and
    /// the eye reads the wider of them as the structure. A mark IS its sector
    /// continued outward, which is the case in miniature — a stand-off
    /// narrower than the gap to the next sector would read as a join rather
    /// than as the same interruption seen twice.
    ///
    /// 0 closes the whole node up: the sectors become a solid annulus, the
    /// marks seat against the band, and every ring meets the one inside it.
    /// A gap is only ever spent between two DRAWN layers, so a ring dialled to
    /// 0 costs its own slot and the gap that would have stood it off together.
    pub ring_gap: f32,
    /// The whole lattice AT REST, as an `L*` 0..100 — one neutral grey under
    /// the three surfaces that draw where nothing is sounding:
    ///
    /// - the **grid** between node positions, every segment of it, and the
    ///   colour a sevens link fades in to as a note hangs from it
    ///   ([`derive_grid`](crate::derive::derive_grid));
    /// - the **audio ring** wherever it reads silence, which its ramp is
    ///   re-anchored to open on ([`ring_gradient`](crate::ring_gradient));
    /// - the **MIDI ring**'s octave slices that are not sounding, which ARE
    ///   this colour, with a sounding octave's pitch painted over them.
    ///
    /// One number under all three, like [`ring_gap`](Self::ring_gap) above it,
    /// and for the same reason: they are one picture read together — two annuli
    /// a gap apart with the lines of the lattice running into them — so a
    /// ground that differed between them says the three are different KINDS of
    /// thing when the only thing they have in common is being empty. Each
    /// deriving its own ground instead — the audio ring off the analyzer's
    /// gradient (whose dark end carries that gradient's own hue), the MIDI ring
    /// off the note's colour whitened and laid on at a fixed opacity, the grid
    /// off the CHROME's hairline at another — lands three near-greys a hair
    /// apart in tint, and no bar can dial three routes onto one value.
    ///
    /// **Neutral**, because the ground is what they have in common rather than
    /// a colour any of them owns: it carries no hue, and each surface's light
    /// is added over it.
    ///
    /// Stated in `L*` because that is the axis the ask is on: perceived
    /// brightness, the same units [`Gradient::lightness`](crate::Gradient) is
    /// authored in, so a ground and a gradient can be compared by their
    /// numbers. There is no off position and it needs none — each layer has its
    /// own ([`grid_thickness`](Self::grid_thickness) for the lines, a width for
    /// each ring). What the bottom of the bar reaches is black, which against
    /// this skin's panel reads as holes punched through the lattice; a little
    /// above it, at the panel's own `L*` (8.8 on the fresh skin), the whole
    /// resting picture vanishes into the pane together and only sounding notes
    /// draw.
    pub lattice_ground: f32,
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
    // Which shimmer sweeps the octave glyphs has no field here: the sheet is
    // the melody/bass marks' alone (`pulse_marks`), and the octave layer draws
    // steady whatever those are doing. Saved blobs still carry the
    // `pulse_octaves` key, naming a pattern nothing reads any more; serde
    // ignores unknown keys, so such a blob loads intact, opens on the steady
    // octave layer, and drops the key on the next save.
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
    /// exactly as a radius of 0 is what turns the core off. One off switch per
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
    /// Narrow fresh, at 10¢, because just intonation is what this is for:
    /// partials of just-tuned notes land dead on nodes, so a narrow kernel
    /// draws them crisp and rejects everything between. 12-TET material reads
    /// softer at that width and wants the bar dragged right — a tempered major
    /// third's 5th harmonic sits 13.7¢ off the node it belongs to and a
    /// harmonic seventh 31¢ off.
    pub spectral_width: f32,
    /// How thick the audio ring is, in the same quad UV units as the octave
    /// band's own width ([`band_width`](Self::band_width)) — and **0 turns the
    /// ring off**, which is the only switch the LAYER has. Which nodes wear it
    /// is the gate's ([`spectral_ring_gate`](Self::spectral_ring_gate)), and
    /// the two are asked in that order: no width is no ring anywhere, and the
    /// gate never runs.
    ///
    /// It sits between the core and the band in the stack
    /// ([`rings`](Self::rings)), so its inner edge is the core's radius plus a
    /// gap and everything outside it moves when it is dragged. INSIDE the band
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
    /// Fresh at 200¢, a whole tone across a wedge. Wide enough to hold every
    /// miss the material makes — a tempered major third's 5th harmonic sits
    /// 13.7¢ off its node, a harmonic seventh 31¢, and the syntonic comma
    /// between two just spellings is 21.5¢ — and narrow enough that those are
    /// a seventh of the wedge apart rather than a fiftieth, which is the
    /// difference between reading a detuning and taking it on trust.
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
    /// sawtooth 24 dB down over the fresh window: the fold rings 601 nodes of
    /// 1025 at 0.1 and 177 at 0.4, where the spectrum rings all 1025 at both
    /// and none by 0.6.
    pub spectral_ring_gate: f32,
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
    // An unlit node has no mark of its own: the grid lines between node
    // positions are the whole of what says a position is there, and they say
    // it on the home sheet alone (see `derive_grid`) — off it, a position at
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
    // nothing else — it never touches the core, whose color is the note's
    // own. That also makes it the layer that survives a chord voiced within
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
    /// ([`rings`](Self::rings)): a gap out from whatever ring the node ends
    /// with, ordinarily the octave band whose slice it is continuing. That is
    /// the same padding that separates one indicator from the next, so the mark
    /// reads as that indicator continued; a [`ring_gap`](Self::ring_gap) of 0
    /// closes the stand-off, and the mark meets its slice.
    ///
    /// 0 turns the marks off, as a radius of 0 turns the core off. Absolute
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
    /// Which shimmer sweeps the melody/bass marks (see [`Pulse`]): the sheet
    /// takes both marks AND the octave slice each one extends. That reach
    /// into the other layer is the mark being one slice in two pieces, so
    /// light crossing one crosses the other — and it is the whole
    /// of where the sheet goes, the octave layer drawing steady everywhere a
    /// mark does not reach. [`Pulse::Bands`] fresh, so the sweep is on out of
    /// the box; a blob with no `pulse_marks` key gets that same value, the
    /// container-level `#[serde(default)]` making `impl Default` the one
    /// fallback for this field as for every other.
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

    // ---- Home grid -------------------------------------------------------
    // The structural grid between node positions (see `derive_grid`), and with
    // no idle marker under it, the whole of what an unplayed lattice draws.
    // Line width and inset are its settings HERE; its colour is
    // [`lattice_ground`](Self::lattice_ground), up with the note's own layers,
    // because that one grey is the whole at-rest picture — this grid and both
    // of a node's rings where nothing is lit — and a bar that moved only the
    // lines would take the lattice apart. A lit segment is the same ground,
    // arrived rather than brighter.
    /// Grid line thickness as a multiple of the built-in width. 1 is the
    /// classic hairline; the shader scales its grid half-width by this, so 0
    /// takes the lines away and with them the lattice's resting picture.
    pub grid_thickness: f32,
    /// How far a grid segment stops short of each node center, as a factor
    /// of the node radius — the gap a line leaves around the position it
    /// runs to. 0 runs the lines right into the centers; 1.05 sits slightly
    /// wider than the disc's visual radius, so the gap fully contains a
    /// sounding note's circle.
    ///
    /// The gap is what the LINES leave around a node position: they say a node
    /// is there by stopping short of it. What stands in the gap is whatever
    /// the node draws at rest — nothing at all with the audio ring off, and
    /// the ring's own annulus with it on, which is where a fresh view starts
    /// (the inset sits well inside the ring, so the two do not fight).
    pub grid_inset: f32,
    // ---- Trail (where the music has already been) ------------------------
    // The one part of the view about the past rather than the present, and it
    // is drawn in TYPE alone -- see the `trail` module for why that is the
    // whole design and not an implementation detail.
    /// Seconds before a pitch is forgotten, measured from when it last
    /// sounded. **0 means never** -- the default, and the point of the
    /// feature: a whole piece's territory rather than a rolling window.
    pub trail_memory: f32,
    /// Keep the note name and cents on a visited node, not just on sounding
    /// and hovered ones -- so the harmonic space can be read off the screen
    /// by name, with its tuning.
    ///
    /// The trail's on/off, the names being the whole of what a memory draws:
    /// with this off nothing fills [`NodeInstance::trail`](crate::NodeInstance::trail)
    /// at all.
    pub trail_labels: bool,

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
}

/// Where each layer of a node lands, in quad UV units, read outward from its
/// center — what [`ViewConfig::rings`] turns the four size bars into.
///
/// The bars are WIDTHS (the core's radius being its width, measured from the
/// center it starts at), and a ring's inner edge is wherever the last drawn
/// layer ended plus one [`gap`](Self::gap). That is the whole of what stacking
/// buys: widening one layer slides everything outside it out as far as the quad
/// edge, no bar can be dragged behind its neighbour, and a layer dialled to 0
/// hands its slot AND its gap back to the ones around it instead of leaving a
/// hole.
///
/// The quad edge is where sliding stops and DROPPING starts: a ring is the
/// width its bar reads or it is not drawn, so the first layer the stack can no
/// longer fit whole comes out as an empty pair, and every layer outside it goes
/// the same way. Widen the core far enough and the node loses the band, then the
/// audio ring, and ends as the core alone — rather than wearing two hairlines
/// whose bars read out sizes nothing on screen matches (`stacked`).
///
/// An empty pair is also what makes `outer > inner` the one test for "this ring
/// draws": there is nothing else to ask, and no second flag that could
/// disagree with the geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingStack {
    /// The core disc's radius, from the node's center; 0 = off.
    pub core_radius: f32,
    /// The audio ring's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::spectral_ring_width`]).
    pub audio: (f32, f32),
    /// The octave band's inner and outer radius, or `(0.0, 0.0)` when it is off
    /// (see [`ViewConfig::band_width`]).
    pub band: (f32, f32),
    /// The outer edge of the outermost ring DRAWN — the core's radius where
    /// nothing outside it is on, and 0 on a node with no layers at all.
    ///
    /// What the melody/bass marks stand off, and what a node's gutter is
    /// measured from, so neither has to know which of the rings inside it
    /// happened to be the last one on.
    pub outer: f32,
    /// Where the melody/bass mark strip STARTS: a gap out from
    /// [`outer`](Self::outer), or the node's center when the stack is empty
    /// and there is nothing to stand off.
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
    /// The padding between two drawn layers, and between one octave sector and
    /// the next (see [`ViewConfig::ring_gap`]).
    pub gap: f32,
}

impl RingStack {
    /// The four boundaries the stack is laid out on, read outward: where the
    /// audio ring's slot begins, where the octave band's begins, where the
    /// melody/bass strip's begins, and where that strip ENDS.
    ///
    /// Each is the outer limit of the layer INSIDE it — the first three a
    /// [`gap`](Self::gap) past where that layer stopped, the last one flush,
    /// there being no layer after the marks to stand off. So the four run
    /// core, audio ring, band, marks: one boundary per layer, and moving one
    /// is that layer's own width changing.
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
        let after_audio =
            if self.audio.1 > self.audio.0 { self.audio.1 } else { self.core_radius };
        [
            slot_start(self.core_radius, self.gap),
            slot_start(after_audio, self.gap),
            self.mark_inner,
            self.mark_inner + self.mark_thickness,
        ]
    }
}

/// The stack a node's layers are handed out of, innermost first: a cursor at
/// the outer edge of the last layer DRAWN, and whether the room has run out.
///
/// A ring draws at exactly the width its bar reads or it is not drawn at all,
/// which is why a slot that does not fit is REFUSED rather than clipped to the
/// room left. Clipping keeps the layer alive at whatever width the stack
/// happened to leave it — a bar reading 0.19 drawing 0.0008, a hairline at the
/// node's rim that no setting asked for and nothing on screen explains.
struct Stack {
    cursor: f32,
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
    /// stack coming apart in no order at all: widen the core past the audio
    /// ring's slot and the BAND takes it, so a band that had been gone for a
    /// quarter of the bar's travel reappears, and the ring's own width bar
    /// picks up a second off position at the TOP of its travel, where dragging
    /// it wider is what removes it.
    full: bool,
}

impl Stack {
    /// The next layer's slot: `width` thick, a `gap` out from the cursor,
    /// which it then advances to its own outer edge.
    ///
    /// The gap is skipped at a cursor of 0, where there is nothing to stand
    /// off: with the core off the innermost ring reaches the node's center and
    /// its sectors close into pie wedges, rather than opening a hole the size
    /// of a padding around nothing.
    fn take(&mut self, gap: f32, width: f32) -> (f32, f32) {
        if width <= 0.0 || self.full {
            return (0.0, 0.0);
        }
        let inner = slot_start(self.cursor, gap);
        let outer = inner + width;
        if outer > 1.0 {
            self.full = true;
            return (0.0, 0.0);
        }
        self.cursor = outer;
        (inner, outer)
    }
}

/// Where the next layer out begins, given the outer edge of the last one
/// DRAWN: a `gap` past it, or the node's own center when nothing has been
/// drawn yet.
///
/// The second clause is the whole of why this is a function rather than a sum.
/// With the core off the innermost ring reaches the center and its sectors
/// close into pie wedges, rather than opening a hole the size of a padding
/// around nothing — and three places have to agree about that: [`Stack::take`]
/// handing out a slot, [`ViewConfig::rings`] placing the mark strip, and
/// [`RingStack::edges`] saying where a layer that is switched OFF would have
/// started. A second copy of the rule is how a handle comes to sit a gap off a
/// ring that is not there.
fn slot_start(cursor: f32, gap: f32) -> f32 {
    if cursor > 0.0 {
        cursor + gap
    } else {
        0.0
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
    /// One predicate because three places have to agree on it — the pane
    /// grays the marks' own controls, `derive_scene` folds
    /// [`pulse_marks`](Self::pulse_marks) off, and the Shimmer bars gray on
    /// the same thing, that pattern being the only thing they size — and three
    /// copies of a two-term condition is how they come to disagree. They did:
    /// the fold tested the thickness alone, so a saved view with both marks off
    /// kept shipping a live pulse mode, and the Shimmer bars tested neither and
    /// stayed draggable with nothing to drag.
    ///
    /// Says nothing about whether a mark is drawn NOW — that is a held note's
    /// business, per node. This is whether the layer is switched on.
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
        let core = size(self.core_radius, CORE_RADIUS_MAX);
        // The cursor is the outer edge of the last layer DRAWN, which is what
        // makes a ring dialled to 0 cost its gap as well as its slot: nothing
        // moved the cursor, so the next ring starts where it would have.
        let mut stack = Stack { cursor: core, full: false };
        let audio = stack.take(gap, size(self.spectral_ring_width, RING_WIDTH_MAX));
        let band = stack.take(gap, size(self.band_width, RING_WIDTH_MAX));
        RingStack {
            core_radius: core,
            audio,
            band,
            outer: stack.cursor,
            // The strip's own slot, on the stack's terms — a gap out from the
            // last layer drawn, or the node's center when nothing was. Only
            // the outer edge is left to the renderer, because that is the one
            // the billboard's margin lets run past the quad.
            mark_inner: slot_start(stack.cursor, gap),
            mark_thickness: size(self.mark_thickness, MARK_THICKNESS_MAX),
            gap,
        }
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

        // The gutter and its fade, which are ONE control (the Note section's
        // Clearance bar) over two numbers: the fade is a distance measured back
        // from the reach, so a fade wider than the reach is a low end off the
        // bottom of the axis. It draws as a fade over the whole reach either
        // way — the shader floors it at the node's rim, which is the one part
        // that must stay cleared — so holding it there costs the picture
        // nothing and keeps the bar reading out the number the blob holds.
        self.sevens_gutter = finite_or(self.sevens_gutter, fresh.sevens_gutter).clamp(0.0, 0.5);
        self.sevens_gutter_soft = finite_or(self.sevens_gutter_soft, fresh.sevens_gutter_soft)
            .clamp(0.0, self.sevens_gutter);

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

        self.shimmer_speed = finite_or(self.shimmer_speed, fresh.shimmer_speed);
        self.shimmer_width = finite_or(self.shimmer_width, fresh.shimmer_width);
        self.shimmer_intensity = finite_or(self.shimmer_intensity, fresh.shimmer_intensity);
        self.shimmer_softness = finite_or(self.shimmer_softness, fresh.shimmer_softness);
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

/// The `L*` a fresh [`ViewConfig::lattice_ground`] opens on, named because
/// [`ViewConfig::lattice_ground_lightness`] needs it without building a whole
/// fresh view to read one field off. Named, and not a second value: the
/// `Default` below is written in terms of it, the way it is written in terms of
/// `octaves::DEFAULT_COUNT`.
///
/// Which grey this is, and why that rung, is at
/// [`skin::surface_faint_color`](crate::skin::surface_faint_color).
const DEFAULT_RING_GROUND: f32 = 20.0;

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
            // Off, so the grid runs unbroken under a sounding node. A fresh
            // view is flat (extent_sevens 0), and there the clearing has no
            // sheet behind it to hide — it only cuts the grid, which is a look
            // to reach for rather than one to open on. Opening depth is the
            // cue to turn it on, the same move that brings sevens_size down.
            sevens_gutter: 0.0,
            sevens_gutter_soft: 0.0,
            sevens_label: SevensLabel::Name,
            show_labels: true,
            // A third over the built-in size, which is what the lattice is
            // actually read at: a name has to carry its accidental and its
            // comma mark, and at 1 those are strokes rather than marks. The
            // built-in size stays where it is — it is what the marks and the
            // cents line are proportioned against, and this bar sizes the
            // whole label together.
            label_scale: 1.3,
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
            // A small, soft core inside the octave band, with the band's
            // silent slots ghosted in: the pitch class reads as a compact
            // center and the octaves carry the node's outline. (The band's
            // own width is set below.)
            core_solidity: 0.4,
            core_radius: 0.256_256_76,
            // A narrow octave band, stopping short of the quad edge, with a
            // tight gap everywhere: the octaves read as a ring of distinct
            // marks rather than a solid annulus, and every layer keeps clear
            // space around it. (The backdrop that holds the whole ring's shape
            // behind them is fixed on.)
            //
            // The four sizes below the gap are what put the band at 0.661 and
            // its outer edge at 0.851, which is where it has always sat: the
            // stack is core (0.256), gap, audio ring, gap, band, gap, marks.
            band_width: 0.190_065_75,
            ring_gap: 0.051_732_67,
            // The rung of the chrome's own ladder the rings stand on: `L*` 20.0
            // is the skin's `surface_faint`, a step ABOVE the lattice's panel
            // ground (8.8) and well clear of the well grey (4.7), which beside
            // the panel reads as black rather than as a raised surface. A quiet
            // ring is therefore a faintly raised backdrop that is plainly still
            // a reading — `the_fresh_ground_is_the_skins_faint_surface` holds
            // the number to the skin, so retuning that rung and leaving this
            // behind is a test failure rather than a drift.
            lattice_ground: DEFAULT_RING_GROUND,
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
            spectral_width: 10.0,
            // The whole annulus between the fresh core and the fresh octave
            // band, less a gap at each end: 0.308 to 0.609, the widest of the
            // three rings. It carries the most reading of any of them — one
            // wedge per octave, each a level or a window of spectrum — and it
            // is the layer that most rewards the room, where the band's glyphs
            // say one bit apiece.
            //
            // The gaps are what make it a ring rather than a thick edge on the
            // core: the eye reads the three bands (core, audio, octaves) as
            // separate the moment none of them touches. Both marks are outside
            // the band, so the room above the band is the marks' rather than
            // this ring's.
            spectral_ring_width: 0.301_695_2,
            // A whole tone across a wedge — see the field for why that width
            // and not the octave that makes the ring continuous.
            spectral_ring_range: 200.0,
            // Two fifths of the way up the Level window — on the fresh window
            // (−60 dB to 0) a ring for anything down to 36 dB under a
            // full-scale sine, and on a window pulled in to −35 dB, down to 21.
            // It is a share of the window rather than a dB deliberately, so
            // what it says is "no ring dimmer than this much of the ramp" and
            // moving the window moves the gate with the colours it is judging.
            //
            // Measured over the fresh window's 1025 nodes under the fold:
            // silence rings nothing at all — the picture this arrived for,
            // where ungated every one of them wears the ramp's floor — a lone
            // full-scale sine rings 95 (its own pitch class, and the comma
            // neighbours the analyzer cannot resolve from it), a sawtooth 24 dB
            // down rings 177 and one 12 dB down rings 765. So loud dense
            // material still rings most of the lattice, which is what it is
            // actually doing; quiet material rings its constellation.
            //
            // Permissive at the top end on purpose: hiding a ring that had
            // something to show is the failure a person cannot see, where too
            // many rings is one they can, and the bar is right there. The
            // spectrum reading discriminates far less at any setting, and that
            // is the reading rather than the gate — see the field.
            spectral_ring_gate: 0.4,
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
            mark_thickness: 0.078_269_88,
            // Just past a passing sixteenth — 125ms at 120bpm — which is
            // where the wait stops rejecting notes anyone is listening to and
            // starts rejecting the ones nobody is. Not the 0 the bar's low
            // end offers, because a mark outlives its key (see `mark_delay`):
            // at 0 every momentary crowning fades its way OUT over the whole
            // Fade, so lifting a chord one key at a time leaves a fading mark
            // on nearly every note of it, and opening there would ship the
            // mechanism switched off.
            mark_delay: 0.15,
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
            grid_thickness: 1.103_806_3,
            grid_inset: 0.3,
            // The trail on, and never forgetting: where the music has been is
            // most of what the lattice is for, and the names are quiet enough
            // to accumulate over a whole piece without ever competing with
            // what is sounding.
            trail_memory: 0.0,
            trail_labels: true,
            meantone: false,
            meantone_auto: true,
            marvel: false,
            marvel_auto: true,
            frameless: false,
            show_perf: false,
            show_perf_detail: false,
            render_scale: 1.0,
            // A halo at about four fifths strength: the small soft core and
            // the thin octave marks are quiet shapes, and the bloom is what
            // gives them presence.
            bloom_strength: 0.806_154_85,
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
    /// for EVERY layer of the node: the pitch class core, the octave glyphs,
    /// and the melody/bass marks.
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
    /// the core they sit on, because a ring and the sector it links back to
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
        FrameParams {
            fade_time: 1.0,
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
        }
    }
}
