//! Where each octave indicator sits around a node: how many octaves one turn
//! of the wheel is cut into, which pitch sits at the top of it, and which
//! octaves a node draws.
//!
//! **Every node draws N slices, and they are the same N slices turned.** The
//! wheel is a COUNT of full-size octaves, a number of EXTRAS added small at
//! each end, and a CENTER (the pitch at the top), so a slice is always exactly
//! one octave and — with no extras — always exactly a turn over N. That is the
//! whole of what the settings say; nothing about a node can make one of its
//! slices short.
//!
//! **The center pitch is at the top of every node, whatever its pitch class.**
//! With no extras, one monotone map takes an absolute MIDI pitch to an
//! angle — `center` straight up, rising pitch clockwise, a full turn every N
//! octaves — and each node draws the N octaves of ITSELF nearest the center,
//! laid on that map. So an indicator's angle means an absolute pitch, and two
//! nodes' slices for the same octave NUMBER sit at different angles, exactly
//! as their pitches differ. (Extras make that map per-node — see
//! [`octave_layout`] — and the center pitch is the one point it cannot move.)
//! What differs per node is only WHERE ITS OCTAVES FALL: a node whose class
//! sits `d` semitones from the center's has its whole ring turned by `d` of an
//! octave — left for the half octave below, right for the half octave above,
//! and never further than half a slice either way.
//!
//! **What moves instead is the seam.** The map wraps every N octaves, and the
//! point where a node's lowest slice meets its highest is that wrap. It rests
//! at the bottom for exactly one pitch class — the center's own where N is
//! odd, the tritone from it where N is even (see [`Ring::seam`]) — and turns
//! away from there with the ring for every other, which is the trade this
//! geometry makes — and it is the right way round: a wandering seam costs a
//! discontinuity nobody reads twice, where the alternative (pinning the seam
//! and cutting the two end slices to fit) costs every node a pair of
//! indicators that are the wrong SIZE for the octave they name.
//!
//! **Extras buy reach, not resolution.** The count says how many octaves are
//! drawn at full size; each extra adds one MORE octave to the wheel, at the
//! bottom and at the top together, and takes only a sliver of the turn for it.
//! So raising the count and adding extras both reach further up and down the
//! keyboard, and they differ in what they charge: the count divides the turn
//! among more equals and every octave loses degrees, where extras leave the
//! full-size ones nearly where they were and put the new octaves at the edge
//! of legibility. What that costs is the one property an even wheel has — see
//! [`octave_layout`].
//!
//! **A node draws N octaves whether or not they can sound.** The nearest N to
//! the center are what tile the turn; when the center is far up or down the
//! keyboard some of them are octaves no MIDI note reaches, and those draw as
//! backdrop and never light. Cutting them instead would leave a wedge of the
//! ring missing. Notes past either end of the ring fold onto the outermost
//! slice on their side, so a narrow span is a way of READING the music rather
//! than a filter over it.
//!
//! The widths are computed here, on the CPU, and handed to the shader as one
//! table of cumulative angles that every node shares — the alternative is
//! accumulating the same widths per pixel per sector, which is the same
//! arithmetic done a few million times a frame for a value that changes when a
//! setting does.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Octave indicator slots: MIDI octaves -1..=9, so `slot = octave + 1` and
/// middle C (octave 4, MIDI 60) is slot 5. Slot `s` is the octave whose C is
/// MIDI `12 * s`. Eleven covers the whole MIDI range; the renderer packs one
/// byte per slot into 3 words and asserts the count fits.
///
/// A ring can name slots OUTSIDE this, at the top and bottom of the pitch
/// limits — see [`Ring::base`]. They are octaves no note reaches, so they draw
/// and never light, and nothing indexes the packing by them.
pub const OCTAVE_SLOTS: usize = 11;

/// Slot of middle C's octave: the slot a note at MIDI 60 lights, and the one
/// the default center sits in.
pub const MIDDLE_C_SLOT: usize = 5;

/// Lowest and highest pitch the center can be set to — the MIDI note numbers,
/// so the readout is a note anyone can name.
pub const PITCH_FLOOR: f32 = 0.0;
/// See [`PITCH_FLOOR`].
pub const PITCH_CEIL: f32 = 127.0;

/// Fewest octaves one turn can be cut into, counting the extras. Two
/// half-turn slices is already a picture that says very little, and one would
/// be a single slice covering the whole turn — where a wedge is no longer a
/// wedge and the shader's two-half-plane test has nothing to say.
pub const MIN_SPAN: u32 = 2;

/// Most octaves one turn can be cut into, counting the extras: the eleven MIDI
/// octaves, which is every slot there is and also exactly what the boundary
/// table holds.
pub const MAX_SPAN: u32 = OCTAVE_SLOTS as u32;

/// Fewest FULL-SIZE octaves. One is a wheel that is a single octave and a
/// fringe around it, which is a picture worth being able to ask for; it is
/// only legal with an extra to go either side of it, since on its own it would
/// be that whole-turn slice [`MIN_SPAN`] rules out. [`clamp_wheel`] is where
/// the pair is settled.
pub const MIN_COUNT: u32 = 1;

/// Most extras per end: they come in pairs and the whole wheel fits in
/// [`MAX_SPAN`], so this is what is left over once the count is at its
/// smallest.
pub const MAX_EXTRAS: u32 = (MAX_SPAN - MIN_COUNT) / 2;

/// The count a fresh view starts on: five octaves to the turn, an octave worth
/// 72 degrees, and — centered on middle C — C1..C5 in the DAW's numbering,
/// which is the register a keyboard part actually lives in.
pub const DEFAULT_COUNT: u32 = 5;

/// The pitch a fresh view puts at the top: middle C, MIDI 60, which the UI
/// spells C3 in Bitwig's numbering. The wheel then reads like a keyboard, with
/// the note under the player's hand straight up.
pub const DEFAULT_CENTER: f32 = 60.0;

/// Floor on the extra size. At 0 an extra octave would have no width at all,
/// which is a slice that claims to show a pitch and doesn't; a tenth of an
/// even slice is still a sliver but a visible one — 3 degrees on the fullest
/// wheel, 12 on the smallest that can carry an extra.
pub const MIN_EXTRA_SIZE: f32 = 0.1;

/// Size a fresh extra comes in at, as a fraction of an even slice: small
/// enough to read as fringe at a glance rather than as a slightly short
/// octave, wide enough to see what pitch it is lighting.
pub const DEFAULT_EXTRA_SIZE: f32 = 0.35;

/// Blend a fresh view starts at: none, so every extra is the same size and the
/// wheel is exactly two tiers. The ramp is a thing to reach for, not the
/// resting state — see [`octave_layout`].
pub const DEFAULT_EXTRA_BLEND: f32 = 0.0;

/// Semitones to the octave, as a float: this module is all pitch arithmetic
/// and the conversions read better named.
const SEMIS: f32 = 12.0;

/// Straight up, in the renderer's angles: the bottom of a node is
/// `-FRAC_PI_2`, and clockwise — the direction pitch rises — subtracts.
const UP: f32 = -FRAC_PI_2 - PI;

/// A settable count and extras reduced to a pair the layout can draw. The
/// controls cannot produce anything else, so this is for the values that did
/// not come from them: a hand-edited blob, a project saved by a build whose
/// limits were different, a migration off the old pitch window.
///
/// The count wins where the two compete, because it is the one the picture is
/// about: extras are dropped to whatever is left of [`MAX_SPAN`] rather than
/// the count being cut to make room for them. The exception is the one pair
/// that would draw a single whole-turn slice — a lone octave with no extras —
/// where the count opens to [`MIN_SPAN`] instead.
pub fn clamp_wheel(count: u32, extras: u32) -> (u32, u32) {
    let count = count.clamp(MIN_COUNT, MAX_SPAN);
    let extras = extras.min((MAX_SPAN - count) / 2);
    if count + 2 * extras < MIN_SPAN {
        (MIN_SPAN, extras)
    } else {
        (count, extras)
    }
}

/// The same for the center pitch, non-finite included — a NaN center poisons
/// every angle on the wheel, and `clamp` alone does not catch it since NaN is
/// its own answer.
pub fn clamp_center(center: f32) -> f32 {
    if center.is_finite() {
        center.clamp(PITCH_FLOOR, PITCH_CEIL)
    } else {
        DEFAULT_CENTER
    }
}

/// The pitch axis the octave indicators are drawn on, ready for the shader.
///
/// Computed once per frame from the view settings, not per node: the slice
/// WIDTHS are the same on every node, and all a node contributes is where its
/// own octaves fall against the center (see [`Self::ring`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveLayout {
    /// MIDI pitch at the top of every node's wheel.
    pub center: f32,
    /// Octaves one full turn is cut into, extras included: `count + 2 *
    /// extras`, so a slice is a turn over this while there are no extras and
    /// they are what moves it off that. The whole of the ring geometry —
    /// [`Self::ring`], [`Self::walk`], the shader's own sector test — is in
    /// terms of this and the boundary table, and knows nothing about which
    /// slices are extras.
    pub span: u32,
    /// Full-size octaves: the ones sharing whatever the extras leave, so they
    /// are all the same width and it is the widest one there is.
    pub count: u32,
    /// Extras at EACH end, so the wheel carries `2 * extras` of them and stays
    /// symmetric. Zero is an even axis.
    pub extras: u32,
    /// Angle from a ring's own seam to each of its slice boundaries, walking
    /// CLOCKWISE (the direction pitch rises) and always positive: `bounds[0]`
    /// is 0, the seam itself, and `bounds[span]` is `TAU`, the same seam a
    /// full turn on. Every node reads the same table and subtracts it from its
    /// own seam angle, which is what makes one ring the other turned.
    ///
    /// Entries past `span` repeat the last so a stale index cannot produce a
    /// wild angle.
    pub bounds: [f32; MAX_SPAN as usize + 1],
}

/// Where one node's ring sits: the two numbers that turn the shared widths
/// into that node's own slices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    /// Slot of the ring's LOWEST slice. Slice `i` is slot `base + i`, and the
    /// slots run to `base + span - 1`. Signed, and legitimately outside
    /// `0..OCTAVE_SLOTS` at the extremes of the pitch limits — those octaves
    /// draw and never light.
    pub base: i32,
    /// Angle of the seam at the ring's LOW-pitch end, which is where the walk
    /// through [`OctaveLayout::bounds`] starts.
    ///
    /// It rests at the bottom of the node for exactly one pitch class, and
    /// WHICH one is the span's parity: half a turn is `6 * span` semitones, so
    /// an odd span puts the bottom on a half-octave point — a boundary, which
    /// is what a seam is — for the center's own class, and an even span puts a
    /// whole number of octaves there instead, which lands in the MIDDLE of one
    /// of that class's slices. The class holding the seam at the bottom is
    /// then the tritone from the center's, and the center's own class carries
    /// it half a slice round.
    pub seam: f32,
}

impl Default for OctaveLayout {
    fn default() -> Self {
        octave_layout(DEFAULT_COUNT, DEFAULT_CENTER, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND)
    }
}

/// The pitch axis for `count` full-size octaves centered on `center`, with
/// `extras` more at each end drawn `size` of an even slice and graded by
/// `blend`.
///
/// The wheel is two tiers, and the three knobs say how many of each, how small
/// the small ones are, and whether the step between the tiers is a step at all:
///
/// - The COUNT and the EXTRAS are both a reach, at different exchange rates. A
///   count step adds ONE octave, at whichever end the span's parity puts it
///   (see [`OctaveLayout::ring`]), so reaching one further BOTH ways costs two
///   steps; one extra does that on its own. They are paid for differently too:
///   a count step is paid by every full-size octave equally, since they divide
///   whatever the extras leave, where an extra costs them a fraction of the
///   same reach. Five full-size octaves take 72 degrees each; a pair of extras
///   at the default size reaches an octave further either way and leaves them
///   60.8, where the four count steps that reach those same octaves leave
///   them 40. So the count is the register being read and the extras are how
///   far around it you can still see.
/// - The SIZE is a fraction of an EVEN slice — the turn over the whole span,
///   extras included — and that is what makes it the one number both tiers can
///   be stated in. An extra comes out smaller than a full-size octave exactly
///   when `size < 1`, at every count and every number of extras: the full-size
///   width works out to `(span - 2 * extras * size) / count` even slices, which
///   beats `size` precisely when `span > size * span`. So there is no ceiling
///   that moves under the other two controls, and `size` 1 is an even axis
///   rather than a mode beside one.
/// - The BLEND grades the extras from the outermost inward, a step of
///   `blend * (full - size) / extras` each, so 0 is the flat fringe the size
///   alone describes and 1 is a ramp whose last step into the full-size
///   octaves is the same size as the steps within it. The OUTERMOST extra is
///   `size` whatever the blend, deliberately: that is a size on screen rather
///   than a ratio against another octave, and pinning the relative weight
///   instead would let dragging the blend take degrees away from the slices
///   with the least to give.
///
/// Each knob is inert where it has nothing to say, which is what the UI's
/// enable gates have to agree with. The size and the blend do nothing with no
/// extras, there being no second tier; the blend does nothing at ONE extra per
/// end, there being no pair of extras to differ; and the blend does nothing at
/// `size` 1, where the tiers are already the same width and a ramp between
/// them is a ramp between equals. See
/// [`the_blend_is_inert_below_two_extras`](self).
///
/// The two tiers turn WITH each node's ring rather than staying pinned to the
/// screen, so every node shows the same profile and the extras are always that
/// node's own outermost octaves. The price is paid in the one property an even
/// axis has: with extras, two nodes place the same octave a little apart, by
/// however much their rings are turned, so a pitch is one angle across the
/// lattice only while there are none. The CENTER pitch is the exception, and
/// it is pinned rather than lucky — [`OctaveLayout::ring`] solves for it, so
/// the top of the picture means one pitch at every setting there is.
pub fn octave_layout(count: u32, center: f32, extras: u32, size: f32, blend: f32) -> OctaveLayout {
    let (count, extras) = clamp_wheel(count, extras);
    let center = clamp_center(center);
    // Non-finite in means the default out. `clamp` alone does not catch it —
    // NaN is its own answer, so it passes both comparisons and comes out the
    // far side — and a NaN then reaches every width through the arithmetic
    // below, which draws a wheel with a hole in it and nothing to say why.
    let size = if size.is_finite() { size.clamp(MIN_EXTRA_SIZE, 1.0) } else { DEFAULT_EXTRA_SIZE };
    let blend = if blend.is_finite() { blend.clamp(0.0, 1.0) } else { DEFAULT_EXTRA_BLEND };
    let span = count + 2 * extras;
    let (n, r, p) = (span as f32, count as f32, extras as f32);

    // Widths in EVEN SLICES, so every line here is a ratio and the turn is
    // divided out once at the end.
    //
    // The extras are a ramp of `extras` steps rising inward from `size`, and
    // the full-size octaves take what is left. Which is circular — the ramp
    // climbs toward a width that depends on how much the ramp took — so it is
    // solved rather than iterated: writing the ramp as `size + step * j` with
    // `step = blend * (full - size) / extras`, the widths sum to the span at
    //
    //     full = (span - 2*extras*size + ramp*size) / (count + ramp)
    //
    // with `ramp = blend * (extras - 1)`, the extra weight the grading moves
    // onto the full-size octaves' side of the books. At `blend` 0, or at one
    // extra per end where a ramp has nowhere to rise, `ramp` is 0 and this is
    // just "the extras take `2*extras*size` and the rest is shared".
    let ramp = blend * (p - 1.0).max(0.0);
    let full = (n - 2.0 * p * size + ramp * size) / (r + ramp);
    let step = if extras > 0 { blend * (full - size) / p } else { 0.0 };
    // Slice `i` counting up from the ring's low end: the first `extras` are
    // the bottom fringe, outermost first, then the full-size octaves, then the
    // top fringe mirrored so the wheel is symmetric about its middle.
    let width = |i: usize| {
        let i = i as u32;
        if i < extras {
            size + step * i as f32
        } else if i >= extras + count {
            size + step * (span - 1 - i) as f32
        } else {
            full
        }
    };
    // Normalized by what the widths ACTUALLY come to rather than by the span
    // the algebra says they come to, so the ring closes exactly however the
    // float arithmetic above rounded.
    let total = (0..span as usize).map(width).sum::<f32>().max(1e-6);
    let mut bounds = [TAU; MAX_SPAN as usize + 1];
    let mut acc = 0.0;
    for i in 0..span as usize {
        acc += width(i);
        bounds[i + 1] = TAU * acc / total;
    }
    bounds[0] = 0.0;

    OctaveLayout { center, span, count, extras, bounds }
}

impl OctaveLayout {
    /// Where the ring of a node whose pitch class is `cents` (0..1200) sits.
    ///
    /// The node draws the `span` octaves of its own class NEAREST the center
    /// pitch, which is what keeps every node's ring over the same stretch of
    /// keyboard however its class falls. With an odd span that set is
    /// symmetric about the node's nearest octave; with an even one it reaches
    /// one octave further on the side of that octave the CENTER itself sits —
    /// a center just under one of the node's octaves reaches down, one just
    /// over it reaches up — and a center landing exactly on one of them breaks
    /// the tie downward.
    pub fn ring(&self, cents: f32) -> Ring {
        let off = cents / 100.0;
        // The node's octave nearest the center, and how far above the center
        // that octave sits — the distance that turns the ring. Halves round
        // up, so a node exactly a tritone from the center counts as the half
        // octave ABOVE it; the two readings draw the same slices anyway, one
        // octave apart in what they are named.
        let nearest = ((self.center - off) / SEMIS + 0.5).floor();
        let d = nearest * SEMIS + off - self.center;
        let span = self.span as i32;
        let low = if span % 2 == 1 {
            -(span - 1) / 2
        } else if d < 0.0 {
            1 - span / 2
        } else {
            -span / 2
        };
        // Turned so the CENTER pitch lands straight up — which is the whole of
        // where a ring sits, and is why this is solved for rather than derived
        // from the ring's middle. The two agree while the axis is even; with
        // extras they part, because the slice the center falls in is not one
        // span-th of the turn and the pitch sits at its own fraction of
        // whatever width that slice has.
        let base = nearest as i32 + low;
        let along = (self.center - self.slot_pitch(base, cents)) / SEMIS + 0.5;
        Ring { base, seam: UP + self.walk(along) }
    }

    /// Angle from a ring's seam to `x` slices along it, walking clockwise.
    /// Linear inside a slice, so a pitch stands at the same fraction of its
    /// own octave's wedge as it does of the octave.
    fn walk(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, self.span as f32);
        let i = (x.floor() as usize).min(self.span as usize - 1);
        let t = x - i as f32;
        self.bounds[i] + (self.bounds[i + 1] - self.bounds[i]) * t
    }

    /// Angle of boundary `i` of `ring`, counting clockwise from its seam.
    pub fn edge(&self, ring: Ring, i: usize) -> f32 {
        ring.seam - self.bounds[i.min(self.span as usize)]
    }

    /// The slots a node whose pitch class is `cents` draws, inclusive: `span`
    /// of them, always. Signed — see [`Ring::base`].
    pub fn slots(&self, cents: f32) -> (i32, i32) {
        let base = self.ring(cents).base;
        (base, base + self.span as i32 - 1)
    }

    /// MIDI pitch of octave slot `slot` on a node whose pitch class is
    /// `cents` (0..1200): the pitch that indicator stands for, and the pitch
    /// it is centered on.
    pub fn slot_pitch(&self, slot: i32, cents: f32) -> f32 {
        slot as f32 * SEMIS + cents / 100.0
    }

    /// The two edge angles of slot `slot`'s indicator on a node whose pitch
    /// class is `cents` — the counter-clockwise one first. The shader's
    /// `oct_sector` is this.
    ///
    /// Exactly one octave wide, at every slot and every node: the ring holds
    /// whole octaves of the node's own class and nothing is cut to fit, so the
    /// indicators meet edge to edge and close the turn without any of them
    /// standing for less pitch than it claims.
    pub fn sector(&self, slot: i32, cents: f32) -> (f32, f32) {
        let ring = self.ring(cents);
        let i = (slot - ring.base).clamp(0, self.span as i32 - 1) as usize;
        (self.edge(ring, i), self.edge(ring, i + 1))
    }

    /// Where MIDI pitch `pitch` sits on the wheel of a node whose pitch class
    /// is `cents`, in radians. Linear within each slice and monotone across
    /// them, so an interval reads as an angle.
    ///
    /// Per node because the extras are: with an even axis this is one shared map
    /// and every node agrees on it (see [`octave_layout`]). Outside the ring
    /// it CLAMPS, at either end — an indicator never reaches past the seam,
    /// and continuing round instead would land at the wrong pitch, since one
    /// turn comes back to a pitch a whole span of octaves away.
    pub fn angle(&self, pitch: f32, cents: f32) -> f32 {
        let ring = self.ring(cents);
        // In slices from the ring's low edge, which is half an octave under
        // its lowest slot's own pitch.
        let along = (pitch - self.slot_pitch(ring.base, cents)) / SEMIS + 0.5;
        ring.seam - self.walk(along)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blend bar at both ends and its middle: a flat fringe, a half-graded
    /// one, and the full ramp.
    const BLENDS: [f32; 3] = [0.0, 0.5, 1.0];

    /// The size bar at both ends and in between: the thinnest sliver, the
    /// default fringe, and the setting where an extra IS a full-size octave.
    const SIZES: [f32; 3] = [MIN_EXTRA_SIZE, DEFAULT_EXTRA_SIZE, 1.0];

    /// Wheels to run the invariants over, as `(count, extras, center)`: the
    /// default, the narrowest wheel there is, the widest reached two ways (all
    /// count, and the fewest full-size octaves a wheel can have — the
    /// narrowest has only the one, since `1 + 2 * extras` is never 2), even
    /// counts (where the drawn octaves cannot be symmetric about the center),
    /// and centers that are deliberately NOT a C and not near the middle of
    /// the keyboard — the last three are where a ring reaches for octaves no
    /// MIDI note can play.
    const WHEELS: [(u32, u32, f32); 11] = [
        (DEFAULT_COUNT, 0, DEFAULT_CENTER),
        (DEFAULT_COUNT, 2, DEFAULT_CENTER),
        (MIN_SPAN, 0, DEFAULT_CENTER),
        (MAX_SPAN, 0, DEFAULT_CENTER),
        (MIN_COUNT, 1, DEFAULT_CENTER),
        (MIN_COUNT, MAX_EXTRAS, DEFAULT_CENTER),
        (4, 0, DEFAULT_CENTER),
        (4, 2, 54.0),
        (5, 3, 67.0),
        (3, 0, PITCH_FLOOR),
        (3, 4, PITCH_CEIL),
    ];

    /// Pitch classes that put a node's octaves exactly on the center (C), well
    /// clear of it either way, a tritone from it, and just short of a whole
    /// octave.
    const CLASSES: [f32; 5] = [0.0, 350.0, 600.0, 700.0, 1150.0];

    /// Every wheel, size, blend and pitch class — the grid the invariants
    /// below all run over.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        WHEELS.iter().flat_map(|&(count, extras, center)| {
            SIZES.iter().flat_map(move |&size| {
                BLENDS.iter().flat_map(move |&blend| {
                    CLASSES.iter().map(move |&cents| {
                        (
                            octave_layout(count, center, extras, size, blend),
                            cents,
                            format!(
                                "{count}+2x{extras} at {center}, size {size} blend \
                                 {blend}, {cents}c"
                            ),
                        )
                    })
                })
            })
        })
    }

    /// Width of slice `i`, in radians.
    fn width(l: &OctaveLayout, i: usize) -> f32 {
        l.bounds[i + 1] - l.bounds[i]
    }

    /// The whole of what the settings promise with no extras: every slice on
    /// every node is one span-th of the turn. Nothing about a node's pitch
    /// class, and nothing about where the center falls, can shorten one.
    #[test]
    fn an_even_axis_gives_every_node_even_slices() {
        for &(count, _, center) in &WHEELS {
            let (count, _) = clamp_wheel(count, 0);
            let l = octave_layout(count, center, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
            let even = TAU / count as f32;
            for cents in CLASSES {
                let (low, high) = l.slots(cents);
                let case = format!("{count} at {center}, {cents}c");
                assert_eq!(high - low + 1, count as i32, "{case}: not {count} slices");
                for slot in low..=high {
                    let (e0, e1) = l.sector(slot, cents);
                    assert!(
                        (e0 - e1 - even).abs() < 1e-4,
                        "{case}: slot {slot} spans {} of the {even} an octave is worth",
                        e0 - e1
                    );
                }
            }
        }
    }

    /// The size and the blend are about the extras and nothing else, so with
    /// none of them the wheel is even at every setting of both — which is what
    /// makes zero extras "off" rather than a mode beside it.
    #[test]
    fn the_size_and_blend_do_nothing_without_extras() {
        let even = octave_layout(7, 60.0, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        for size in SIZES {
            for blend in BLENDS {
                let l = octave_layout(7, 60.0, 0, size, blend);
                let case = format!("size {size} blend {blend}");
                for i in 0..=7 {
                    assert!((l.bounds[i] - even.bounds[i]).abs() < 1e-5, "{case}: moved");
                }
            }
        }
    }

    /// The center pitch is straight up on EVERY node, whatever its pitch
    /// class — which is the thing a per-node rotation of the wheel is for, and
    /// what makes the top of the picture mean one pitch across the lattice.
    #[test]
    fn the_center_pitch_is_straight_up_on_every_node() {
        for (l, cents, case) in every_case() {
            let up = l.angle(l.center, cents);
            assert!((up - UP).abs() < 1e-4, "{case}: the center is at {up}, not {UP}");
        }
    }

    /// Which way a node turns: the half octave BELOW the center turns left
    /// (the top of the ring moves counter-clockwise) and the half above turns
    /// right, by the pitch distance and never by more than half a slice.
    #[test]
    fn a_node_turns_toward_its_own_octave() {
        // An even axis, where the turn is the whole of what a pitch class
        // does — extras move the widths too, and then "turned by d" is a
        // statement about the ring's middle rather than about one edge.
        let l = octave_layout(5, 60.0, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        let seam = |cents: f32| l.ring(cents).seam;
        let straight = seam(0.0);
        for (cents, turn) in [(100.0f32, 1.0), (500.0, 5.0), (600.0, 6.0)] {
            // Above the center: clockwise, which subtracts.
            let want = straight - TAU * turn / 60.0;
            assert!((seam(cents) - want).abs() < 1e-4, "{cents}c did not turn right");
        }
        for (cents, turn) in [(1100.0f32, 1.0), (700.0, 5.0)] {
            let want = straight + TAU * turn / 60.0;
            assert!((seam(cents) - want).abs() < 1e-4, "{cents}c did not turn left");
        }
        // Half a slice is the whole of the travel: a tritone is as far as a
        // pitch class can be from the center's own.
        let half = TAU / (2.0 * 5.0);
        for cents in (0..1200).step_by(10) {
            let turn = (seam(cents as f32) - straight).abs();
            assert!(turn <= half + 1e-4, "{cents}c turned {turn}, past half a slice");
        }
    }

    /// The indicators tile the turn: each one's clockwise edge is the next
    /// one's counter-clockwise edge, and the ring closes on every node
    /// whatever its pitch class, count, center, extras, size and blend.
    #[test]
    fn the_indicators_tile_the_turn() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slots(cents);
            let mut total = 0.0;
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 > e1, "{case}: slot {slot} runs backwards");
                total += e0 - e1;
                if slot < high {
                    let next = l.sector(slot + 1, cents).0;
                    assert!((e1 - next).abs() < 1e-4, "{case}: slot {slot} leaves {}", e1 - next);
                }
            }
            assert!((total - TAU).abs() < 1e-4, "{case}: the ring covers {total} rad");
        }
    }

    /// A drawn indicator is on its own pitch, dead center of it, at every
    /// setting — which is what keeps the mapping honest: an indicator moved to
    /// fit would misstate its pitch.
    #[test]
    fn drawn_indicators_sit_on_their_own_pitch() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slots(cents);
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                let inside = l.angle(l.slot_pitch(slot, cents), cents);
                assert!(
                    (inside - 0.5 * (e0 + e1)).abs() < 1e-4,
                    "{case}: slot {slot} is not centered on its pitch"
                );
            }
        }
    }

    /// What "faithful" means, as an assertion: the map is strictly falling in
    /// pitch across the ring, and equal intervals WITHIN one slice subtend
    /// equal angles, so an indicator sits on its pitch rather than near it.
    #[test]
    fn the_axis_is_monotone_and_linear_inside_a_slice() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slots(cents);
            let bottom = l.slot_pitch(low, cents) - 0.5 * SEMIS;
            let top = l.slot_pitch(high, cents) + 0.5 * SEMIS;
            let mut previous = l.angle(bottom, cents);
            let mut pitch = bottom;
            while pitch < top - 0.25 {
                pitch += 0.5;
                let a = l.angle(pitch, cents);
                assert!(a < previous, "{case}: not falling at {pitch}");
                previous = a;
            }
            // Three points evenly spaced in pitch inside one slice come out
            // evenly spaced in angle. Inside the LOWEST slice, which is an
            // extra wherever the wheel has any.
            let base = l.slot_pitch(low, cents) - 4.0;
            let (a, b, c) =
                (l.angle(base, cents), l.angle(base + 4.0, cents), l.angle(base + 8.0, cents));
            assert!(((a - b) - (b - c)).abs() < 1e-4, "{case}: uneven inside a slice");
        }
    }

    /// With no extras the axis is SHARED: one pitch is one angle on every
    /// node, which is what makes an indicator's position mean an absolute
    /// pitch rather than a place in that node's own private ring.
    #[test]
    fn an_even_axis_puts_a_pitch_at_the_same_angle_on_every_node() {
        for &(count, _, center) in &WHEELS {
            let (count, _) = clamp_wheel(count, 0);
            let l = octave_layout(count, center, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
            for step in -6..=6 {
                let pitch = center + step as f32 * 2.5;
                let mut want: Option<f32> = None;
                for cents in CLASSES {
                    // Only where the pitch is actually on this node's ring:
                    // past either end the angle clamps, and a node whose ring
                    // stops short of the pitch legitimately says so.
                    let (low, high) = l.slots(cents);
                    let bottom = l.slot_pitch(low, cents) - 0.5 * SEMIS;
                    let top = l.slot_pitch(high, cents) + 0.5 * SEMIS;
                    if pitch <= bottom + 1e-3 || pitch >= top - 1e-3 {
                        continue;
                    }
                    let a = l.angle(pitch, cents);
                    match want {
                        None => want = Some(a),
                        Some(w) => assert!(
                            (a - w).abs() < 1e-4,
                            "{count} at {center}: pitch {pitch} is {a} on \
                             {cents}c and {w} elsewhere"
                        ),
                    }
                }
            }
        }
    }

    /// The promise the whole design rests on, and the reason the size is a
    /// fraction of an even slice rather than an angle: an extra is never
    /// wider than a full-size octave, at every count, every number of extras,
    /// every size under 1 and every blend. There is no ceiling on the size bar
    /// that moves as the other two controls do.
    #[test]
    fn an_extra_is_never_wider_than_a_full_size_octave() {
        for &(count, extras, center) in &WHEELS {
            for size in SIZES {
                for blend in BLENDS {
                    let l = octave_layout(count, center, extras, size, blend);
                    if l.extras == 0 {
                        continue;
                    }
                    let case = format!("{count}+2x{extras}, size {size} blend {blend}");
                    let full = width(&l, l.extras as usize);
                    for i in 0..l.extras as usize {
                        let extra = width(&l, i);
                        // Equal is the honest answer at size 1, where the two
                        // tiers ARE the same width and the wheel is even.
                        let slack = if size < 1.0 { -1e-5 } else { 1e-5 };
                        assert!(
                            extra - full <= slack,
                            "{case}: extra {i} is {extra} rad against a full {full}"
                        );
                    }
                }
            }
        }
    }

    /// The two tiers, as the picture describes them: the full-size octaves are
    /// all one width, the extras rise inward toward them, and the whole
    /// profile is symmetric about the middle of the ring.
    #[test]
    fn the_wheel_is_two_symmetric_tiers() {
        for &(count, extras, center) in &WHEELS {
            for size in SIZES {
                for blend in BLENDS {
                    let l = octave_layout(count, center, extras, size, blend);
                    let case = format!("{count}+2x{extras}, size {size} blend {blend}");
                    let span = l.span as usize;
                    let first = l.extras as usize;
                    for i in first..first + l.count as usize {
                        assert!(
                            (width(&l, i) - width(&l, first)).abs() < 1e-5,
                            "{case}: full-size octave {i} is not the width of the first"
                        );
                    }
                    // Never narrowing on the way in from the low end, which is
                    // the whole of "the fringe is the outside of the wheel".
                    for i in 1..first + 1 {
                        assert!(
                            width(&l, i) - width(&l, i - 1) > -1e-5,
                            "{case}: slice {i} is narrower than the one outside it"
                        );
                    }
                    for i in 0..span {
                        assert!(
                            (width(&l, i) - width(&l, span - 1 - i)).abs() < 1e-5,
                            "{case}: lopsided at slice {i}"
                        );
                    }
                }
            }
        }
    }

    /// The size alone sets the OUTERMOST extra, and the blend leaves it
    /// exactly where it is: that slice is `size` of an even one at every
    /// blend, count and number of extras. Dragging the blend across its whole
    /// travel must not cost the outermost octaves a degree — they are the ones
    /// with the least to give.
    #[test]
    fn the_size_alone_sets_the_outermost_extra() {
        for (count, extras) in [(1u32, 1u32), (3, 1), (3, 4), (5, 3), (9, 1)] {
            for size in SIZES {
                let even = TAU / (count + 2 * extras) as f32;
                for blend in BLENDS {
                    let l = octave_layout(count, 60.0, extras, size, blend);
                    let case = format!("{count}+2x{extras}, size {size} blend {blend}");
                    let edge = width(&l, 0);
                    assert!(
                        (edge - size * even).abs() < 1e-4,
                        "{case}: the outermost extra is {edge} rad, not {}",
                        size * even
                    );
                }
            }
        }
    }

    /// What the blend DOES move, given it cannot touch the outermost extra:
    /// the ones inside it. Every step right lifts them toward the full-size
    /// octaves — which the full-size ones pay for — and at the top of the bar
    /// the last step into them is the same as the steps within the fringe, so
    /// the profile is one ramp rather than a ramp and then a jump.
    #[test]
    fn the_blend_grades_the_extras() {
        let l = |blend: f32| octave_layout(3, 60.0, 4, MIN_EXTRA_SIZE, blend);
        // Flat: four extras a side, every one the same sliver.
        let flat = l(0.0);
        for i in 0..4 {
            assert!((width(&flat, i) - width(&flat, 0)).abs() < 1e-5, "blend 0 is not flat");
        }
        let mut previous: Option<(f32, f32)> = None;
        for blend in BLENDS {
            let l = l(blend);
            let (inner, full) = (width(&l, 3), width(&l, 4));
            if let Some((was_inner, was_full)) = previous {
                assert!(inner > was_inner, "blend {blend}: the inner extra did not widen");
                assert!(full < was_full, "blend {blend}: the full-size octaves did not pay");
            }
            previous = Some((inner, full));
        }
        // The top of the bar is one straight ramp across the whole fringe and
        // on into the full-size octaves.
        let ramp = l(1.0);
        let step = width(&ramp, 1) - width(&ramp, 0);
        for i in 1..=4 {
            let got = width(&ramp, i) - width(&ramp, i - 1);
            assert!((got - step).abs() < 1e-4, "the full ramp steps {got} at {i}, not {step}");
        }
    }

    /// WHERE the blend starts biting, which is the thing the UI's enable gate
    /// has to agree with. A ramp needs two extras to rise between: with one
    /// per end there is nothing between the outermost extra (pinned by the
    /// size) and the full-size octaves, so the bar has nowhere to put a step.
    /// It is equally inert at size 1, where the tiers are already the same
    /// width.
    #[test]
    fn the_blend_is_inert_below_two_extras() {
        let moved = |count: u32, extras: u32, size: f32| {
            let (flat, ramped) = (
                octave_layout(count, 60.0, extras, size, 0.0),
                octave_layout(count, 60.0, extras, size, 1.0),
            );
            (0..=MAX_SPAN as usize)
                .map(|i| (flat.bounds[i] - ramped.bounds[i]).abs())
                .fold(0.0f32, f32::max)
        };
        for (count, extras, size) in [(5, 0, 0.3), (5, 1, 0.3), (9, 1, 0.3), (5, 3, 1.0)] {
            let got = moved(count, extras, size);
            assert!(got < 1e-5, "{count}+2x{extras} at size {size}: the blend moved {got} rad");
        }
        for (count, extras) in [(1u32, 2u32), (3, 4), (5, 2)] {
            let got = moved(count, extras, 0.3);
            assert!(got > 0.01, "{count}+2x{extras}: the blend moved nothing");
        }
    }

    /// What extras are FOR: each one adds an octave the wheel did not draw,
    /// one at the bottom and one at the top, and charges the full-size
    /// octaves a fraction of what reaching those octaves by raising the count
    /// would have cost.
    #[test]
    fn extras_reach_further_than_the_count_would_for_the_price() {
        let plain = octave_layout(5, 60.0, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        let fringed = octave_layout(5, 60.0, 2, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        let wider = octave_layout(9, 60.0, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        // The same octaves either way: two more at each end.
        assert_eq!(fringed.slots(0.0), wider.slots(0.0), "the two reach different octaves");
        let (low, high) = plain.slots(0.0);
        assert_eq!(fringed.slots(0.0), (low - 2, high + 2), "extras did not reach further");
        // And what each charges the octaves that were already there.
        let (kept, spent) = (width(&fringed, 2), width(&wider, 0));
        assert!(kept > 1.5 * spent, "the fringe cost {kept} against the count's {spent}");
    }

    /// An indicator can pass a half turn but never a whole one, which is
    /// exactly the pair of facts the shader's wedge test is built on: past a
    /// half turn a wedge is the UNION of its two half-planes rather than
    /// their intersection, and at a whole turn neither reading means
    /// anything. The widest there is is the lone full-size octave of a
    /// one-plus-a-pair wheel at the thinnest size, where two extras at a tenth
    /// of an even slice leave the one between them 336 degrees.
    #[test]
    fn an_indicator_can_pass_a_half_turn_but_never_a_whole_one() {
        let mut widest: f32 = 0.0;
        for (l, cents, case) in every_case() {
            let (low, high) = l.slots(cents);
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 - e1 < TAU - 1e-3, "{case}: slot {slot} spans {} rad", e0 - e1);
                widest = widest.max(e0 - e1);
            }
        }
        assert!(widest > PI, "nothing passes a half turn: widest {widest} rad");
        // The extreme, and where it lives: the fewest full-size octaves, the
        // fewest extras to flank them, at the thinnest size. Every blend
        // reaches it, since one extra per end has no ramp to spread.
        let l = octave_layout(MIN_COUNT, 60.0, 1, MIN_EXTRA_SIZE, 0.5);
        let middle = width(&l, 1).to_degrees();
        assert!((middle - 336.0).abs() < 0.5, "the extreme is {middle} deg");
        assert!(
            widest.to_degrees() <= middle + 1e-3,
            "something reaches past the lone octave's {middle} deg"
        );
    }

    /// A ring at the pitch limits reaches for octaves no note can play, and
    /// draws them rather than cutting the turn short — the invariant that
    /// matters there is that it is still `span` slices tiling a turn, which
    /// `the_indicators_tile_the_turn` covers over the same wheels.
    #[test]
    fn a_ring_at_the_limits_keeps_its_span() {
        let l = octave_layout(5, PITCH_CEIL, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        let (low, high) = l.slots(0.0);
        assert_eq!(high - low + 1, 5, "a ring at the ceiling lost a slice");
        assert!(high >= OCTAVE_SLOTS as i32, "nothing reaches past the table at the ceiling");
        let l = octave_layout(5, PITCH_FLOOR, 0, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        let (low, high) = l.slots(1150.0);
        assert_eq!(high - low + 1, 5, "a ring at the floor lost a slice");
        assert!(low < 0, "nothing reaches under the table at the floor");
    }

    /// A count and extras outside the settable pair — a hand-edited blob, a
    /// migration — are brought back inside it rather than producing a layout
    /// the shader's fixed-size tables cannot hold.
    #[test]
    fn a_wheel_outside_the_limits_is_clamped() {
        assert_eq!(clamp_wheel(0, 0), (MIN_SPAN, 0), "a collapsed wheel opens");
        assert_eq!(clamp_wheel(40, 0), (MAX_SPAN, 0), "and the ceiling holds");
        // A lone full-size octave is legal only with a pair to flank it.
        assert_eq!(clamp_wheel(1, 0), (MIN_SPAN, 0), "a whole-turn slice is not drawable");
        assert_eq!(clamp_wheel(1, 1), (1, 1), "a fringed lone octave is");
        // The count wins where the two compete: the extras are what yields.
        assert_eq!(clamp_wheel(MAX_SPAN, 3), (MAX_SPAN, 0), "the extras did not yield");
        assert_eq!(clamp_wheel(5, 99), (5, 3), "the wheel overran the table");
        assert_eq!(clamp_wheel(MIN_COUNT, 99), (MIN_COUNT, MAX_EXTRAS), "and at the extreme");
        assert_eq!(clamp_center(-40.0), PITCH_FLOOR);
        assert_eq!(clamp_center(400.0), PITCH_CEIL);
        assert_eq!(clamp_center(f32::NAN), DEFAULT_CENTER, "nonsense falls back");
        let l = octave_layout(99, f32::INFINITY, 99, DEFAULT_EXTRA_SIZE, DEFAULT_EXTRA_BLEND);
        assert_eq!((l.count, l.extras, l.span, l.center), (MAX_SPAN, 0, MAX_SPAN, DEFAULT_CENTER));
        // Both continuous knobs, because a NaN reaches the widths by two
        // different routes and neither is caught by a `clamp`: the size is
        // every extra's width and the blend is the step between them.
        for (size, blend) in [(f32::NAN, DEFAULT_EXTRA_BLEND), (0.5, f32::NAN)] {
            let l = octave_layout(5, 60.0, 3, size, blend);
            assert!(
                l.bounds.iter().all(|b| b.is_finite() && (0.0..=TAU + 1e-3).contains(b)),
                "size {size} blend {blend} poisoned the widths: {:?}",
                l.bounds
            );
        }
    }

    /// The default wheel is the register a keyboard part lives in, drawn even:
    /// five octaves of 72 degrees each, C1..C5 in the DAW's numbering, with
    /// middle C straight up and no fringe.
    #[test]
    fn the_default_wheel_is_five_even_octaves_around_middle_c() {
        let l = OctaveLayout::default();
        assert_eq!((l.count, l.extras), (DEFAULT_COUNT, 0), "the default wheel has a fringe");
        assert_eq!(l.slots(0.0), (3, 7), "not C1..C5");
        assert_eq!(l.slot_pitch(MIDDLE_C_SLOT as i32, 0.0), 60.0);
        let (e0, e1) = l.sector(MIDDLE_C_SLOT as i32, 0.0);
        assert!((e0 - e1 - TAU / 5.0).abs() < 1e-5, "middle C's slice is not a fifth of the turn");
        assert!((0.5 * (e0 + e1) - UP).abs() < 1e-5, "middle C is not straight up");
    }
}
