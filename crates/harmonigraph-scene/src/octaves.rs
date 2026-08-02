//! Where each octave indicator sits around a node: how many octaves one turn
//! of the wheel is cut into, which pitch sits at the top of it, and which
//! octaves a node draws.
//!
//! **Every node draws N slices, and they are the same N slices turned.** The
//! wheel is a SPAN (how many octaves fill one turn) and a CENTER (the pitch at
//! the top), so a slice is always exactly one octave and — with the taper off
//! — always exactly a turn over N. That is the whole of what the settings say;
//! nothing about a node can make one of its slices short.
//!
//! **The center pitch is at the top of every node, whatever its pitch class.**
//! With the taper off, one monotone map takes an absolute MIDI pitch to an
//! angle — `center` straight up, rising pitch clockwise, a full turn every N
//! octaves — and each node draws the N octaves of ITSELF nearest the center,
//! laid on that map. So an indicator's angle means an absolute pitch, and two
//! nodes' slices for the same octave NUMBER sit at different angles, exactly
//! as their pitches differ. (A taper makes that map per-node — see
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

/// Fewest octaves one turn can be cut into. Two half-turn slices is already a
/// picture that says very little, and one would be a single slice covering the
/// whole turn — where a wedge is no longer a wedge and the shader's
/// two-half-plane test has nothing to say.
pub const MIN_SPAN: u32 = 2;

/// Most octaves one turn can be cut into: the eleven MIDI octaves, which is
/// every slot there is and also exactly what the boundary table holds.
pub const MAX_SPAN: u32 = OCTAVE_SLOTS as u32;

/// The span a fresh view starts on: five octaves to the turn, an octave worth
/// 72 degrees, and — centered on middle C — C1..C5 in the DAW's numbering,
/// which is the register a keyboard part actually lives in.
pub const DEFAULT_SPAN: u32 = 5;

/// The pitch a fresh view puts at the top: middle C, MIDI 60, which the UI
/// spells C3 in Bitwig's numbering. The wheel then reads like a keyboard, with
/// the note under the player's hand straight up.
pub const DEFAULT_CENTER: f32 = 60.0;

/// Ceiling on the taper amount. At 1 the outermost octave would have no
/// width at all, which is a slice that claims to show a pitch and doesn't;
/// 0.9 leaves it a tenth of an even slice, still a sliver but a visible
/// one — 3 degrees at the widest span, 12 at the narrowest that tapers.
pub const MAX_TAPER_AMOUNT: f32 = 0.9;

/// Semitones to the octave, as a float: this module is all pitch arithmetic
/// and the conversions read better named.
const SEMIS: f32 = 12.0;

/// Straight up, in the renderer's angles: the bottom of a node is
/// `-FRAC_PI_2`, and clockwise — the direction pitch rises — subtracts.
const UP: f32 = -FRAC_PI_2 - PI;

/// How far the Shape setting can bend the taper either way, as an exponent:
/// the ends of the bar are `x^(1/4)` and `x^4`, and its middle is the
/// straight ramp `x`. Four is where more travel stops buying a different
/// picture — at the sharp end the octave beside the middle already keeps under
/// a third of the width the middle one takes over an even slice, and at the
/// flat end every octave inside the edges is within a fifth of the middle.
const SHAPE_EXTREME: f32 = 4.0;

/// The exponent the distance is raised to, from a Shape setting of 0..1.
/// Logarithmic, so the bar's middle is the straight ramp (exponent 1) and
/// the two halves are mirror images of each other rather than one being a
/// squashed version of the other.
///
/// Non-finite in means the straight ramp out. `clamp` alone does not catch it
/// — NaN is its own answer, so it passes both comparisons and comes out the
/// far side — and the NaN then reaches the widths through `powf`, where it
/// spares the edge slices (distance 1 to any power is 1) and poisons every
/// slice inside them. A table like that draws a wheel with a hole in it and
/// nothing to say why.
fn shape_exponent(shape: f32) -> f32 {
    let shape = if shape.is_finite() { shape } else { DEFAULT_TAPER_SHAPE };
    SHAPE_EXTREME.powf(2.0 * shape.clamp(0.0, 1.0) - 1.0)
}

/// Shape the view starts at: the straight ramp, which is the middle of the
/// bar and the one shape that is neither a spotlight nor a plateau. It shows
/// nothing until the Amount leaves 0.
pub const DEFAULT_TAPER_SHAPE: f32 = 0.5;

/// A settable span reduced to one the layout can draw. The Span control cannot
/// produce anything else, so this is for the value that did not come from it:
/// a hand-edited blob, a project saved by a build whose limits were different,
/// a migration off the old pitch window.
pub fn clamp_span(span: u32) -> u32 {
    span.clamp(MIN_SPAN, MAX_SPAN)
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
    /// Octaves one full turn is cut into, so a slice is a turn over this under
    /// an even axis and the taper is what moves it off that.
    pub span: u32,
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
        octave_layout(DEFAULT_SPAN, DEFAULT_CENTER, 0.0, DEFAULT_TAPER_SHAPE)
    }
}

/// The pitch axis for `span` octaves to the turn centered on `center`, tapered
/// by `amount` in the shape `shape`.
///
/// Two knobs that mean two different things, which is the whole reason the
/// taper is a pair of bars and not a list of named curves:
///
/// - The AMOUNT sets the EDGE slices, and nothing else does: they come out
///   `1 - amount` of an even slice at every shape and every span. That is a
///   size on screen rather than a ratio against another octave, which matters
///   because the octave a ratio would be against is the one the shape moves the
///   most — pinning the edge's RELATIVE weight instead let dragging Shape
///   toward the plateau, which widens everything but the middle, take degrees
///   away from the edge slices.
/// - The SHAPE says where the width the edges give up lands, through the
///   exponent `p` (see [`shape_exponent`]). Left of the bar's middle, `p` is
///   under 1 and the fall happens at once: the octave next to the middle gives
///   up most of what the edge one does and the outer ones flatten off, so the
///   middle keeps nearly all of the lift — a spotlight. Right of it `p` is over
///   1 and the fall is held back to the extremes: the octaves either side of
///   the middle stay nearly as wide as it is, a plateau rather than a gradient.
///
/// An even axis is `amount` 0, so it is where the bar starts rather than a mode
/// beside it — and the shape is inert there, which is exactly what "no taper"
/// should mean.
///
/// Each is also inert at the bottom of the span, and at a different span,
/// which is why the two bars are gated separately. The AMOUNT does nothing at
/// a span of 2, where every slice IS an edge slice and there is nowhere for
/// the width they give up to go. The SHAPE does nothing at 4 and under: it can
/// only redistribute the lift between slices at different distances from the
/// middle, and at 3 there is one slice between the edges while at 4 the two of
/// them are equidistant and split it evenly whatever the exponent. See
/// [`the_shape_is_inert_at_four_octaves_and_under`](self).
///
/// The taper turns WITH each node's ring rather than staying pinned to the
/// screen, so every node shows the same shape and the widest slice is always
/// the node's own middle octave. The price is paid in the one property an even
/// axis has: under a taper two nodes place the same octave a little apart, by
/// however much their rings are turned, so a pitch is one angle across the
/// lattice only while the amount is 0. The CENTER pitch is the exception, and
/// it is pinned rather than lucky — [`OctaveLayout::ring`] solves for it, so
/// the top of the picture means one pitch at every setting there is.
pub fn octave_layout(span: u32, center: f32, amount: f32, shape: f32) -> OctaveLayout {
    let span = clamp_span(span);
    let center = clamp_center(center);
    let amount = if amount.is_finite() { amount.clamp(0.0, MAX_TAPER_AMOUNT) } else { 0.0 };
    let n = span as f32;

    // How far each slice sits from the middle of the ring, normalized by the
    // furthest — so the taper's shape is the same picture at every span and
    // only its resolution changes. The edge slices are at distance 1, which is
    // what pins them to `1 - amount` whatever the shape does. With an even
    // span there is no middle SLICE, only a middle boundary, and the two
    // either side of it share the near end between them.
    //
    // What the shape bends is how much each slice keeps on the way out,
    // `fall`, which is 1 at the middle and 0 at the edge whatever the exponent.
    let far = 0.5 * (n - 1.0);
    let p = shape_exponent(shape);
    let mut fall = [0f32; MAX_SPAN as usize];
    let mut total_fall = 0.0;
    for (i, f) in fall.iter_mut().enumerate().take(span as usize) {
        let dist = if far > 0.0 { ((i as f32 - far).abs() / far).min(1.0) } else { 0.0 };
        *f = 1.0 - dist.powf(p);
        total_fall += *f;
    }

    // The lift is what the slices inside the edges share out, and it is not a
    // setting: the widths have to add up to the circle, and that pins it. At a
    // span of 2 every slice is an edge and `total_fall` is 0 — nothing has
    // anything to gain, so the amount has nothing to move and the axis stays
    // even rather than coming out short of a turn.
    let lift = if total_fall > 1e-6 { n * amount / total_fall } else { 0.0 };
    let width = |i: usize| (1.0 - amount) + lift * fall[i];
    let total = (0..span as usize).map(width).sum::<f32>().max(1e-6);
    let mut bounds = [TAU; MAX_SPAN as usize + 1];
    let mut acc = 0.0;
    for i in 0..span as usize {
        acc += width(i);
        bounds[i + 1] = TAU * acc / total;
    }
    bounds[0] = 0.0;

    OctaveLayout { center, span, bounds }
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
        // from the ring's middle. The two agree while the axis is even; under
        // a taper they part, because the slice the center falls in is not one
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
    /// Per node because the taper is: with an even axis this is one shared map
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

    /// The Shape bar at both ends, at its middle, and either side of it: the
    /// sharpest spotlight, the straight ramp, and the flattest plateau.
    const SHAPES: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

    /// Wheels to run the invariants over: the default, the two span limits, an
    /// even span (where the drawn octaves cannot be symmetric about the
    /// center), and centers that are deliberately NOT a C and not near the
    /// middle of the keyboard — the last two are where a ring reaches for
    /// octaves no MIDI note can play.
    const WHEELS: [(u32, f32); 8] = [
        (DEFAULT_SPAN, DEFAULT_CENTER),
        (MIN_SPAN, DEFAULT_CENTER),
        (MAX_SPAN, DEFAULT_CENTER),
        (4, DEFAULT_CENTER),
        (4, 54.0),
        (5, 67.0),
        (3, PITCH_FLOOR),
        (6, PITCH_CEIL),
    ];

    /// Pitch classes that put a node's octaves exactly on the center (C), well
    /// clear of it either way, a tritone from it, and just short of a whole
    /// octave.
    const CLASSES: [f32; 5] = [0.0, 350.0, 600.0, 700.0, 1150.0];

    /// Every wheel, shape, amount and pitch class — the grid the invariants
    /// below all run over.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        WHEELS.iter().flat_map(|&(span, center)| {
            SHAPES.iter().flat_map(move |&shape| {
                [0.0f32, 0.35, 0.9].iter().flat_map(move |&amount| {
                    CLASSES.iter().map(move |&cents| {
                        (
                            octave_layout(span, center, amount, shape),
                            cents,
                            format!(
                                "span {span} at {center}, amount {amount} shape {shape}, {cents}c"
                            ),
                        )
                    })
                })
            })
        })
    }

    /// The whole of what the settings promise with the taper off: every slice
    /// on every node is one span-th of the turn. Nothing about a node's pitch
    /// class, and nothing about where the center falls, can shorten one.
    #[test]
    fn an_even_axis_gives_every_node_even_slices() {
        for &(span, center) in &WHEELS {
            let l = octave_layout(span, center, 0.0, DEFAULT_TAPER_SHAPE);
            let even = TAU / span as f32;
            for cents in CLASSES {
                let (low, high) = l.slots(cents);
                let case = format!("span {span} at {center}, {cents}c");
                assert_eq!(high - low + 1, span as i32, "{case}: not {span} slices");
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
        // does — a taper moves the widths too, and then "turned by d" is a
        // statement about the ring's middle rather than about one edge.
        let l = octave_layout(5, 60.0, 0.0, DEFAULT_TAPER_SHAPE);
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
    /// whatever its pitch class, span, center and taper.
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
    /// taper — which is what keeps the mapping honest: an indicator moved to
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
            // evenly spaced in angle.
            let base = l.slot_pitch(low, cents) - 4.0;
            let (a, b, c) =
                (l.angle(base, cents), l.angle(base + 4.0, cents), l.angle(base + 8.0, cents));
            assert!(((a - b) - (b - c)).abs() < 1e-4, "{case}: uneven inside a slice");
        }
    }

    /// With the taper off the axis is SHARED: one pitch is one angle on every
    /// node, which is what makes an indicator's position mean an absolute
    /// pitch rather than a place in that node's own private ring.
    #[test]
    fn an_even_axis_puts_a_pitch_at_the_same_angle_on_every_node() {
        for &(span, center) in &WHEELS {
            let l = octave_layout(span, center, 0.0, DEFAULT_TAPER_SHAPE);
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
                            "span {span} at {center}: pitch {pitch} is {a} on \
                             {cents}c and {w} elsewhere"
                        ),
                    }
                }
            }
        }
    }

    /// Octaves shrink outward, symmetrically, at every shape — and only when
    /// an amount is asked for: the middle of the ring is what carries the
    /// extra weight.
    #[test]
    fn octaves_shrink_away_from_the_middle() {
        for shape in SHAPES {
            for amount in [0.0f32, 0.6] {
                // An odd span, so there is a middle SLICE for the weight to
                // land on and the widths compare directly either side of it.
                let l = octave_layout(9, 60.0, amount, shape);
                let width = |i: usize| l.bounds[i + 1] - l.bounds[i];
                let span = l.span as usize;
                for i in 0..span / 2 {
                    let (inner, outer) = (width(i + 1), width(i));
                    let case = format!("shape {shape}, amount {amount}, slice {i}");
                    if amount == 0.0 {
                        assert!((inner - outer).abs() < 1e-5, "{case}: an even axis moved");
                    } else {
                        assert!(outer < inner, "{case}: {outer} !< {inner}");
                    }
                    // Symmetric about the middle slice.
                    assert!((width(i) - width(span - 1 - i)).abs() < 1e-5, "{case}: lopsided");
                }
            }
        }
    }

    /// The Amount alone sets the edge slices, and the Shape leaves them
    /// exactly where they are: an edge octave is `1 - amount` of an even
    /// slice at every shape and every span. Dragging Shape across its whole
    /// travel must not cost the outermost octaves a degree — they are the
    /// ones with the least to give.
    #[test]
    fn the_amount_alone_sets_the_edge_slices() {
        for span in [3u32, 4, 5, 9, MAX_SPAN] {
            for amount in [0.0f32, 0.35, MAX_TAPER_AMOUNT] {
                let even = TAU / span as f32;
                for shape in SHAPES {
                    let l = octave_layout(span, 60.0, amount, shape);
                    let edge = l.bounds[1] - l.bounds[0];
                    let case = format!("span {span}, amount {amount}, shape {shape}");
                    assert!(
                        (edge - (1.0 - amount) * even).abs() < 1e-4,
                        "{case}: the edge slice is {edge} rad, not {}",
                        (1.0 - amount) * even
                    );
                }
            }
        }
    }

    /// Two slices is every slice an edge slice, so there is nowhere for the
    /// width an amount takes to go: the axis stays even instead of coming out
    /// short of a turn.
    #[test]
    fn the_narrowest_span_has_nothing_to_taper() {
        for shape in SHAPES {
            let l = octave_layout(MIN_SPAN, 60.0, MAX_TAPER_AMOUNT, shape);
            for (i, want) in [0.0, PI, TAU].iter().enumerate() {
                assert!((l.bounds[i] - want).abs() < 1e-5, "shape {shape}: two slices moved");
            }
        }
    }

    /// What the Shape does move, given it cannot touch the edges: degrees
    /// between the middle octave and the ones out toward them. Every step
    /// right flattens the profile — the middle narrows and the octaves next
    /// to the edges widen — and every step left concentrates it again.
    #[test]
    fn the_shape_flattens_the_profile_between_the_edges() {
        let width = |shape: f32, i: usize| {
            let l = octave_layout(9, 60.0, 0.6, shape);
            (l.bounds[i + 1] - l.bounds[i]).to_degrees()
        };
        // Nine octaves: the middle one is index 4, and index 1 is the widest
        // one the shape can still reach, just inside the pinned edge.
        let mut previous: Option<(f32, f32)> = None;
        for shape in SHAPES {
            let (middle, outer) = (width(shape, 4), width(shape, 1));
            if let Some((was_middle, was_outer)) = previous {
                assert!(middle < was_middle, "shape {shape}: the middle did not narrow");
                assert!(outer > was_outer, "shape {shape}: the outer one did not widen");
            }
            previous = Some((middle, outer));
        }
        // The bar's middle is the straight ramp: equal steps octave to octave.
        let ramp: Vec<f32> = (0..4).map(|i| width(0.5, i + 1) - width(0.5, i)).collect();
        for step in &ramp {
            assert!((step - ramp[0]).abs() < 1e-3, "the bar's middle is not a straight ramp");
        }
    }

    /// WHERE the shape starts biting, which is the thing the UI's enable gate
    /// has to agree with. The exponent only has somewhere to land once two
    /// slices inside the edges sit at DIFFERENT distances from the middle: at
    /// four octaves the two of them are equidistant, so they split the lift
    /// evenly whatever the curve, exactly as the single one at three octaves
    /// takes all of it. Five is the first span the bar can move.
    #[test]
    fn the_shape_is_inert_at_four_octaves_and_under() {
        let moved = |span: u32| {
            let (flat, sharp) =
                (octave_layout(span, 60.0, 0.6, 0.0), octave_layout(span, 60.0, 0.6, 1.0));
            (0..=span as usize)
                .map(|i| (flat.bounds[i] - sharp.bounds[i]).abs())
                .fold(0.0f32, f32::max)
        };
        for span in [MIN_SPAN, 3, 4] {
            assert!(moved(span) < 1e-5, "span {span}: the shape moved {} rad", moved(span));
        }
        for span in [5u32, 9, MAX_SPAN] {
            assert!(moved(span) > 0.01, "span {span}: the shape moved nothing");
        }
    }

    /// An indicator can pass a half turn but never a whole one, which is
    /// exactly the pair of facts the shader's wedge test is built on: past a
    /// half turn a wedge is the UNION of its two half-planes rather than
    /// their intersection, and at a whole turn neither reading means
    /// anything. The widest there is is the middle of a THREE-octave wheel at
    /// the fullest amount, where two edge slices at a tenth of an even slice
    /// leave the one between them 336 degrees.
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
        // The extreme, and where it lives: the smallest span that can taper at
        // all, at the fullest amount. Every shape reaches it, since with one
        // slice between the two edges there is no profile left to bend.
        let l = octave_layout(3, 60.0, MAX_TAPER_AMOUNT, 0.5);
        let middle = (l.bounds[2] - l.bounds[1]).to_degrees();
        assert!((middle - 336.0).abs() < 0.5, "the extreme is {middle} deg");
        assert!(
            widest.to_degrees() <= middle + 1e-3,
            "something reaches past the three-octave middle's {middle} deg"
        );
    }

    /// A ring at the pitch limits reaches for octaves no note can play, and
    /// draws them rather than cutting the turn short — the invariant that
    /// matters there is that it is still `span` slices tiling a turn, which
    /// `the_indicators_tile_the_turn` covers over the same wheels.
    #[test]
    fn a_ring_at_the_limits_keeps_its_span() {
        let l = octave_layout(5, PITCH_CEIL, 0.0, DEFAULT_TAPER_SHAPE);
        let (low, high) = l.slots(0.0);
        assert_eq!(high - low + 1, 5, "a ring at the ceiling lost a slice");
        assert!(high >= OCTAVE_SLOTS as i32, "nothing reaches past the table at the ceiling");
        let l = octave_layout(5, PITCH_FLOOR, 0.0, DEFAULT_TAPER_SHAPE);
        let (low, high) = l.slots(1150.0);
        assert_eq!(high - low + 1, 5, "a ring at the floor lost a slice");
        assert!(low < 0, "nothing reaches under the table at the floor");
    }

    /// A span or center outside the settable pair — a hand-edited blob, a
    /// migration — is brought back inside it rather than producing a layout
    /// the shader's fixed-size tables cannot hold.
    #[test]
    fn a_wheel_outside_the_limits_is_clamped() {
        assert_eq!(clamp_span(0), MIN_SPAN, "a collapsed span opens");
        assert_eq!(clamp_span(40), MAX_SPAN, "and the ceiling holds");
        assert_eq!(clamp_center(-40.0), PITCH_FLOOR);
        assert_eq!(clamp_center(400.0), PITCH_CEIL);
        assert_eq!(clamp_center(f32::NAN), DEFAULT_CENTER, "nonsense falls back");
        let l = octave_layout(99, f32::INFINITY, f32::NAN, DEFAULT_TAPER_SHAPE);
        assert_eq!((l.span, l.center), (MAX_SPAN, DEFAULT_CENTER));
        // Both taper knobs, because a NaN reaches the widths by two different
        // routes and neither is caught by a `clamp`: the amount multiplies
        // every width, and the shape is the exponent the fall is raised to.
        // The second is the quieter one — the edge slices survive it (they sit
        // at distance 1, and 1 to any power is 1) so the table comes back with
        // a plausible first entry and NaN inside it.
        for (amount, shape) in [(f32::NAN, DEFAULT_TAPER_SHAPE), (0.5, f32::NAN)] {
            let l = octave_layout(5, 60.0, amount, shape);
            assert!(
                l.bounds.iter().all(|b| b.is_finite() && (0.0..=TAU + 1e-3).contains(b)),
                "amount {amount} shape {shape} poisoned the widths: {:?}",
                l.bounds
            );
        }
    }

    /// The default wheel is the register a keyboard part lives in, drawn even:
    /// five octaves of 72 degrees each, C1..C5 in the DAW's numbering, with
    /// middle C straight up.
    #[test]
    fn the_default_wheel_is_five_even_octaves_around_middle_c() {
        let l = OctaveLayout::default();
        assert_eq!(l.slots(0.0), (3, 7), "not C1..C5");
        assert_eq!(l.slot_pitch(MIDDLE_C_SLOT as i32, 0.0), 60.0);
        let (e0, e1) = l.sector(MIDDLE_C_SLOT as i32, 0.0);
        assert!((e0 - e1 - TAU / 5.0).abs() < 1e-5, "middle C's slice is not a fifth of the turn");
        assert!((0.5 * (e0 + e1) - UP).abs() < 1e-5, "middle C is not straight up");
    }
}
