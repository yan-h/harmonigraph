//! Framing the lattice on the nodes a piece lights: which nodes those are,
//! where the camera has to stand to show all of them, and what shape of pane
//! holds them with nothing to spare.
//!
//! The Video pane's Frame notes button and the offline renderer's Frame
//! lattice to notes both come here, so a framing chosen live and one chosen at
//! render time are the same framing.

use glam::{Vec2, Vec3, Vec4};
use harmonigraph_core::{LatticePos, PitchClass, Tuning};

use crate::{lattice_to_world, Camera, ViewConfig, NODE_RADIUS_FACTOR};

/// How far past the rim of the outermost framed node the picture runs, in
/// lattice steps: half a step, so an edge node's name and halo have room and
/// the next node out is not half on screen.
const PAD: f32 = 0.5;

/// How far a played pitch is looked for from where the camera stands, in steps
/// along the fifths and the thirds. Past the zoomed-out window, which is about
/// twenty steps tall (see [`Camera::MAX_DISTANCE`]), so a node beyond it could
/// not have been framed with the rest anyway.
const REACH: i32 = 24;

/// Recentring and rescaling is exact in one pass under orthographic and
/// cabinet, whose screen position is affine in the camera; perspective
/// converges on it, and this many passes leave it well under a pixel.
const PASSES: usize = 8;

/// The node each of `classes` lights under `tuning`, the one nearest `near`
/// (absolute fifths and thirds) where several do, on the sheets `view` draws.
///
/// Several do under a tempered tuning, where a pitch class repeats across the
/// whole sheet, and under a just one wherever a comma falls inside the
/// tolerance. Every copy lights, but framing them all would frame the whole
/// sheet; the copy nearest the camera is the one already being looked at.
/// A class that lights no node in reach, a bend in passing or a pitch the
/// tuning has no node for, frames nothing, as it draws nothing.
///
/// Matching is [`Tuning::matches`], the test that lights a node.
pub fn lit_positions(
    classes: &[PitchClass],
    tuning: &Tuning,
    view: &ViewConfig,
    near: Vec2,
) -> Vec<LatticePos> {
    let mut classes = classes.to_vec();
    classes.sort_unstable();
    classes.dedup();
    let n = classes.len();
    if n == 0 || !near.is_finite() {
        return Vec::new();
    }
    let (low, high) = view.sevens_window();
    let (fives, threes) = (near.x.round() as i32, near.y.round() as i32);
    let mut best: Vec<Option<(f32, LatticePos)>> = vec![None; n];
    for sevens in low..=high {
        for t in threes - REACH..=threes + REACH {
            for f in fives - REACH..=fives + REACH {
                let pos = LatticePos::new(t, f, sevens);
                let node = tuning.pitch_class(pos);
                let d = Vec3::new(
                    f as f32 - near.x,
                    t as f32 - near.y,
                    (sevens - view.center_sevens) as f32,
                )
                .length_squared();
                // Sorted and unique, so the classes this node matches are one
                // run either side of where it would sort, wrapping at the
                // octave. Up from there and then down, never visiting one twice.
                let at = classes.partition_point(|&c| c < node);
                let mut take = |i: usize| {
                    if best[i].is_none_or(|(nearest, _)| d < nearest) {
                        best[i] = Some((d, pos));
                    }
                };
                let mut up = 0;
                while up < n && tuning.matches(classes[(at + up) % n], node) {
                    take((at + up) % n);
                    up += 1;
                }
                let mut down = 1;
                while up + down <= n && tuning.matches(classes[(at + n - down) % n], node) {
                    take((at + n - down) % n);
                    down += 1;
                }
            }
        }
    }
    let mut positions: Vec<_> = best.into_iter().flatten().map(|(_, pos)| pos).collect();
    positions.sort_unstable_by_key(crate::derive::lattice_order);
    positions.dedup();
    positions
}

/// A camera that frames a set of nodes, and the shape of what it frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framed {
    pub camera: Camera,
    /// The framed nodes' width over their height on screen, rims and padding
    /// included: the aspect of the pane they would fill exactly.
    pub aspect: f32,
    /// False where the nodes need the camera further back than
    /// [`Camera::MAX_DISTANCE`] lets it go, so the ones at the edges are cut
    /// off. A piece too small to fill the pane at the nearest distance, one
    /// node say, still fits.
    pub fits: bool,
}

/// Move `camera`'s target and distance so every one of `positions` is on a
/// pane of `aspect`, filling it along one axis. Its orbit and projection are
/// kept: framing is pan and zoom, the two moves done by hand.
///
/// The target stays on the home sheet, as the camera's pan parameters keep it,
/// and is relative to `view`'s center like every camera target. `None` for no
/// positions.
pub fn frame(
    camera: Camera,
    view: &ViewConfig,
    positions: &[LatticePos],
    aspect: f32,
) -> Option<Framed> {
    if positions.is_empty() || !(aspect.is_finite() && aspect > 0.0) {
        return None;
    }
    let center = view.center();
    let outer = view.rings().outer * NODE_RADIUS_FACTOR;
    let nodes: Vec<(Vec3, f32)> = positions
        .iter()
        .map(|&pos| {
            let centered = pos - center;
            // Drawn at the size `derive_scene` gives the node's sheet.
            let size = view.sevens_size.powi(centered.sevens.unsigned_abs() as i32);
            (lattice_to_world(centered), outer * size + PAD)
        })
        .collect();
    let (lo, hi) = nodes
        .iter()
        .fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(lo, hi), (p, _)| {
            (lo.min(p.truncate()), hi.max(p.truncate()))
        });
    let mut camera = camera;
    camera.target = ((lo + hi) * 0.5).extend(0.0);
    camera.distance = camera.distance.clamp(Camera::MIN_DISTANCE, Camera::MAX_DISTANCE);
    for _ in 0..PASSES {
        let (lo, hi) = extent(&camera, &nodes, aspect)?;
        let middle = (lo + hi) * 0.5;
        // How far the picture moves per step of target along each sheet axis.
        // Moving the target is moving every node the other way, so this is
        // where one step from the target lands.
        let to_ndc = |p: Vec3| {
            let clip = camera.view_proj(aspect) * p.extend(1.0);
            clip.truncate().truncate() / clip.w
        };
        let x = to_ndc(camera.target + Vec3::X);
        let y = to_ndc(camera.target + Vec3::Y);
        let det = x.x * y.y - y.x * x.y;
        // Edge-on to the sheet, where no pan in it moves the picture sideways;
        // the zoom below still applies.
        if det.abs() > 1e-6 {
            let step = Vec2::new(
                (middle.x * y.y - y.x * middle.y) / det,
                (x.x * middle.y - middle.x * x.y) / det,
            );
            camera.target += step.extend(0.0);
        }
        let reach = ((hi - lo) * 0.5).max_element();
        if !(reach.is_finite() && reach > 0.0) {
            return None;
        }
        camera.distance =
            (camera.distance * reach).clamp(Camera::MIN_DISTANCE, Camera::MAX_DISTANCE);
    }
    let (lo, hi) = extent(&camera, &nodes, aspect)?;
    let half = (hi - lo) * 0.5;
    Some(Framed {
        camera,
        aspect: half.x / half.y * aspect,
        fits: half.max_element() <= 1.0 + 1e-3,
    })
}

/// The NDC rectangle the nodes' billboards cover, each `(center, radius)`
/// facing the camera as a node's quad does. Points behind the eye are left
/// out: perspective cannot frame them and they draw nothing.
fn extent(camera: &Camera, nodes: &[(Vec3, f32)], aspect: f32) -> Option<(Vec2, Vec2)> {
    let transform = camera.view_proj(aspect);
    let (right, up) = camera.right_up();
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for &(p, r) in nodes {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let clip: Vec4 = transform * (p + (right * x + up * y) * r).extend(1.0);
            if clip.w > 0.0 && clip.z >= 0.0 {
                let ndc = clip.truncate().truncate() / clip.w;
                lo = lo.min(ndc);
                hi = hi.max(ndc);
            }
        }
    }
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Projection;

    fn view() -> ViewConfig {
        let mut view = ViewConfig::default();
        view.sanitize();
        view
    }

    fn piece() -> Vec<LatticePos> {
        // A spread wider than tall, off the origin, across two sheets.
        vec![
            LatticePos::new(-1, -3, 0),
            LatticePos::new(2, 4, 0),
            LatticePos::new(0, 1, 1),
            LatticePos::new(1, -2, 0),
        ]
    }

    /// Every node's rim is on screen and the tighter axis is filled, under all
    /// three projections, at a wide and a tall pane.
    #[test]
    fn a_framed_piece_fills_the_pane_along_one_axis_and_stays_inside_it() {
        let mut view = view();
        view.max_sevens = 1;
        let center = view.center();
        let outer = view.rings().outer * NODE_RADIUS_FACTOR;
        for projection in [Projection::Cabinet, Projection::Orthographic, Projection::Perspective] {
            for aspect in [0.5, 1.0, 2.4] {
                let camera = Camera { projection, ..Camera::default() };
                let framed = frame(camera, &view, &piece(), aspect).unwrap();
                assert!(framed.fits, "{projection:?} at {aspect}");
                let nodes: Vec<_> = piece()
                    .iter()
                    .map(|&p| {
                        let c = p - center;
                        let size = view.sevens_size.powi(c.sevens.unsigned_abs() as i32);
                        (lattice_to_world(c), outer * size + PAD)
                    })
                    .collect();
                let (lo, hi) = extent(&framed.camera, &nodes, aspect).unwrap();
                let tolerance = if projection == Projection::Perspective { 1e-2 } else { 1e-4 };
                assert!(lo.min_element() >= -1.0 - tolerance, "{projection:?} {aspect}: {lo}");
                assert!(hi.max_element() <= 1.0 + tolerance, "{projection:?} {aspect}: {hi}");
                let fill = ((hi - lo) * 0.5).max_element();
                assert!((fill - 1.0).abs() < tolerance, "{projection:?} {aspect}: fills {fill}");
                // Centred on the axis it fills, and on the other one too.
                assert!(((lo + hi) * 0.5).abs().max_element() < tolerance);
                assert_eq!(framed.camera.target.z, 0.0, "the target stays on the home sheet");
            }
        }
    }

    /// The shape reported is the shape that fills: a pane of that aspect is
    /// filled along both axes at once.
    #[test]
    fn a_pane_at_the_framed_aspect_is_filled_both_ways() {
        let view = view();
        let flat: Vec<_> = piece().into_iter().filter(|p| p.sevens == 0).collect();
        let camera = Camera::default();
        let wide = frame(camera, &view, &flat, 16.0 / 9.0).unwrap();
        assert!(wide.aspect > 1.0, "the piece is wider than tall: {}", wide.aspect);
        let exact = frame(camera, &view, &flat, wide.aspect).unwrap();
        let (lo, hi) = extent(
            &exact.camera,
            &flat
                .iter()
                .map(|&p| {
                    (
                        lattice_to_world(p - view.center()),
                        view.rings().outer * NODE_RADIUS_FACTOR + PAD,
                    )
                })
                .collect::<Vec<_>>(),
            wide.aspect,
        )
        .unwrap();
        assert!((hi - lo - Vec2::splat(2.0)).abs().max_element() < 1e-3, "{lo} {hi}");
    }

    /// A piece bigger than the zoomed-out window stops at the limit and says so.
    #[test]
    fn a_piece_past_the_zoom_limit_is_framed_as_far_as_it_goes() {
        let far = [LatticePos::new(0, -40, 0), LatticePos::new(0, 40, 0)];
        let framed = frame(Camera::default(), &view(), &far, 1.0).unwrap();
        assert_eq!(framed.camera.distance, Camera::MAX_DISTANCE);
        assert!(!framed.fits);
        assert!(frame(Camera::default(), &view(), &[], 1.0).is_none());
    }

    /// Under a just tuning each played class is its own node, wherever it is;
    /// under 12-TET each is the copy nearest the camera; a pitch between nodes
    /// is nowhere.
    #[test]
    fn played_pitches_land_on_the_nodes_they_light() {
        let view = view();
        let just = Tuning::just();
        let piece =
            [LatticePos::new(-1, -3, 0), LatticePos::new(2, 4, 0), LatticePos::new(5, 0, 0)];
        let classes: Vec<_> = piece.iter().map(|&p| just.pitch_class(p)).collect();
        let mut expected = piece.to_vec();
        expected.sort_unstable_by_key(crate::derive::lattice_order);
        assert_eq!(lit_positions(&classes, &just, &view, Vec2::ZERO), expected);

        let tempered = Tuning::default();
        let e = PitchClass::from_midi_note(64);
        let near = Vec2::new(10.0, 0.0);
        let found = lit_positions(&[e], &tempered, &view, near);
        assert_eq!(found.len(), 1);
        assert!(tempered.matches(e, tempered.pitch_class(found[0])));
        // Twelve fifths and three thirds both return an E; nothing else that
        // lights it is nearer (10, 0) than the one found.
        let d = |p: LatticePos| Vec2::new(p.fives as f32, p.threes as f32).distance(near);
        for t in -12..=12 {
            for f in -2..=22 {
                let p = LatticePos::new(t, f, 0);
                if tempered.matches(e, tempered.pitch_class(p)) {
                    assert!(d(p) >= d(found[0]), "{p:?} is nearer than {:?}", found[0]);
                }
            }
        }

        let between = PitchClass::from_cents(450.0);
        assert!(lit_positions(&[between], &tempered, &view, Vec2::ZERO).is_empty());
    }
}
