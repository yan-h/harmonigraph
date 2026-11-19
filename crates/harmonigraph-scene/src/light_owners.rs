//! The light owners off a pane: positions outside its drawn window whose halo
//! still reaches the frame, chosen beside the rules that size and place a node
//! so that none of them is restated where the choice is made (#1861 R5).
//!
//! Selects geometry only, without an animation clock of its own: what a
//! position's light does is the motion's and the glow fade's, and the owners
//! are derived and stepped with the rest of the window.

use glam::Vec2;
use harmonigraph_core::{LatticePos, NoteTracker, PitchClass, Tuning};

use crate::derive::sheet_scale;
use crate::framing::billboard_corners;
use crate::{
    lattice_to_world, Camera, DrawnWindow, NodeMotion, ViewConfig, MAX_DRAWN_NODES, MIN_NODE_SCALE,
};

/// The positions outside `window` that light the frame at `now`, nearest the
/// eye first where more qualify than the node budget leaves room for.
///
/// A position qualifies when a note within the motion's replay horizon could
/// light it, or `owns` says a surface already carries light there — the
/// lattice motion's ink or marks ([`NodeMotion::owns_light`]) and the glow
/// fade's lights, which live in the UI crate and so arrive as a predicate —
/// and the billboard of its halo, sized as the renderer sizes it, overlaps the
/// frame.
#[allow(clippy::too_many_arguments)]
pub fn light_owners(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    camera: &Camera,
    window: &DrawnWindow,
    aspect: f32,
    owns: impl Fn(LatticePos) -> bool,
    now: f64,
) -> Vec<LatticePos> {
    let spare = MAX_DRAWN_NODES.saturating_sub(window.count());
    if spare == 0 || !now.is_finite() || !view.glow_draws() {
        return Vec::new();
    }
    let Some(bounds) = view.light_owner_window(camera, aspect) else { return Vec::new() };
    if window.contains(bounds.min) && window.contains(bounds.max) {
        return Vec::new();
    }
    let classes = NodeMotion::light_classes(tracker, view, now);
    let center = view.center();
    let transform = camera.view_proj(aspect);
    let axes = camera.right_up();
    let radius = view.light_owner_radius();
    let mut extra: Vec<_> = bounds
        .positions()
        .filter(|&pos| {
            if window.contains(pos) {
                return false;
            }
            let class = PitchClass::from_cents(tuning.pitch_class(pos).to_cents());
            // A nearest class is on either side in sorted octave order, wrapping
            // at the ends, just as the trail's remembered-class lookup does.
            let matches = if classes.is_empty() {
                false
            } else {
                let above = classes.partition_point(|&pitch| pitch < class);
                tuning.matches(classes[above % classes.len()], class)
                    || tuning.matches(classes[(above + classes.len() - 1) % classes.len()], class)
            };
            if !matches && !owns(pos) {
                return false;
            }
            // The halo's billboard at its sheet's size, under the floor the
            // shader puts on that size. A corner behind the eye is skipped
            // rather than disqualifying: the halo can still reach the frame
            // from the corners in front.
            let centered = pos - center;
            let r = radius * sheet_scale(view, centered).max(MIN_NODE_SCALE);
            let mut lo = Vec2::splat(f32::INFINITY);
            let mut hi = Vec2::splat(f32::NEG_INFINITY);
            for p in billboard_corners(transform, axes, lattice_to_world(centered), r) {
                if p.w > 0.0 && p.z >= 0.0 {
                    let projected = p.truncate().truncate() / p.w;
                    lo = lo.min(projected);
                    hi = hi.max(projected);
                }
            }
            hi.x >= -1.0 && hi.y >= -1.0 && lo.x <= 1.0 && lo.y <= 1.0
        })
        .collect();
    if extra.len() > spare {
        let eye = camera.eye();
        let distance = |p: &LatticePos| lattice_to_world(*p - center).distance_squared(eye);
        extra.sort_unstable_by(|a, b| distance(a).total_cmp(&distance(b)));
        extra.truncate(spare);
    }
    extra
}
