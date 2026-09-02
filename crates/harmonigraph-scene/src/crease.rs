//! The two-distance rule's arithmetic (#568 §2), in Rust: what a Distance
//! shadow reads where two pieces of one caster face each other.
//!
//! A distance shadow spends `p(d) = exp(-`[`SHADOW_TAIL`]`·d/w)` of the
//! distance to the NEAREST ink, and nearest is a `min`, so between two facing
//! pieces the second one contributes nothing and the medial axis carries a
//! crease. The rule unions a SECOND distance, weighted by how squarely its foot
//! faces the point, and carries the pair as one effective distance so the
//! consumer still reads one exponential.
//!
//! One copy, because it is evaluated in three places that must agree: the CPU
//! model the ramp was settled against (`lattice_tests::crease`), the glyph
//! bake that stores a second distance per texel (`harmonigraph-ui`'s
//! `text_sdf`), and the shader that reads it back.

use glam::Vec2;

use crate::style::{BEYOND_RAMP, SHADOW_STOP, SHADOW_TAIL};

/// The share of the standoff a distance of `d` spends at width `w`, unwindowed:
/// `standoff_coverage`'s exponential without its window.
pub fn spend(d: f32, w: f32) -> f32 {
    (-SHADOW_TAIL * d.max(0.0) / w).exp()
}

/// common.wgsl's `standoff_coverage`: that decay, closed by its window.
pub fn standoff_coverage(d: f32, w: f32) -> f32 {
    let u = d.max(0.0) / w.max(1.0e-6);
    (-SHADOW_TAIL * u).exp() * (1.0 - smoothstep(1.0, SHADOW_STOP, u))
}

/// WGSL's `smoothstep`.
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The one distance whose standoff coverage equals the union of two — #568
/// §2's `union_distance`, in the form the shader carries.
///
/// Equals `d1` exactly where the second term is past the window or faces away,
/// so a lone feature round-trips; and never more than `(w/`[`SHADOW_TAIL`]`)·ln 2`
/// under `d1`, so the pad a cell is packed with still holds the whole shadow.
pub fn union_distance(d1: f32, d2: f32, k: f32, w: f32) -> f32 {
    if d1 <= 0.0 || d2 >= SHADOW_STOP * w || k <= 0.0 {
        return d1;
    }
    let a = spend(d1, w);
    let b = k * spend(d2, w);
    -(w / SHADOW_TAIL) * (1.0 - (1.0 - a) * (1.0 - b)).ln()
}

/// How much of a second foot counts at a ramp of `ramp`: 1 on or beyond the
/// plane facing away from the nearest ink, 0 at `ramp` behind it.
///
/// Both arguments are offsets FROM the point — `near` to its nearest ink,
/// `foot` to the second feature's nearest point — which is the vector the
/// producers carry. `ramp` 0 is the hard predicate #568's first comment
/// measures the step of, and is a parameter so one test can evaluate both.
pub fn facing_at(near: Vec2, foot: Vec2, ramp: f32) -> f32 {
    let (n, f) = (near.length(), foot.length());
    if n <= 0.0 || f <= 0.0 {
        return 0.0;
    }
    facing_cosine(foot.dot(-near / n) / f, ramp)
}

/// [`facing_at`] from the cosine alone, which is what the sheet stores and the
/// shader reads back.
pub fn facing_cosine(cos_phi: f32, ramp: f32) -> f32 {
    if ramp <= 0.0 {
        return f32::from(cos_phi >= 0.0);
    }
    smoothstep(-ramp, 0.0, cos_phi)
}

/// [`facing_at`] at the ramp the picture is drawn with.
pub fn facing(near: Vec2, foot: Vec2) -> f32 {
    facing_at(near, foot, BEYOND_RAMP)
}

/// One e-fold of the standoff, in the units `w` is given in: the length a
/// pocket's correction tapers over as its foot approaches the end of the
/// segment it stands on.
///
/// The decay length of `p` itself and not a length read off the gap, because
/// what the taper has to be small compared to is the distance over which the
/// second term is worth anything at all.
pub const fn taper_length(w: f32) -> f32 {
    w / SHADOW_TAIL
}

/// How much of a facing pair counts, 0..=1, from each foot's clearance to the
/// nearest CONVEX end of its own segment: zero, with zero slope, where either
/// foot IS that end.
///
/// [`facing`] alone admits two convex corners seen from outside a gap — the
/// exterior bisector of a mouth puts the far corner within the ramp at every
/// width, so no ramp separates a mouth from a concave junction; both stand
/// their feet at 90°. What separates them is that a mouth's feet are segment
/// ENDS, and a concave junction's are segment interiors.
///
/// The PRODUCT and not a taper of the smaller clearance: a pair is worth what
/// both of its feet are worth, so two feet each half a taper from their ends
/// count for a quarter rather than a half.
pub fn pocket(h_near: f32, h_foot: f32, l: f32) -> f32 {
    smoothstep(0.0, l, h_near) * smoothstep(0.0, l, h_foot)
}
