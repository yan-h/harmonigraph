//! The color table's cache, which is invisible in what it returns and so is
//! the one thing about it a value test cannot reach.
//!
//! Two gradients are live in one frame — the lattice's, walked per node by the
//! scene derive, and the Spectral pane's, walked per slab by the spectrum curve
//! — so a cache holding one would rebuild both every frame rather than neither.
//! What that costs is the whole reason the table exists: a rebuild is
//! `PITCH_LUT_N` gamut bisections, each a Newton solve and an Oklab->sRGB
//! conversion.

use crate::color::REBUILDS;
use crate::{gradient_color, Gradient};

/// One gradient per number, differing in a knob that certainly changes the
/// table — a hue arc nothing else in the tree opens on.
fn nth(n: u32) -> Gradient {
    Gradient { hue_start: 7.0 * n as f32 + 3.0, ..Gradient::default() }
}

/// How many tables were built while `body` ran.
fn rebuilds(body: impl FnOnce()) -> u32 {
    let before = REBUILDS.with(|n| n.get());
    body();
    REBUILDS.with(|n| n.get()) - before
}

/// Two gradients asked for in turn, over and over, cost two rebuilds — which is
/// what a frame drawing both the lattice and the heatmap does, and what a
/// one-slot cache turns into two rebuilds per alternation.
#[test]
fn two_gradients_alternating_are_both_resident() {
    let (a, b) = (nth(1), nth(2));
    // Warm both, so the count below is about staying resident rather than about
    // arriving.
    gradient_color(0.5, a);
    gradient_color(0.5, b);
    let built = rebuilds(|| {
        for i in 0..50 {
            // Several asks of each before switching, which is the real shape:
            // a pane walks its whole curve before the next one draws.
            for _ in 0..8 {
                gradient_color(i as f32 / 50.0, if i % 2 == 0 { a } else { b });
            }
        }
    });
    assert_eq!(built, 0, "a resident pair must not rebuild at all");
}

/// And a third evicts, rather than the cache growing without bound: the policy
/// is two slots, most recently used kept.
#[test]
fn a_third_gradient_evicts_the_stalest() {
    let (a, b, c) = (nth(3), nth(4), nth(5));
    gradient_color(0.5, a);
    gradient_color(0.5, b);
    // `a` is now the stale one, so asking for `c` costs one build and drops it.
    assert_eq!(rebuilds(|| { gradient_color(0.5, c); }), 1);
    assert_eq!(rebuilds(|| { gradient_color(0.5, b); }), 0, "b was the fresher of the pair");
    assert_eq!(rebuilds(|| { gradient_color(0.5, a); }), 1, "a was evicted by c");
}

/// A gradient that sanitizes to the one already held is the SAME entry, not a
/// second — which is what keeps a hand-edited blob, or a NaN out of a corrupt
/// float, from costing a rebuild on every frame it is drawn.
#[test]
fn gradients_drawing_one_picture_are_one_entry() {
    let g = nth(6);
    gradient_color(0.5, g);
    // A span past a full turn and a hue past a full circle both come back to
    // what is already in the slot.
    let same = Gradient { hue_start: g.hue_start + 360.0, hue_span: g.hue_span, ..g };
    assert_eq!(rebuilds(|| { gradient_color(0.5, same); }), 0);
    // And a NaN, which is the case that would break the cache rather than only
    // cost it: NaN is not equal to itself, so an unsanitized key would MISS
    // against the entry it just wrote and rebuild on every single call.
    // `nth` builds off the type's own defaults, so the repair lands back on
    // exactly `g` and the same entry serves.
    let nonfinite = Gradient { lightness: f32::NAN, ..g };
    assert_eq!(rebuilds(|| { gradient_color(0.5, nonfinite); }), 0);
    assert_eq!(rebuilds(|| { gradient_color(0.5, nonfinite); }), 0, "and again, not just once");
}
