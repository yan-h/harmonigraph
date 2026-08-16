//! The color table's cache, which is invisible in what it returns and so is
//! the one thing about it a value test cannot reach.
//!
//! Three gradients are live in one frame — the lattice's, walked per node by
//! the scene derive; the Spectral pane's, walked per slab by the spectrum
//! curve; and the audio ring's, which is the analyzer's own re-anchored on the
//! lattice's bed and so a key of its own — and a FOURTH while any of them is
//! being dragged, a bar painting the value it has just written over the value
//! every pane above it read. A cache shallower than that rebuilds tables it is
//! still holding. What that costs is the whole reason
//! the table exists: a rebuild is `PITCH_LUT_N` gamut bisections, each a Newton
//! solve and an Oklab->sRGB conversion.
//!
//! The hue circle's memo is here too, for the half of that argument which
//! applies to it. It holds ONE slot, every caller asking for the same pair, so
//! what a value test cannot see about it is not eviction but whether it looks
//! at its key at all.

use crate::color::{LUT_SLOTS, REBUILDS};
use crate::{gradient_color, hue_circle, Gradient, ViewConfig};

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

/// A frame of a GRADIENT DRAG asks for four, and only the newest of them is a
/// fair rebuild.
///
/// The live set is not "one gradient per picture" but one per picture plus the
/// one a bar is in the middle of writing, because a settings pane draws AFTER
/// the display panes: everything above the bar in a frame reads the value the
/// bar wrote LAST frame, and then the bar writes and paints a new one. Three
/// pictures are live — the lattice's nodes, the Spectral pane's curve, and the
/// audio ring, whose table is the analyzer's gradient re-anchored onto the
/// lattice's bed (`SpectralPaint::new`) and so a KEY OF ITS OWN — so a drag
/// frame walks four keys, of which three were already built.
///
/// The drag here is the LATTICE's, which is where a slot short of four shows
/// up: the ring's key and the Spectral pane's hold still through the whole
/// gesture, and it is exactly those two that a cache one short evicts and
/// rebuilds every frame for nothing. Dragging the ANALYZER's gradient moves two
/// pictures at once instead — the pane's and the ring's copy — and still walks
/// four keys, since the ring reads last frame's value like every other display;
/// two of its four rebuilds are then fair.
///
/// Held as the rebuild count over several frames rather than over one: the
/// first frame of a drag legitimately misses on everything, and what this is
/// about is the steady state a held drag settles into.
#[test]
fn a_frame_of_a_drag_rebuilds_only_what_the_drag_changed() {
    // The analyzer's picture and the audio ring's copy of it, which stand still
    // while the lattice's own gradient is dragged. Opening at black, as every
    // analyzer preset does — and the ring's copy is a second key at any opening,
    // its silent end being pinned to the node's ground rather than floored at
    // it.
    let analyzer = Gradient { lightness: 50.0, lightness_ramp: 100.0, ..nth(3) };
    let ring = crate::ring_gradient(analyzer, ViewConfig::default().lattice_ground);
    assert_ne!(ring, analyzer.sanitized(), "the ring's table is the analyzer's own entry");
    // Ten frames of a drag, each writing a gradient one step further round the
    // circle — which is what a bar being dragged does.
    let dragged = |frame: u32| Gradient { hue_start: 200.0 + frame as f32, ..Gradient::default() };
    // Warm the state the drag begins from, so the count below is the steady
    // state rather than the arrival.
    gradient_color(0.5, dragged(0));
    gradient_color(0.5, ring);
    gradient_color(0.5, analyzer);
    let built = rebuilds(|| {
        for frame in 1..=10 {
            // The order a frame actually draws in: the lattice's nodes off the
            // value the bar wrote last frame, its audio ring, the Spectral
            // display's own curve, the roll's ribbons off the lattice's
            // gradient again, then the bar itself painting what it has just
            // written.
            gradient_color(0.5, dragged(frame - 1));
            gradient_color(0.5, ring);
            gradient_color(0.5, analyzer);
            gradient_color(0.5, dragged(frame - 1));
            gradient_color(0.5, dragged(frame));
        }
    });
    assert_eq!(
        built, 10,
        "a drag frame may rebuild the one table its own step invalidated and no \
         more; {built} builds over 10 frames means the pictures are evicting each \
         other",
    );
}

/// One more than fits evicts, rather than the cache growing without bound: the
/// policy is [`LUT_SLOTS`] slots, most recently used kept.
///
/// Written against the constant rather than against three named gradients, so
/// that changing the slot count moves this test's arithmetic with it instead of
/// leaving a test that pins a number the cache no longer holds to.
#[test]
fn one_gradient_too_many_evicts_the_stalest() {
    // A full cache, oldest first — so `held[0]` is the one next to go.
    let held: Vec<Gradient> = (0..LUT_SLOTS).map(|i| nth(20 + i as u32)).collect();
    for &g in &held {
        gradient_color(0.5, g);
    }
    for &g in &held {
        assert_eq!(rebuilds(|| { gradient_color(0.5, g); }), 0, "a full cache holds all of them");
    }
    // Asking for all of them in that order left `held[0]` the least recently
    // used, so it is the one the newcomer displaces.
    let extra = nth(20 + LUT_SLOTS as u32);
    assert_eq!(rebuilds(|| { gradient_color(0.5, extra); }), 1, "the newcomer is built");
    // The survivors BEFORE the evicted one, because asking for a gradient that
    // is gone rebuilds it — which evicts in its turn, and would leave this loop
    // measuring the damage its own previous line did.
    for &g in &held[1..] {
        assert_eq!(rebuilds(|| { gradient_color(0.5, g); }), 0, "the fresher ones stayed");
    }
    assert_eq!(rebuilds(|| { gradient_color(0.5, held[0]); }), 1, "and the stalest was dropped");
}

/// A level off the end of the range, or off the number line entirely, is drawn
/// at the nearest end rather than as a color nobody can see.
///
/// The non-finite case is the one worth a test rather than an argument, and it
/// is not a clamp: `f32::clamp` hands NaN straight back, `as usize` then
/// saturates the index to 0, and the lerp weight stays NaN — so the guard is
/// what stands between a NaN level and a NaN color riding out into the instance
/// buffer unannounced. Nothing upstream can produce one today, which is exactly
/// why the branch needs holding: the caller that could is the one not written
/// yet.
#[test]
fn a_level_off_the_range_lands_on_the_nearest_end() {
    let g = nth(7);
    let (bottom, top) = (gradient_color(0.0, g), gradient_color(1.0, g));
    for over in [1.0001f32, 2.0, 1e9, f32::INFINITY] {
        assert_eq!(gradient_color(over, g), top, "{over} is past the top of the range");
    }
    for under in [-0.0001f32, -1.0, f32::NEG_INFINITY] {
        assert_eq!(gradient_color(under, g), bottom, "{under} is under the bottom");
    }
    let nan = gradient_color(f32::NAN, g);
    assert_eq!(nan, bottom, "a NaN level must draw the bottom of the range");
    assert!(
        nan.is_finite(),
        "a NaN level rode out as {nan:?}, which nothing downstream would report",
    );
}

/// The hue circle's own memo answers the pair it is ASKED for, and not the one
/// it happens to be holding.
///
/// One slot and, in this tree, one caller — a spectrum bar's track, which hands
/// it two constants. That is what makes this worth pinning rather than
/// obvious: a memo that returned its entry without comparing the key would
/// serve the first circle ever built to everyone, and nothing else here would
/// notice, because nothing else asks for a second pair. The branch is there for
/// the second caller `hue_circle`'s doc invites, and this is what stands behind
/// it until one arrives.
///
/// Both knobs, because the key is a pair and half a key is a key that looks
/// right on the test that only moves the other half.
#[test]
fn the_hue_circle_answers_the_pair_it_is_asked_for() {
    let dark = hue_circle(30.0, 0.85);
    let light = hue_circle(70.0, 0.85);
    assert_ne!(dark, light, "two lightnesses drew one circle");
    let pale = hue_circle(30.0, 0.35);
    assert_ne!(pale, dark, "two chromas drew one circle");
    // Asked again, out of the order they were built in: each comes back as
    // itself rather than as whatever the slot last held.
    assert_eq!(hue_circle(70.0, 0.85), light, "the light circle came back changed");
    assert_eq!(hue_circle(30.0, 0.85), dark, "the dark circle came back changed");
    assert_eq!(hue_circle(30.0, 0.35), pale, "the pale circle came back changed");
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
