//! Per-surface ownership of the shared lattice animation.
use harmonigraph_scene::Scene;

pub(super) fn apply(scene: &mut Scene, state: &mut crate::PictureState, surface: usize, now: f64) {
    // The pane's own envelope, assembled where every other one is
    // (`ViewConfig::envelope`) rather than rebuilt from the two halves here.
    // The shape is a blob field and the duration a host parameter, and the
    // single assembly point is what keeps a caller from pairing one with the
    // wrong other — the Fade bar's preview is the one exception the doc there
    // names, and this was a second, undeclared one.
    let env = state.appearance.view.envelope(&state.runtime.frame_params);
    state.surfaces.node_motion.entry(surface).or_default().step(
        scene,
        &state.runtime.tracker,
        &state.runtime.tuning,
        &state.appearance.view,
        &env,
        now,
    );
}
