//! The halo's footprint, pinned to numbers worked by hand rather than to the
//! formula the renderer ships.

use crate::{glow_span, ViewConfig};

fn near(got: f32, want: f32) -> bool {
    (got - want).abs() < 1e-6
}

/// The widest rim is the mark strip's wherever marks draw, the rings'
/// otherwise, whether or not any node wears a mark; Reach goes on top, and
/// the whole is never under 0.1.
#[test]
fn the_glow_span_is_the_widest_drawn_rim_plus_reach_floored() {
    // Marks on and reaching past the rings: their outer edge is the rim.
    assert!(near(glow_span(1.0, 1.1, 0.3, 0.5), 1.9));
    // Marks on but inside the rings: the rings' edge is.
    assert!(near(glow_span(1.2, 1.1, 0.05, 0.5), 1.7));
    // Marks off: their slot counts for nothing, however far out it starts.
    assert!(near(glow_span(1.0, 1.1, 0.0, 0.5), 1.5));
    // A negative Reach reaches nothing.
    assert!(near(glow_span(1.0, 0.0, 0.0, -1.0), 1.0));
    // Nothing drawn and no Reach, or very little of both: the floor.
    assert_eq!(glow_span(0.0, 0.0, 0.0, 0.0), 0.1);
    assert_eq!(glow_span(0.03, 0.0, 0.0, 0.02), 0.1);
}

/// The off-pane light selector's bound is that span in world units: a node
/// radius of 0.25 lattice steps, and 1.8 radii to the uv.
#[test]
fn a_light_owners_radius_is_the_span_in_world_units() {
    let world = |span: f32| span * 0.25 * 1.8;
    let marked = ViewConfig { glow_reach: 0.5, mark_thickness: 0.3, ..ViewConfig::default() };
    let rings = marked.rings();
    assert!(rings.mark_inner + 0.3 > rings.outer, "the fixture's marks reach past the rings");
    let want = world(rings.mark_inner + 0.3 + 0.5);
    assert!(near(marked.light_owner_radius(), want), "{} {want}", marked.light_owner_radius());
    // Unmarked, the rings' own edge, though the mark slot lies past it.
    let unmarked = ViewConfig { mark_thickness: 0.0, ..marked.clone() };
    let want = world(unmarked.rings().outer + 0.5);
    assert!(near(unmarked.light_owner_radius(), want), "{} {want}", unmarked.light_owner_radius());
    // Nothing drawn and no Reach: the floor.
    let bare = ViewConfig {
        glow_reach: 0.0,
        mark_thickness: 0.0,
        band_width: 0.0,
        spectral_ring_width: 0.0,
        ..ViewConfig::default()
    };
    assert_eq!(bare.rings().outer, 0.0, "the fixture draws no ring");
    assert!(near(bare.light_owner_radius(), world(0.1)), "{}", bare.light_owner_radius());
}
