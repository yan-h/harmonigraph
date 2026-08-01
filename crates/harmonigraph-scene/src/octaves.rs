//! Where each octave indicator sits around a node: the pitch axis the wheel
//! is, how wide each octave of it is, and which octaves a node draws.
//!
//! **The wheel is a pitch axis, and it is the same axis on every node.** One
//! monotone map takes an absolute MIDI pitch to an angle: middle C straight
//! up, rising pitch clockwise, one full turn covering the window the Range
//! setting names. An indicator is drawn at the angle of the pitch it stands
//! for — so where a node's indicators sit says which pitches those octaves
//! ARE, and two nodes' indicators for the same octave sit at different
//! angles exactly as their pitches differ.
//!
//! **The Range is a count of indicators, and the window is what holds them.**
//! At ±`span` a node draws the `2 * span + 1` octaves from `span` below
//! middle C's to `span` above — the same octave NUMBERS on every node
//! whatever its pitch class, so ±2 is C1..C5 everywhere. Each is one whole
//! octave of the axis, centered on its own pitch, so the window has to be
//! `2 * span + 1` octaves wide to hold them: half an octave past the
//! outermost indicators' own pitches on either side. They then tile the turn
//! exactly, each meeting its neighbours at the boundary they share.
//!
//! **The bottom is both ends of the window at once, and the axis runs
//! THROUGH it.** The lowest pitch the window shows and the highest land on
//! the same point, straight down, on every node whatever its pitch class —
//! which is the thing a per-node rotation of the wheel cannot give. On a
//! node whose pitch class is not C the highest indicator runs past the high
//! end and comes round to the low one, since that is the same point of the
//! circle. The axis's first and last octaves are the same distance from
//! middle C and so exactly the same width, which is what makes continuing
//! past the seam and wrapping round it the same angle.
//!
//! The map is computed here, on the CPU, and handed to the shader as a table
//! of boundary angles — the alternative is accumulating the same widths per
//! pixel per sector, which is the same arithmetic done a few million times a
//! frame for a value that changes when a setting does.

use std::f32::consts::{FRAC_PI_2, TAU};

/// Octave indicator slots: MIDI octaves -1..=9, so `slot = octave + 1` and
/// middle C (octave 4, MIDI 60) is slot 5. Slot `s` is the octave whose C is
/// MIDI `12 * s`. Eleven covers the whole MIDI range; the renderer packs one
/// byte per slot into 3 words and asserts the count fits.
pub const OCTAVE_SLOTS: usize = 11;

/// Slot of middle C's octave, and the middle of every window: the Range
/// setting counts octaves out from here, so this slot's indicator is drawn on
/// every node at every setting.
pub const MIDDLE_C_SLOT: usize = 5;

/// Narrowest Range: 2 octaves either side of middle C's, so 5 indicators.
pub const MIN_OCTAVE_SPAN: u32 = 2;
/// Widest Range: 5 either side, so 11 indicators — every octave MIDI has.
pub const MAX_OCTAVE_SPAN: u32 = 5;

/// Indicators the widest Range draws, which is also the octaves its window
/// spans — they are one to one.
const MAX_INDICATORS: usize = 2 * MAX_OCTAVE_SPAN as usize + 1;

// The widest Range has to fit the fixed-size tables it is written into: one
// slot per indicator, and one boundary angle per octave of the window plus
// the closing one. Raising MAX_OCTAVE_SPAN alone would run `octave_layout`
// off the end of `bounds`, a runtime panic in the render path; the renderer's
// own ceiling on OCTAVE_SLOTS is a build error, and this makes the pair fail
// the same way.
const _: () = assert!(MAX_INDICATORS <= OCTAVE_SLOTS);

/// Ceiling on the taper amount. At 1 the outermost octave would have no
/// width at all, which is a window that claims to show a pitch and doesn't;
/// 0.9 leaves it a tenth of the middle, still a sliver but a visible one.
pub const MAX_TAPER_AMOUNT: f32 = 0.9;

/// Semitones to the octave, as a float: this module is all pitch arithmetic
/// and the conversions read better named.
const SEMIS: f32 = 12.0;

/// How much of the circle an octave of the axis gets, as a function of its
/// distance from middle C, so the middle of the window carries more visual
/// weight than its extremes.
///
/// A taper bends the pitch axis without breaking it: the map stays monotone
/// and stays linear WITHIN each octave, so an indicator still sits on its
/// own pitch and still spans exactly its own octave. What changes is how
/// many degrees an octave is worth, which is a statement about emphasis
/// rather than about pitch.
///
/// Each TAPERING formula is a function of ONE normalized distance (0 at
/// middle C, 1 at the edge of the window) and ONE amount, and all three
/// agree at both ends: full width at middle C, `1 - amount` at the edge. So
/// the amount always means the same thing — how much of its width the
/// outermost octave gives up — and the three differ only in how that loss is
/// distributed in between, which is what makes flipping between them at a
/// fixed amount a comparison of shapes. [`Uniform`] is the baseline they
/// depart from and is outside that: it ignores the amount.
///
/// [`Uniform`]: OctaveTaper::Uniform
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OctaveTaper {
    /// Equal octaves; the amount is inert. The plain circular division — a
    /// pitch axis of constant scale, where equal intervals anywhere in the
    /// window subtend equal angles.
    #[default]
    Uniform,
    /// Straight ramp: each octave out is the same absolute amount narrower
    /// than the one inside it.
    Linear,
    /// Constant RATIO per octave: each octave out is the same fraction of
    /// its inner neighbour. Falls away fastest near the middle and flattens
    /// toward the edges, so the outer octaves stay legible while the middle
    /// two or three take most of the circle.
    Geometric,
    /// Quadratic: barely narrows the octaves either side of middle C and
    /// takes almost all of the loss at the extremes. Reads as a PLATEAU of
    /// full-size middle octaves with the ends falling away, rather than as a
    /// gradient, which is the difference from Linear.
    ///
    /// A cosine ease is the obvious shape for this and is the wrong one: any
    /// curve antisymmetric about its own midpoint gives exactly `1 - a/2`
    /// halfway out, which is what Linear gives there too — so at the
    /// narrowest window, where halfway out is most of what there is, the two
    /// would be all but identical and one of the four choices would do
    /// nothing.
    Plateau,
}

impl OctaveTaper {
    /// Relative width of an octave `x` of the way from middle C (0) to the
    /// edge of the window (1). Only the RATIOS between the returned weights
    /// matter — [`octave_layout`] normalizes them onto the circle — so this
    /// is a shape, not a size.
    pub fn weight(self, x: f32, amount: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let amount = amount.clamp(0.0, MAX_TAPER_AMOUNT);
        match self {
            OctaveTaper::Uniform => 1.0,
            OctaveTaper::Linear => 1.0 - amount * x,
            OctaveTaper::Geometric => (1.0 - amount).powf(x),
            OctaveTaper::Plateau => 1.0 - amount * x * x,
        }
    }
}

/// The pitch axis the octave indicators are drawn on, ready for the shader.
///
/// Computed once per frame from the view settings, not per node: every node
/// reads the SAME axis, which is what makes an indicator's angle mean an
/// absolute pitch rather than a position in that node's own private ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveLayout {
    /// MIDI pitch at the seam approached from the LEFT — the lowest pitch
    /// the window shows. [`high_pitch`](Self::high_pitch) is the same point
    /// on the circle, come round the other way.
    pub low_pitch: f32,
    /// Octaves the window spans: `2 * span + 1`, one per indicator. Its ends
    /// are the F♯s half an octave outside the outermost indicators' Cs, so
    /// each of its octaves is CENTERED on a C and the axis's own octaves are
    /// exactly a C node's indicators.
    pub octaves: u32,
    /// Angle of each octave boundary, in radians, walking CLOCKWISE (the
    /// direction pitch rises) from the seam: `bounds[0]` is the bottom at
    /// `low_pitch`, and `bounds[octaves]` is a full turn on — the same seam,
    /// at the window's high end. Entries past `octaves` repeat the last so a
    /// stale index cannot produce a wild angle.
    pub bounds: [f32; OCTAVE_SLOTS + 1],
}

impl Default for OctaveLayout {
    fn default() -> Self {
        octave_layout(DEFAULT_OCTAVE_SPAN, OctaveTaper::Uniform, 0.0)
    }
}

/// Range the view starts at: 4 octaves either side of middle C's, so 9
/// indicators — C0..C8 in this crate's numbering (middle C = C4; the UI
/// spells the same nine C-1..C7, in Bitwig's). They reach past both ends of
/// any keyboard part, and at 9 octaves to the turn an octave is worth 40
/// degrees, which is wide enough to read at a glance.
pub const DEFAULT_OCTAVE_SPAN: u32 = 4;

/// The pitch axis for a Range of `span` octaves either side of middle C's,
/// tapered by `taper` at strength `amount`.
///
/// The seam is where the walk STARTS and a full turn is what it covers,
/// which is the whole of why the window's two ends meet at the bottom under
/// any settings: the invariant is structural rather than a property of the
/// widths that a new formula could break. Middle C lands straight up because
/// the widths depend only on the distance from it and are therefore
/// symmetric about it — half the circle either side.
pub fn octave_layout(span: u32, taper: OctaveTaper, amount: f32) -> OctaveLayout {
    let span = span.clamp(MIN_OCTAVE_SPAN, MAX_OCTAVE_SPAN);
    let octaves = 2 * span + 1;
    let n = span as f32;

    // Each octave's share of the circle, from how far its own C is from
    // middle C. Normalized by the span, so the taper's shape is the same
    // picture at every Range and only its resolution changes. The middle
    // octave is at distance 0 and the two end ones at 1, which is what makes
    // them equal — and that equality is what lets the highest indicator run
    // off the top of the axis and come round the seam at the right width.
    let mut weights = [0f32; OCTAVE_SLOTS];
    let mut total = 0.0;
    for (j, w) in weights.iter_mut().take(octaves as usize).enumerate() {
        *w = taper.weight((j as f32 - n).abs() / n, amount);
        total += *w;
    }

    // Clockwise is the direction pitch rises (uv.y is up, so the angle
    // decreases), which is why the walk subtracts.
    let mut bounds = [-FRAC_PI_2; OCTAVE_SLOTS + 1];
    let mut acc = 0.0;
    for j in 0..octaves as usize {
        acc += weights[j];
        bounds[j + 1] = -FRAC_PI_2 - TAU * (acc / total);
    }
    for j in octaves as usize + 1..bounds.len() {
        bounds[j] = bounds[octaves as usize];
    }

    OctaveLayout {
        low_pitch: 60.0 - SEMIS * (n + 0.5),
        octaves,
        bounds,
    }
}

impl OctaveLayout {
    /// The highest MIDI pitch the window shows — the seam again, approached
    /// from the right.
    pub fn high_pitch(&self) -> f32 {
        self.low_pitch + SEMIS * self.octaves as f32
    }

    /// Where MIDI pitch `pitch` sits on the wheel, in radians. Linear within
    /// each octave and monotone across them, so an interval reads as an
    /// angle.
    ///
    /// Past the window's high end it CONTINUES rather than clamping, at the
    /// last octave's scale: the highest indicator on a node whose pitch class
    /// is not C runs up to an octave past that end, and continuing is the
    /// same angle as wrapping round the seam into the window's first octave,
    /// which is the same width. Below the low end there is nothing to
    /// continue for — no indicator reaches under it — so that side clamps.
    pub fn angle(&self, pitch: f32) -> f32 {
        let x = ((pitch - self.low_pitch) / SEMIS).max(0.0);
        let j = (x as usize).min(self.octaves as usize - 1);
        self.bounds[j] + (self.bounds[j + 1] - self.bounds[j]) * (x - j as f32)
    }

    /// MIDI pitch of octave slot `slot` on a node whose pitch class is
    /// `cents` (0..1200): the pitch that indicator stands for, and the pitch
    /// it is centered on.
    pub fn slot_pitch(&self, slot: u32, cents: f32) -> f32 {
        slot as f32 * SEMIS + cents / 100.0
    }

    /// The two edge angles of slot `slot`'s indicator on a node whose pitch
    /// class is `cents` — its octave's two ends, read off the axis, the
    /// counter-clockwise one first. The shader's `oct_sector` is this.
    pub fn sector(&self, slot: u32, cents: f32) -> (f32, f32) {
        let pitch = self.slot_pitch(slot, cents);
        (self.angle(pitch - 6.0), self.angle(pitch + 6.0))
    }

    /// The slots every node draws, inclusive: `span` either side of middle
    /// C's. The same octaves on every node whatever its pitch class — a
    /// slot is a MIDI octave, so the Range names octave NUMBERS and each
    /// node's pitch class only says where in the turn its own land.
    ///
    /// Notes outside it fold onto the nearest end (see `derive_scene`), so
    /// this is also what a voice's octave is clamped into.
    pub fn slot_range(&self) -> (u32, u32) {
        let span = (self.octaves - 1) / 2;
        (MIDDLE_C_SLOT as u32 - span, MIDDLE_C_SLOT as u32 + span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const TAPERS: [OctaveTaper; 4] = [
        OctaveTaper::Uniform,
        OctaveTaper::Linear,
        OctaveTaper::Geometric,
        OctaveTaper::Plateau,
    ];

    /// Every span, formula and amount, against pitch classes that put an
    /// indicator's edge exactly on the seam (C), well clear of it, and just
    /// short of it — the grid the invariants below all run over.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        (MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN).flat_map(|span| {
            TAPERS.iter().flat_map(move |&taper| {
                [0.0f32, 0.35, 0.9].iter().flat_map(move |&amount| {
                    [0.0f32, 350.0, 700.0, 1150.0].iter().map(move |&cents| {
                        (
                            octave_layout(span, taper, amount),
                            cents,
                            format!("span {span}, {taper:?} {amount}, {cents}c"),
                        )
                    })
                })
            })
        })
    }

    /// The seam: the window's two ends are one point, and it is the bottom
    /// of the node.
    #[test]
    fn the_windows_ends_meet_at_the_bottom() {
        for (l, _, case) in every_case() {
            assert!((l.angle(l.low_pitch) - -FRAC_PI_2).abs() < 1e-5, "{case}");
            let high = l.angle(l.high_pitch());
            assert!((high - (-FRAC_PI_2 - TAU)).abs() < 1e-4, "{case}: {high}");
        }
    }

    /// Middle C is straight up — on the axis itself, so on every node.
    #[test]
    fn middle_c_is_straight_up() {
        for (l, _, case) in every_case() {
            let up = l.angle(60.0);
            assert!((up - (-FRAC_PI_2 - PI)).abs() < 1e-4, "{case}: {up}");
        }
    }

    /// What "faithful" means, as an assertion: the map is strictly falling
    /// in pitch, and equal intervals WITHIN one octave subtend equal angles,
    /// so an indicator sits on its pitch rather than near it.
    #[test]
    fn the_axis_is_monotone_and_linear_inside_an_octave() {
        for (l, _, case) in every_case() {
            let mut previous = l.angle(l.low_pitch);
            let mut pitch = l.low_pitch;
            while pitch < l.high_pitch() - 0.25 {
                pitch += 0.5;
                let a = l.angle(pitch);
                assert!(a < previous, "{case}: not falling at {pitch}");
                previous = a;
            }
            // Three points evenly spaced in pitch inside ONE octave come out
            // evenly spaced in angle.
            let base = l.low_pitch + SEMIS * (l.octaves / 2) as f32;
            let (a, b, c) = (l.angle(base), l.angle(base + 4.0), l.angle(base + 8.0));
            assert!(((a - b) - (b - c)).abs() < 1e-4, "{case}: uneven inside an octave");
        }
    }

    /// Past the window's high end the axis keeps going, and where it goes is
    /// exactly where wrapping round the seam would put it — the octave up
    /// there and the one at the bottom are equally far from middle C, so
    /// they are equally wide. That is what carries the highest indicator on
    /// a node that is not a C across the bottom in one piece.
    #[test]
    fn the_axis_continues_past_the_seam_where_wrapping_would_land() {
        for (l, _, case) in every_case() {
            for semitones in 0..=12 {
                let d = semitones as f32;
                let past = l.angle(l.high_pitch() + d);
                let round = l.angle(l.low_pitch + d) - TAU;
                let off = (past - round).abs();
                assert!(off < 1e-4, "{case}: {d} past the seam: {past} vs {round}");
            }
        }
    }

    /// The Range names octave NUMBERS, and every node draws all of them: the
    /// set is `span` slots either side of middle C's whatever the node's
    /// pitch class, so ±2 is C1..C5 on a C node and on every other.
    #[test]
    fn every_node_draws_the_octaves_the_range_names() {
        for span in MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN {
            let l = octave_layout(span, OctaveTaper::Uniform, 0.0);
            for cents in [0.0f32, 350.0, 700.0, 1150.0] {
                let (low, high) = l.slot_range();
                assert_eq!(
                    (low, high),
                    (MIDDLE_C_SLOT as u32 - span, MIDDLE_C_SLOT as u32 + span),
                    "span {span}, {cents}c"
                );
                assert_eq!(high - low + 1, 2 * span + 1, "span {span}: indicator count");
                // Room for all of them, with the outermost pair's own pitches
                // half an octave inside the window's ends.
                assert!(l.slot_pitch(low, cents) - 6.0 >= l.low_pitch - 1e-3);
                assert!(l.slot_pitch(high, cents) - 6.0 < l.high_pitch());
            }
        }
    }

    /// A drawn indicator is a WHOLE octave, centered on its own pitch. That
    /// is what keeps the mapping honest: an indicator cut short would
    /// misstate its octave's width, and one moved to fit would misstate its
    /// pitch.
    #[test]
    fn drawn_indicators_are_whole_octaves_on_their_own_pitch() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            assert!(low <= high, "{case}: nothing drawn");
            for slot in low..=high {
                let pitch = l.slot_pitch(slot, cents);
                // Its edges are its octave's ends, read off the axis: half an
                // octave either side of its own pitch, and nothing else.
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 > e1, "{case}: slot {slot} runs backwards");
                let inside = l.angle(pitch);
                assert!(e0 > inside && inside > e1, "{case}: slot {slot} misses its own pitch");
                // Under an even axis that also puts the pitch at the
                // indicator's angular MIDDLE. A taper legitimately breaks
                // that and only that: the scale changes at each boundary, so
                // an indicator straddling one has its two halves at different
                // scales. Its edges are still exactly its octave's ends,
                // which is what "positioned by pitch" means here.
                if l.bounds[0] - l.bounds[1] == l.bounds[1] - l.bounds[2] {
                    assert!(
                        (inside - 0.5 * (e0 + e1)).abs() < 1e-4,
                        "{case}: slot {slot} is not centered on its pitch"
                    );
                }
            }
        }
    }

    /// The indicators tile the turn: each one's clockwise edge is the next
    /// one's counter-clockwise edge, and the highest one's far edge is the
    /// lowest one's near edge come round the seam. So the ring closes on
    /// every node whatever its pitch class — which is the whole point of
    /// giving the window an octave per indicator and half an octave over.
    #[test]
    fn the_indicators_tile_the_turn() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            let mut total = 0.0;
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                total += e0 - e1;
                let next = if slot == high {
                    // Round the seam: the same point, one turn on.
                    l.sector(low, cents).0 - TAU
                } else {
                    l.sector(slot + 1, cents).0
                };
                assert!((e1 - next).abs() < 1e-4, "{case}: slot {slot} leaves {} rad", e1 - next);
            }
            assert!((total - TAU).abs() < 1e-4, "{case}: the ring covers {total} rad");
        }
    }

    /// Octaves shrink outward, symmetrically, and only when a taper is asked
    /// for: the middle of the window is what carries the extra weight.
    #[test]
    fn octaves_shrink_away_from_middle_c() {
        for taper in TAPERS {
            let l = octave_layout(5, taper, 0.6);
            let width = |j: usize| l.bounds[j] - l.bounds[j + 1];
            // Eleven octaves, so middle C's own is the middle one, index 5,
            // and the walk outward from it either way is what shrinks.
            for j in 0..5 {
                let (inner, outer) = (width(j + 1), width(j));
                if taper == OctaveTaper::Uniform {
                    assert!((inner - outer).abs() < 1e-6, "{taper:?} at {j}");
                } else {
                    assert!(outer < inner, "{taper:?} at {j}: {outer} !< {inner}");
                }
                // Symmetric about middle C's octave.
                assert!((width(j) - width(10 - j)).abs() < 1e-6, "{taper:?} at {j}");
            }
        }
    }

    /// Every formula means the same thing by its amount, which is what makes
    /// flipping between them at a fixed amount a fair comparison.
    #[test]
    fn tapers_agree_at_both_ends() {
        for taper in TAPERS {
            assert!((taper.weight(0.0, 0.5) - 1.0).abs() < 1e-6, "{taper:?}");
            if taper != OctaveTaper::Uniform {
                assert!((taper.weight(1.0, 0.5) - 0.5).abs() < 1e-6, "{taper:?}");
            }
        }
    }

    /// An indicator can pass a half turn but never a whole one, which is
    /// exactly the pair of facts the shader's wedge test is built on: past a
    /// half turn a wedge is the UNION of its two half-planes rather than
    /// their intersection, and at a whole turn neither reading means
    /// anything. The widest there is is middle C's own octave at the
    /// narrowest Range under the steepest Ratio taper — five octaves to the
    /// turn with the middle one taking ten times the width of the ends.
    #[test]
    fn an_indicator_can_pass_a_half_turn_but_never_a_whole_one() {
        let mut widest: f32 = 0.0;
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 - e1 < TAU, "{case}: slot {slot} spans {} rad", e0 - e1);
                widest = widest.max(e0 - e1);
            }
        }
        assert!(widest > PI, "nothing passes a half turn: widest {widest} rad");
        let steepest = octave_layout(MIN_OCTAVE_SPAN, OctaveTaper::Geometric, MAX_TAPER_AMOUNT);
        let (e0, e1) = steepest.sector(MIDDLE_C_SLOT as u32, 0.0);
        assert!((widest - (e0 - e1)).abs() < 1e-5, "the widest is somewhere else: {widest}");
    }

    /// A span outside the supported range is clamped rather than producing a
    /// layout the shader's fixed-size tables cannot hold. The widest holds
    /// every octave MIDI has, so nothing folds there.
    #[test]
    fn span_is_clamped_to_the_tables() {
        assert_eq!(octave_layout(0, OctaveTaper::Uniform, 0.0).octaves, 2 * MIN_OCTAVE_SPAN + 1);
        let widest = octave_layout(99, OctaveTaper::Uniform, 0.0);
        assert_eq!(widest.octaves, 2 * MAX_OCTAVE_SPAN + 1);
        assert_eq!(widest.octaves as usize, OCTAVE_SLOTS);
        assert_eq!(widest.slot_range(), (0, OCTAVE_SLOTS as u32 - 1));
        assert_eq!(widest.low_pitch, -6.0);
        assert_eq!(widest.high_pitch(), 126.0);
    }
}
