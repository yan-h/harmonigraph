//! Where each octave indicator sits around a node: how many there are, how
//! wide each one is, and the angles that divide them.
//!
//! The indicators tile a full circle. Two invariants hold for every setting
//! here, and everything else in the module follows from them:
//!
//! - **The split between the lowest and the highest indicator is at the very
//!   bottom of the node.** Both extremes of the shown range end against the
//!   same seam, so the wheel reads as a pitch axis that runs up both sides
//!   and meets at the bottom rather than as a ring that happens to start
//!   somewhere.
//! - **Middle C's indicator is centered straight up.** That falls out of the
//!   first invariant on its own, because the widths are a function of the
//!   distance from middle C and are therefore symmetric about it: half the
//!   circle sits either side of the middle indicator's bisector.
//!
//! The layout is computed here, on the CPU, and handed to the shader as a
//! table of boundary angles — the alternative is recomputing the cumulative
//! widths per pixel per sector, which is the same arithmetic done a few
//! million times a frame for a value that changes when a setting does.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Octave indicator slots: MIDI octaves -1..=9, so `slot = octave + 1` and
/// middle C (octave 4, MIDI 60) is slot 5. Eleven is what the widest span —
/// 5 octaves either side of middle C — needs, and it also covers the whole
/// MIDI range, so at that span no note is folded into a neighbour's
/// indicator. The renderer packs one byte per slot into 3 words and asserts
/// the count fits.
pub const OCTAVE_SLOTS: usize = 11;

/// Slot of middle C. The layout is symmetric about it, and a span of `n`
/// shows slots `MIDDLE_C_SLOT - n ..= MIDDLE_C_SLOT + n`.
pub const MIDDLE_C_SLOT: usize = 5;

/// Narrowest span: 2 octaves either side of middle C, 5 indicators.
pub const MIN_OCTAVE_SPAN: u32 = 2;
/// Widest span: 5 octaves either side, 11 indicators — every MIDI octave.
pub const MAX_OCTAVE_SPAN: u32 = 5;

/// Ceiling on the taper amount. At 1 the outermost indicator would have no
/// width at all, which is a range that claims to show an octave and doesn't;
/// 0.9 leaves it a tenth of the middle one, which is still a sliver but a
/// visible one.
pub const MAX_TAPER_AMOUNT: f32 = 0.9;

/// How an indicator's width falls off with its distance from middle C, so
/// the middle of the range carries more visual weight than its extremes.
///
/// Every formula is a function of ONE normalized distance (0 at middle C, 1
/// at the outermost octave shown) and ONE amount, and every one of them
/// agrees at both ends: width 1 at middle C, `1 - amount` at the extremes.
/// So the amount always means the same thing — how much of its width the
/// outermost octave gives up — and the formulas differ only in how that loss
/// is distributed across the octaves in between. That is what makes them
/// comparable by flipping between them at a fixed amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OctaveTaper {
    /// Equal widths; the amount is inert. The plain circular division, and
    /// what the indicators were before they could be resized.
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
    /// Cosine ease: flat across the middle of the range and steepest at the
    /// halfway point. Reads as a group of full-size middle octaves rather
    /// than as a gradient, which is the difference from Linear.
    Smooth,
}

impl OctaveTaper {
    /// Relative width of an indicator `x` of the way from middle C (0) to
    /// the outermost octave shown (1). Only the RATIOS between the returned
    /// weights matter — [`octave_layout`] normalizes them onto the circle —
    /// so this is a shape, not a size.
    pub fn weight(self, x: f32, amount: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let amount = amount.clamp(0.0, MAX_TAPER_AMOUNT);
        match self {
            OctaveTaper::Uniform => 1.0,
            OctaveTaper::Linear => 1.0 - amount * x,
            OctaveTaper::Geometric => (1.0 - amount).powf(x),
            OctaveTaper::Smooth => 1.0 - amount * 0.5 * (1.0 - (PI * x).cos()),
        }
    }
}

/// The angles that divide the octave indicators, ready for the shader.
///
/// Computed once per frame from the view settings, not per node: every node
/// wears the SAME wheel. What an indicator says is which octave; which pitch
/// class is already said by which node it is drawn on, and turning each
/// node's wheel by its pitch class would only move the seam off the bottom
/// in exchange for saying that twice. The node's cents still COLOR its
/// indicators, each by its own octave's true pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveLayout {
    /// Slot of the lowest octave shown; the shader's sector `u` is slot
    /// `first_slot + u`.
    pub first_slot: u32,
    /// Indicators shown: `2 * span + 1`, always odd, so one of them is
    /// middle C's and the other two halves of the range are equal.
    pub count: u32,
    /// Angle of each boundary, in radians, walking CLOCKWISE (the direction
    /// pitch rises) from the bottom seam: `bounds[0]` is the bottom,
    /// `bounds[u]..bounds[u + 1]` bound sector `u`, and `bounds[count]` is a
    /// full turn on from `bounds[0]` — the same seam, come round the other
    /// way. Entries past `count` repeat the last so a stale index cannot
    /// produce a wild angle.
    pub bounds: [f32; OCTAVE_SLOTS + 1],
}

impl Default for OctaveLayout {
    fn default() -> Self {
        octave_layout(DEFAULT_OCTAVE_SPAN, OctaveTaper::Uniform, 0.0)
    }
}

/// Span the view starts at: 4 octaves either side of middle C (C0..B8),
/// which reaches past both ends of any keyboard part while keeping the
/// indicators wide enough to read at a glance.
pub const DEFAULT_OCTAVE_SPAN: u32 = 4;

/// The boundary angles for `span` octaves either side of middle C, tapered
/// by `taper` at strength `amount`.
///
/// The bottom seam is where the walk STARTS and a full turn is what it
/// covers, which is the whole of why the lowest and highest indicators meet
/// there under any settings: the invariant is structural rather than a
/// property of the widths that a new formula could break.
pub fn octave_layout(span: u32, taper: OctaveTaper, amount: f32) -> OctaveLayout {
    let span = span.clamp(MIN_OCTAVE_SPAN, MAX_OCTAVE_SPAN);
    let count = 2 * span + 1;

    // Distance from middle C normalized to the span, so the taper's shape is
    // the same picture at every span and only its resolution changes.
    let n = span as f32;
    let mut weights = [0f32; OCTAVE_SLOTS];
    let mut total = 0.0;
    for (u, w) in weights.iter_mut().take(count as usize).enumerate() {
        *w = taper.weight((u as f32 - n).abs() / n, amount);
        total += *w;
    }

    // Clockwise is the direction pitch rises (uv.y is up, so the angle
    // decreases), which is why the walk subtracts.
    let mut bounds = [-FRAC_PI_2; OCTAVE_SLOTS + 1];
    let mut acc = 0.0;
    for u in 0..count as usize {
        acc += weights[u];
        bounds[u + 1] = -FRAC_PI_2 - TAU * (acc / total);
    }
    for j in count as usize + 1..bounds.len() {
        bounds[j] = bounds[count as usize];
    }

    OctaveLayout {
        first_slot: MIDDLE_C_SLOT as u32 - span,
        count,
        bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAPERS: [OctaveTaper; 4] = [
        OctaveTaper::Uniform,
        OctaveTaper::Linear,
        OctaveTaper::Geometric,
        OctaveTaper::Smooth,
    ];

    /// The one invariant the whole module exists to hold: whatever the span,
    /// the formula or the amount, the lowest and highest indicators are
    /// split at the very bottom of the node.
    #[test]
    fn lowest_and_highest_split_at_the_bottom() {
        for span in MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN {
            for taper in TAPERS {
                for amount in [0.0, 0.35, 0.9] {
                    let l = octave_layout(span, taper, amount);
                    // Both ends of the walk are the bottom, one turn apart.
                    assert!((l.bounds[0] - -FRAC_PI_2).abs() < 1e-6, "{span} {taper:?}");
                    let end = l.bounds[l.count as usize];
                    assert!(
                        (end - (-FRAC_PI_2 - TAU)).abs() < 1e-5,
                        "{span} {taper:?} {amount}: {end}"
                    );
                }
            }
        }
    }

    /// Middle C's indicator is centered straight up — the symmetry that
    /// falls out of the widths depending only on the distance from it.
    #[test]
    fn middle_c_is_centered_up() {
        for span in MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN {
            for taper in TAPERS {
                let l = octave_layout(span, taper, 0.7);
                let mid_u = span as usize;
                let center = 0.5 * (l.bounds[mid_u] + l.bounds[mid_u + 1]);
                // Up is pi/2; the walk has passed half a turn to get here.
                assert!(
                    (center - (-FRAC_PI_2 - PI)).abs() < 1e-5,
                    "{span} {taper:?}: {center}"
                );
            }
        }
    }

    /// Widths shrink outward, symmetrically, and only when a taper is asked
    /// for: the middle of the range is what carries the extra weight.
    #[test]
    fn widths_shrink_away_from_middle_c() {
        for taper in TAPERS {
            let l = octave_layout(5, taper, 0.6);
            let width = |u: usize| l.bounds[u] - l.bounds[u + 1];
            for u in 0..5 {
                let (inner, outer) = (width(u + 1), width(u));
                if taper == OctaveTaper::Uniform {
                    assert!((inner - outer).abs() < 1e-6, "{taper:?} at {u}");
                } else {
                    assert!(outer < inner, "{taper:?} at {u}: {outer} !< {inner}");
                }
                // Symmetric about the middle indicator.
                assert!((width(u) - width(10 - u)).abs() < 1e-6, "{taper:?} at {u}");
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

    /// A span outside the supported range is clamped rather than producing a
    /// layout the shader's fixed-size table cannot hold.
    #[test]
    fn span_is_clamped_to_the_table() {
        assert_eq!(octave_layout(0, OctaveTaper::Uniform, 0.0).count, 2 * MIN_OCTAVE_SPAN + 1);
        let widest = octave_layout(99, OctaveTaper::Uniform, 0.0);
        assert_eq!(widest.count as usize, OCTAVE_SLOTS);
        assert_eq!(widest.first_slot, 0);
    }
}
