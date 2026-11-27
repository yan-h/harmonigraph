//! Per-surface ownership of the shared lattice animation.
use harmonigraph_scene::Scene;

pub(super) fn apply(scene: &mut Scene, state: &mut crate::PictureState, surface: usize, now: f64) {
    // Stepped on the scene's own view and its envelope, which the motion reads
    // for itself: a look handed in beside the scene could be a second one.
    let motion = state.surfaces.node_motion.entry(surface).or_default();
    motion.step(
        scene,
        &state.runtime.tracker,
        &state.runtime.tuning,
        &state.runtime.ring_fade,
        now,
    );
    let moving = &mut state.surfaces.moving_this_frame;
    moving.hold_if(crate::Reason::Releasing, motion.moving());
    // A departure that starts at its time with nothing moving before it.
    if let Some(at) = motion.off_due() {
        moving.hold_for(crate::Reason::Releasing, at - now);
    }
}
