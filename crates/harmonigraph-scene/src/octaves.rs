//! Where each octave indicator sits around a node: how many octaves one turn
//! of the wheel is cut into, which pitch sits at the top of it, and which
//! octaves a node draws.
//!
//! Every node draws the same number of equal slices, each exactly one octave.
//! The center pitch is at the top of every node; each node's ring turns to
//! place its own octaves on that shared pitch axis. The seam moves with the
//! ring, keeping whole octaves instead of cutting the end slices short.
//! Octaves outside MIDI's reach remain as backdrop. Notes beyond either end
//! fold onto that end's slice.
//!
//! The walk round a ring is the closed form `TAU * x / span`. The renderer
//! repeats it for a slice's width and turns each ring's seam by a per-span
//! table of the same angles for the slices' directions (`OctaveParams::turns`).

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

/// Fewest octaves one turn can be cut into. Two
/// half-turn slices is already a picture that says very little, and one would
/// be a single slice covering the whole turn — where a wedge is no longer a
/// wedge and the shader's two-half-plane test has nothing to say.
pub const MIN_SPAN: u32 = 2;

/// Most octaves one turn can be cut into: the eleven MIDI
/// octaves, which is every slot there is.
pub const MAX_SPAN: u32 = OCTAVE_SLOTS as u32;

/// The count a fresh view starts on: seven octaves to the turn, an octave worth
/// about 51 degrees, and — centered on middle C — the keyboard's full C0..C6
/// span in the DAW's numbering.
pub const DEFAULT_COUNT: u32 = 7;

/// The pitch a fresh view puts at the top: middle C, MIDI 60, which the UI
/// spells C3 in Bitwig's numbering. The wheel then reads like a keyboard, with
/// the note under the player's hand straight up.
pub const DEFAULT_CENTER: f32 = 60.0;

/// Semitones to the octave, as a float: this module is all pitch arithmetic
/// and the conversions read better named.
const SEMIS: f32 = 12.0;

/// Straight up, in the renderer's angles: the bottom of a node is
/// `-FRAC_PI_2`, and clockwise — the direction pitch rises — subtracts.
const UP: f32 = -FRAC_PI_2 - PI;

/// Clamp the octave count to the renderer's drawable range.
pub fn clamp_count(count: u32) -> u32 {
    count.clamp(MIN_SPAN, MAX_SPAN)
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
    /// Equal octave slices in one full turn.
    pub span: u32,
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
    /// Angle of the seam at the ring's LOW-pitch end, which is where
    /// [`OctaveLayout::walk`] starts.
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
        octave_layout(DEFAULT_COUNT, DEFAULT_CENTER)
    }
}

/// Divide one turn into `count` equal octave slices centered on `center`.
pub fn octave_layout(count: u32, center: f32) -> OctaveLayout {
    OctaveLayout { center: clamp_center(center), span: clamp_count(count) }
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
        // Turn the ring so the center pitch lands straight up.
        let base = nearest as i32 + low;
        let along = (self.center - self.slot_pitch(base, cents)) / SEMIS + 0.5;
        Ring { base, seam: UP + self.walk(along) }
    }

    /// Angle from a ring's seam to `x` slices along it, walking CLOCKWISE
    /// (the direction pitch rises) and always positive: 0 is the seam itself
    /// and `span` is `TAU`, the same seam a full turn on. Every slice is the
    /// same width, so the walk is linear in `x` and a pitch stands at the same
    /// fraction of its own octave's wedge as it does of the octave. The
    /// shader's `oct_walk` is this, in the same operation order.
    pub fn walk(&self, x: f32) -> f32 {
        TAU * x.clamp(0.0, self.span as f32) / self.span as f32
    }

    /// Angle of boundary `i` of `ring`, counting clockwise from its seam.
    pub fn edge(&self, ring: Ring, i: usize) -> f32 {
        ring.seam - self.walk(i as f32)
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
    /// `oct_sector` points its edges in these directions.
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
    /// is `cents`, in radians. Linear in pitch across the whole ring, so an
    /// interval reads as an angle.
    ///
    /// Outside the ring it CLAMPS, at either end — an indicator never reaches past the seam,
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

    /// Centers at and between notes and at the keyboard limits.
    const CENTERS: [f32; 5] = [DEFAULT_CENTER, 54.0, 67.5, PITCH_FLOOR, PITCH_CEIL];

    /// Pitch classes that put a node's octaves exactly on the center (C), well
    /// clear of it either way, a tritone from it, and just short of a whole
    /// octave.
    const CLASSES: [f32; 5] = [0.0, 350.0, 600.0, 700.0, 1150.0];

    /// Every drawable count, center and pitch class.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        (MIN_SPAN..=MAX_SPAN).flat_map(|count| {
            CENTERS.into_iter().flat_map(move |center| {
                CLASSES.into_iter().map(move |cents| {
                    (octave_layout(count, center), cents, format!("{count} at {center}, {cents}c"))
                })
            })
        })
    }

    /// The whole of what the settings promise: every slice on
    /// every node is one span-th of the turn. Nothing about a node's pitch
    /// class, and nothing about where the center falls, can shorten one.
    #[test]
    fn an_even_axis_gives_every_node_even_slices() {
        for (l, cents, case) in every_case() {
            let count = l.span;
            let even = TAU / count as f32;
            let (low, high) = l.slots(cents);
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
        let l = octave_layout(5, 60.0);
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
    /// whatever its pitch class, count and center.
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

    /// What "faithful" means, as an assertion: across the whole ring the
    /// angle is the closed form, one span-th of a turn per octave measured
    /// clockwise from the seam, so an indicator sits on its pitch rather than
    /// near it and equal intervals subtend equal angles.
    #[test]
    fn the_axis_is_a_turn_per_span_octaves() {
        for (l, cents, case) in every_case() {
            let ring = l.ring(cents);
            let (low, high) = l.slots(cents);
            let bottom = l.slot_pitch(low, cents) - 0.5 * SEMIS;
            let top = l.slot_pitch(high, cents) + 0.5 * SEMIS;
            let mut pitch = bottom;
            while pitch <= top {
                let want = ring.seam - TAU * (pitch - bottom) / (SEMIS * l.span as f32);
                let a = l.angle(pitch, cents);
                assert!((a - want).abs() < 1e-4, "{case}: {pitch} is at {a}, not {want}");
                pitch += 0.5;
            }
        }
    }

    /// The axis is SHARED: one pitch is one angle on every
    /// node, which is what makes an indicator's position mean an absolute
    /// pitch rather than a place in that node's own private ring.
    #[test]
    fn an_even_axis_puts_a_pitch_at_the_same_angle_on_every_node() {
        for (l, _, _) in every_case().filter(|(_, cents, _)| *cents == CLASSES[0]) {
            let count = l.span;
            let center = l.center;
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

    /// A ring at the pitch limits reaches for octaves no note can play, and
    /// draws them rather than cutting the turn short — the invariant that
    /// matters there is that it is still `span` slices tiling a turn, which
    /// `the_indicators_tile_the_turn` covers over the same wheels.
    #[test]
    fn a_ring_at_the_limits_keeps_its_span() {
        let l = octave_layout(5, PITCH_CEIL);
        let (low, high) = l.slots(0.0);
        assert_eq!(high - low + 1, 5, "a ring at the ceiling lost a slice");
        assert!(high >= OCTAVE_SLOTS as i32, "nothing reaches past the table at the ceiling");
        let l = octave_layout(5, PITCH_FLOOR);
        let (low, high) = l.slots(1150.0);
        assert_eq!(high - low + 1, 5, "a ring at the floor lost a slice");
        assert!(low < 0, "nothing reaches under the table at the floor");
    }

    /// Invalid saved counts stay inside the slots the octave packing holds.
    #[test]
    fn a_wheel_outside_the_limits_is_clamped() {
        assert_eq!(clamp_count(0), MIN_SPAN);
        assert_eq!(clamp_count(1), MIN_SPAN);
        assert_eq!(clamp_count(40), MAX_SPAN);
        assert_eq!(clamp_center(-40.0), PITCH_FLOOR);
        assert_eq!(clamp_center(400.0), PITCH_CEIL);
        assert_eq!(clamp_center(f32::NAN), DEFAULT_CENTER);
        let l = octave_layout(99, f32::INFINITY);
        assert_eq!((l.span, l.center), (MAX_SPAN, DEFAULT_CENTER));
    }
}
