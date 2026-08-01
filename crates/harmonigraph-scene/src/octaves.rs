//! Where each octave indicator sits around a node: the pitch axis the wheel
//! is, how wide each octave of it is, and which of a node's octaves fit.
//!
//! **The wheel is a pitch axis, and it is the same axis on every node.** One
//! monotone map takes an absolute MIDI pitch to an angle: middle C straight
//! up, rising pitch clockwise, one full turn covering the window the Range
//! setting names. An indicator is drawn at the angle of the pitch it stands
//! for — so where a node's indicators sit says which pitches those octaves
//! ARE, and two nodes' indicators for the same octave sit at different
//! angles exactly as their pitches differ.
//!
//! **The bottom is both ends of the window at once.** The map's two ends —
//! the lowest pitch shown and the highest — land on the same point, straight
//! down. That is the seam, and it is at the bottom on every node whatever
//! the node's pitch class, which is the thing a per-node rotation of the
//! wheel cannot give.
//!
//! **An indicator that would cross the seam is not drawn.** Only whole
//! octaves are: each is exactly one octave of the axis wide, centered
//! exactly on its own pitch. A node whose pitch class is not C has its
//! octaves sitting off the window's boundaries, so up to one octave's worth
//! of the circle — split either side of the bottom — is simply empty.
//! Nothing is stretched or turned to close it, which is the whole point: the
//! price of filling the ring is a mapping that lies about where a pitch is.
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

/// Slot of middle C's octave. The window is centered on middle C itself
/// (MIDI 60), which is this slot's C.
pub const MIDDLE_C_SLOT: usize = 5;

/// Narrowest window: 2 octaves either side of middle C.
pub const MIN_OCTAVE_SPAN: u32 = 2;
/// Widest window: 5 octaves either side — MIDI 0..120.
pub const MAX_OCTAVE_SPAN: u32 = 5;

// The widest window has to fit the fixed-size table it is written into: one
// boundary angle per octave of the window, plus the closing one. Raising
// MAX_OCTAVE_SPAN alone would run `octave_layout` off the end of `bounds`, a
// runtime panic in the render path; the renderer's own ceiling on
// OCTAVE_SLOTS is a build error, and this makes the pair fail the same way.
const _: () = assert!(2 * (MAX_OCTAVE_SPAN as usize) <= OCTAVE_SLOTS);

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
    /// Octaves the window spans: `2 * span`, so both its ends are Cs and
    /// middle C is the boundary in the middle.
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

/// Window the view starts at: 4 octaves either side of middle C — C0..C8 in
/// this crate's numbering (middle C = C4; the UI spells the same window
/// C-1..C7, in Bitwig's). It reaches past both ends of any keyboard part,
/// and at 8 octaves to the turn an octave is worth 45 degrees, which is wide
/// enough to read at a glance.
pub const DEFAULT_OCTAVE_SPAN: u32 = 4;

/// The pitch axis for a window of `span` octaves either side of middle C,
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
    let octaves = 2 * span;
    let n = span as f32;

    // Each octave's share of the circle, from the distance of its MIDDLE to
    // middle C. Normalized by the outermost octave's distance rather than by
    // the span, so the taper's shape is the same picture at every window
    // size and only its resolution changes.
    let max_distance = n - 0.5;
    let mut weights = [0f32; OCTAVE_SLOTS];
    let mut total = 0.0;
    for (j, w) in weights.iter_mut().take(octaves as usize).enumerate() {
        *w = taper.weight((j as f32 + 0.5 - n).abs() / max_distance, amount);
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
        low_pitch: 60.0 - SEMIS * n,
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
    /// angle; pitches outside the window clamp to the seam.
    pub fn angle(&self, pitch: f32) -> f32 {
        let x = ((pitch - self.low_pitch) / SEMIS).clamp(0.0, self.octaves as f32);
        let j = (x as usize).min(self.octaves as usize - 1);
        self.bounds[j] + (self.bounds[j + 1] - self.bounds[j]) * (x - j as f32)
    }

    /// MIDI pitch of octave slot `slot` on a node whose pitch class is
    /// `cents` (0..1200): the pitch that indicator stands for, and the pitch
    /// it is centered on.
    pub fn slot_pitch(&self, slot: u32, cents: f32) -> f32 {
        slot as f32 * SEMIS + cents / 100.0
    }

    /// Whether slot `slot`'s whole octave fits inside the window on a node
    /// whose pitch class is `cents`. An indicator spans half an octave either
    /// side of its own pitch, and one that would cross the seam is not drawn
    /// at all rather than cut short or moved — cutting it would misstate the
    /// octave's width, and moving it would misstate its pitch.
    ///
    /// The tolerance is for the C case, where an indicator's edge lands
    /// exactly on the window's boundary and float error alone should not
    /// decide whether it is shown.
    pub fn slot_fits(&self, slot: u32, cents: f32) -> bool {
        let pitch = self.slot_pitch(slot, cents);
        pitch - 6.0 >= self.low_pitch - 1e-3 && pitch + 6.0 <= self.high_pitch() + 1e-3
    }

    /// The slots drawn on a node whose pitch class is `cents`, inclusive.
    ///
    /// Never empty: the narrowest window is 4 octaves, and at least three
    /// whole ones fit inside it wherever the pitch class falls. Notes
    /// outside it fold onto the nearest end (see `derive_scene`), so the
    /// range is also what a voice's octave is clamped into.
    pub fn slot_range(&self, cents: f32) -> (u32, u32) {
        let c = cents / 100.0;
        // The lowest indicator whose bottom edge clears the window's low end,
        // and the highest whose top edge clears its high end. Solved rather
        // than searched, and nudged by the same tolerance `slot_fits` uses so
        // the two cannot disagree about a C node's exactly-flush edges.
        let low = ((self.low_pitch + 6.0 - c) / SEMIS - 1e-4).ceil().max(0.0);
        let high = ((self.high_pitch() - 6.0 - c) / SEMIS + 1e-4).floor();
        (low as u32, high.clamp(low, OCTAVE_SLOTS as f32 - 1.0) as u32)
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

    /// A drawn indicator is a WHOLE octave, centered on its own pitch, clear
    /// of the seam. The first half keeps the seam readable, the second is
    /// what keeps the mapping honest.
    #[test]
    fn drawn_indicators_are_whole_octaves_on_their_own_pitch() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            assert!(low <= high, "{case}: nothing drawn");
            for slot in low..=high {
                assert!(l.slot_fits(slot, cents), "{case}: slot {slot} drawn but doesn't fit");
                let pitch = l.slot_pitch(slot, cents);
                // Its edges are its octave's ends, read off the axis: half an
                // octave either side of its own pitch, and nothing else.
                let (e0, e1) = (l.angle(pitch - 6.0), l.angle(pitch + 6.0));
                assert!(e0 > e1, "{case}: slot {slot} runs backwards");
                let inside = l.angle(pitch);
                assert!(e0 > inside && inside > e1, "{case}: slot {slot} misses its own pitch");
                // Under an even axis that also puts the pitch at the
                // indicator's angular MIDDLE. A taper legitimately breaks
                // that and only that: the scale changes at each C, so an
                // indicator straddling one has its two halves at different
                // scales. Its edges are still exactly its octave's ends,
                // which is what "positioned by pitch" means here.
                if l.bounds[0] - l.bounds[1] == l.bounds[1] - l.bounds[2] {
                    assert!(
                        (inside - 0.5 * (e0 + e1)).abs() < 1e-4,
                        "{case}: slot {slot} is not centered on its pitch"
                    );
                }
            }
            // The range IS what is drawn: nothing outside it fits.
            for slot in 0..OCTAVE_SLOTS as u32 {
                if slot < low || slot > high {
                    assert!(!l.slot_fits(slot, cents), "{case}: slot {slot} fits but isn't drawn");
                }
            }
        }
    }

    /// The gap that buys the faithful mapping: under one octave in total,
    /// and split evenly for a C node, whose octaves line up with the window.
    #[test]
    fn the_seam_gap_is_under_an_octave_and_is_even_on_c() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            let below = l.slot_pitch(low, cents) - 6.0 - l.low_pitch;
            let above = l.high_pitch() - (l.slot_pitch(high, cents) + 6.0);
            assert!(below >= -1e-3 && above >= -1e-3, "{case}: an indicator crosses the seam");
            assert!(below + above < SEMIS + 1e-3, "{case}: {below} + {above} semitones unfilled");
            if cents == 0.0 {
                // A C node's octaves ARE the window's octaves, so all that is
                // left over is the half either side that would cross.
                assert!((below - 6.0).abs() < 1e-3, "{case}: {below}");
                assert!((above - 6.0).abs() < 1e-3, "{case}: {above}");
            }
        }
    }

    /// Octaves shrink outward, symmetrically, and only when a taper is asked
    /// for: the middle of the window is what carries the extra weight.
    #[test]
    fn octaves_shrink_away_from_middle_c() {
        for taper in TAPERS {
            let l = octave_layout(5, taper, 0.6);
            let width = |j: usize| l.bounds[j] - l.bounds[j + 1];
            // Ten octaves, so the window's middle is the boundary BETWEEN
            // octaves 4 and 5 rather than inside one: those two are equally
            // near middle C and come out equal, and the walk outward from
            // there is what shrinks.
            assert!((width(4) - width(5)).abs() < 1e-5, "{taper:?}: middle pair uneven");
            for j in 0..4 {
                let (inner, outer) = (width(j + 1), width(j));
                if taper == OctaveTaper::Uniform {
                    assert!((inner - outer).abs() < 1e-6, "{taper:?} at {j}");
                } else {
                    assert!(outer < inner, "{taper:?} at {j}: {outer} !< {inner}");
                }
                // Symmetric about middle C.
                assert!((width(j) - width(9 - j)).abs() < 1e-6, "{taper:?} at {j}");
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

    /// No indicator can reach a half turn, which is what lets the shader's
    /// wedge test stay a plain intersection of two half-planes. The widest
    /// one there is is the middle octave of the narrowest window under the
    /// steepest taper.
    #[test]
    fn no_indicator_reaches_a_half_turn() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            for slot in low..=high {
                let pitch = l.slot_pitch(slot, cents);
                let width = l.angle(pitch - 6.0) - l.angle(pitch + 6.0);
                assert!(width < PI, "{case}: slot {slot} spans {width} rad");
            }
        }
    }

    /// A span outside the supported range is clamped rather than producing a
    /// layout the shader's fixed-size table cannot hold.
    #[test]
    fn span_is_clamped_to_the_table() {
        assert_eq!(octave_layout(0, OctaveTaper::Uniform, 0.0).octaves, 2 * MIN_OCTAVE_SPAN);
        let widest = octave_layout(99, OctaveTaper::Uniform, 0.0);
        assert_eq!(widest.octaves, 2 * MAX_OCTAVE_SPAN);
        assert_eq!(widest.low_pitch, 0.0);
        assert_eq!(widest.high_pitch(), 120.0);
    }
}
