//! Framing the lattice on the nodes a piece lights: which nodes those are,
//! where the camera has to stand to show all of them, and what shape of pane
//! holds them with nothing to spare.
//!
//! What the Video pane's Frame notes button does, in geometry.

use glam::{Vec2, Vec3, Vec4};
use harmonigraph_core::{LatticePos, PitchClass, Tuning};

use crate::derive::marker_world;
use crate::{lattice_to_world, Camera, ViewConfig};

/// How far past the rim of the outermost framed node the picture runs, in
/// lattice steps: half a step, so the next node out is not half on screen.
/// A wide glow is left to run off the edge, as it does beside any neighbour.
const PAD: f32 = 0.5;

/// Recentre-then-zoom rounds. Under orthographic and cabinet, whose screen
/// position is affine in the camera, the first is exact; under perspective,
/// recentring at one distance shifts the centre a little at another, and the
/// rounds close in on it.
const PASSES: usize = 16;

/// Halvings of the distance range per zoom: past the f32 resolution of a
/// distance in `MIN_DISTANCE..=MAX_DISTANCE`.
const HALVINGS: usize = 32;

/// The node each of `classes` lights under `tuning`, the one nearest `near`
/// (absolute fifths and thirds) where several do, within the naming reach
/// ([`ViewConfig::reach`]): the block a played pitch is hunted through for a
/// name and a node.
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
    if !near.is_finite() {
        return Vec::new();
    }
    let nodes: Vec<_> = view
        .reach()
        .positions()
        .map(|pos| {
            let offset = Vec3::new(
                pos.fives as f32 - near.x,
                pos.threes as f32 - near.y,
                (pos.sevens - view.center_sevens) as f32,
            );
            (pos, tuning.pitch_class(pos), offset.length_squared())
        })
        .collect();
    let mut positions: Vec<_> = classes
        .iter()
        .filter_map(|&class| {
            nodes
                .iter()
                .filter(|&&(_, node, _)| tuning.matches(class, node))
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|&(pos, ..)| pos)
        })
        .collect();
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
/// and is relative to `view`'s center like every camera target. `None` only
/// for no positions (or a pane with no shape).
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
    let nodes: Vec<(Vec3, f32)> = positions
        .iter()
        .map(|&pos| {
            let (world, rim) = footprint(view, pos - center);
            (world, rim + PAD)
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
    let fits = |camera: &Camera| {
        extent(camera, &nodes, aspect)
            .is_some_and(|(lo, hi)| lo.min_element() >= -1.0 && hi.max_element() <= 1.0)
    };
    // Recentring at one distance moves the picture a little at another under
    // perspective, so the rounds alternate, and the best-centred camera that
    // fits is the answer: a round can only make things worse where the view
    // is all but edge-on to the sheet.
    let mut best: Option<(f32, Camera)> = None;
    for _ in 0..PASSES {
        if let Some(target) = recentred(&camera, &nodes, aspect) {
            camera.target = target;
        }
        // The nearest distance that holds every node. Pulling the eye back
        // along its own axis shrinks every node's offset from the middle of
        // the screen, under every projection, so whether it fits is monotone
        // in distance and a bisection finds the edge exactly. Scaling the
        // distance by the overshoot does too, but only where the screen is
        // affine in it: under perspective a node nearer the eye shrinks
        // faster than one farther off, and the scaled step overshoots, or
        // walks past a node behind the eye that `extent` cannot see.
        let at = |distance: f32| Camera { distance, ..camera };
        // A piece that fits even at the nearest distance ends there, the
        // bisection closing on the near end.
        camera.distance = if !fits(&at(Camera::MAX_DISTANCE)) {
            Camera::MAX_DISTANCE
        } else {
            let (mut near, mut far) = (Camera::MIN_DISTANCE, Camera::MAX_DISTANCE);
            for _ in 0..HALVINGS {
                let mid = 0.5 * (near + far);
                if fits(&at(mid)) {
                    far = mid;
                } else {
                    near = mid;
                }
            }
            far
        };
        if let Some((lo, hi)) = extent(&camera, &nodes, aspect).filter(|_| fits(&camera)) {
            let off = ((lo + hi) * 0.5).abs().max_element();
            if best.is_none_or(|(nearest, _)| off < nearest) {
                best = Some((off, camera));
            }
        }
    }
    let camera = best.map_or(camera, |(_, camera)| camera);
    let shape = match extent(&camera, &nodes, aspect) {
        Some((lo, hi)) => {
            let half = (hi - lo) * 0.5;
            half.x / half.y * aspect
        }
        // Too big to show whole even at the farthest distance, with a node
        // left behind the eye: the sheet's own shape is the best guess.
        None => {
            let rim = nodes.iter().map(|&(_, r)| r).fold(0.0, f32::max);
            let size = hi - lo + Vec2::splat(2.0 * rim);
            size.x / size.y
        }
    };
    Some(Framed { camera, aspect: shape, fits: fits(&camera) })
}

/// Where on the home sheet to stand for the nodes to sit in the middle of
/// the screen: a Newton step on the middle of what they cover, as a function
/// of the target, halved until it improves on where the camera stands.
/// `None` where no pan in the sheet moves the picture sideways (a camera
/// edge-on to it) or none helps.
///
/// Solved against the nodes' own middle rather than how the target's
/// neighbourhood moves on screen, because the two differ wherever depth does:
/// under perspective a nearer node moves further for one step of pan, so a
/// node off the home sheet, or a tilted view of the home sheet itself, sent a
/// step solved at the target's depth wide of the middle and sometimes far down
/// the view axis. Under orthographic and cabinet the middle is affine in the
/// target, and the first step is exact.
fn recentred(camera: &Camera, nodes: &[(Vec3, f32)], aspect: f32) -> Option<Vec3> {
    // Where the middle falls is the same at every distance under the affine
    // projections, so they find it from the farthest, where a pan that
    // changes a node's depth cannot take it behind the near plane.
    let distance = match camera.projection {
        crate::Projection::Perspective => camera.distance,
        crate::Projection::Orthographic | crate::Projection::Cabinet => Camera::MAX_DISTANCE,
    };
    let middle = |target: Vec3| {
        let (lo, hi) = extent(&Camera { target, distance, ..*camera }, nodes, aspect)?;
        Some((lo + hi) * 0.5)
    };
    let here = middle(camera.target)?;
    let dx = middle(camera.target + Vec3::X)? - here;
    let dy = middle(camera.target + Vec3::Y)? - here;
    let det = dx.x * dy.y - dy.x * dx.y;
    if det.abs() < 1e-6 {
        return None;
    }
    // Solve [dx dy] · step = -here.
    let mut step = Vec3::new(
        (dy.x * here.y - here.x * dy.y) / det,
        (here.x * dx.y - dx.x * here.y) / det,
        0.0,
    );
    for _ in 0..HALVINGS {
        let target = camera.target + step;
        if middle(target).is_some_and(|m| m.abs().max_element() < here.abs().max_element()) {
            return Some(target);
        }
        step *= 0.5;
    }
    None
}

/// Where a node `centered` steps from the window center draws, and how far its
/// outermost ring reaches in world units, at the size `derive_scene` gives the
/// node's sheet.
fn footprint(view: &ViewConfig, centered: LatticePos) -> (Vec3, f32) {
    let size = view.sevens_size.powi(centered.sevens.unsigned_abs() as i32);
    (lattice_to_world(centered), marker_world(view.rings().outer) * size)
}

/// The NDC rectangle the nodes' billboards cover, each `(center, radius)`
/// facing the camera as a node's quad does. `None` where any corner is behind
/// the eye or nearer than the near plane: that node cannot be on screen at
/// this distance, which is not the same as being inside it.
fn extent(camera: &Camera, nodes: &[(Vec3, f32)], aspect: f32) -> Option<(Vec2, Vec2)> {
    let transform = camera.view_proj(aspect);
    let (right, up) = camera.right_up();
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for &(p, r) in nodes {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let clip: Vec4 = transform * (p + (right * x + up * y) * r).extend(1.0);
            if clip.w <= 0.0 || clip.z < 0.0 {
                return None;
            }
            let ndc = clip.truncate().truncate() / clip.w;
            lo = lo.min(ndc);
            hi = hi.max(ndc);
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

    /// Each node's center and the reach of its padded rim, measured as the
    /// scene draws it: the ring stack's outer edge is in quad uv, which
    /// `derive`'s own `marker_world` turns into world units, and an off-sheet
    /// node is drawn smaller by its sheet's scale.
    fn drawn(view: &ViewConfig, positions: &[LatticePos]) -> Vec<(Vec3, f32)> {
        let rim = crate::derive::marker_world(view.rings().outer);
        positions
            .iter()
            .map(|&p| {
                let c = p - view.center();
                let size = view.sevens_size.powi(c.sevens.unsigned_abs() as i32);
                (Vec3::new(c.fives as f32, c.threes as f32, c.sevens as f32), rim * size + PAD)
            })
            .collect()
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

    /// Every node's rim is on screen, one edge is touched and the piece is
    /// centred, under all three projections, at a wide and a tall pane, from
    /// a straight-on orbit and an oblique one. The oblique perspective camera
    /// is the one a scaled zoom overshot by 5% and collapsed to the nearest
    /// distance on.
    #[test]
    fn a_framed_piece_fills_the_pane_along_one_axis_and_stays_inside_it() {
        let mut view = view();
        view.max_sevens = 1;
        let nodes = drawn(&view, &piece());
        for projection in [Projection::Cabinet, Projection::Orthographic, Projection::Perspective] {
            for (yaw, pitch) in [(0.4, 0.3), (1.2, 0.0), (0.4, 0.9)] {
                for aspect in [0.5, 1.0, 2.4] {
                    let camera = Camera { projection, yaw, pitch, ..Camera::default() };
                    let at = format!("{projection:?} yaw {yaw} pitch {pitch} at {aspect}");
                    let framed = frame(camera, &view, &piece(), aspect).unwrap();
                    assert!(framed.fits, "{at}");
                    let (lo, hi) = extent(&framed.camera, &nodes, aspect).unwrap();
                    assert!(lo.min_element() >= -1.0 && hi.max_element() <= 1.0, "{at}: {lo} {hi}");
                    let edge = (-lo.min_element()).max(hi.max_element());
                    assert!(edge > 1.0 - 1e-4, "{at}: reaches only {edge}");
                    assert!(((lo + hi) * 0.5).abs().max_element() < 1e-3, "{at}: {lo} {hi}");
                    assert_eq!(framed.camera.target.z, 0.0, "the target stays on the home sheet");
                }
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
        let (lo, hi) = extent(&exact.camera, &drawn(&view, &flat), wide.aspect).unwrap();
        assert!((hi - lo - Vec2::splat(2.0)).abs().max_element() < 1e-3, "{lo} {hi}");
    }

    /// Views that graze the sheet, where a pan solved at the target's own
    /// depth was ill-conditioned: under perspective it left the piece a tenth
    /// of the screen off centre, and with a node off the home sheet it sent
    /// the target down the view axis until nothing fitted and no framing came
    /// back at all. That last one is edge-on to the sevens axis, which is the
    /// screen's horizontal there, so no pan on the home sheet can centre its
    /// off-sheet node; it is held to fitting.
    #[test]
    fn a_grazing_view_still_frames_and_centres() {
        let mut stacked = view();
        (stacked.min_sevens, stacked.max_sevens) = (-1, 1);
        let p = LatticePos::new;
        let cases = [
            (
                Projection::Perspective,
                2.33,
                -0.77,
                2.0,
                view(),
                vec![p(2, -2, 0), p(-1, 1, 0), p(0, -1, 0), p(2, 0, 0)],
            ),
            (Projection::Perspective, 1.68, 0.53, 6.0, view(), vec![p(-5, 5, 0), p(-6, -4, 0)]),
            (
                Projection::Perspective,
                1.61,
                1.44,
                24.0,
                view(),
                vec![p(-1, 4, 0), p(0, 2, 0), p(-4, 4, 0), p(2, -1, 0)],
            ),
            (Projection::Orthographic, -1.57, 0.32, 6.0, stacked, vec![p(-4, -1, 1)]),
        ];
        for (projection, yaw, pitch, distance, view, piece) in cases {
            let camera = Camera { projection, yaw, pitch, distance, ..Camera::default() };
            let at = format!("{projection:?} yaw {yaw} pitch {pitch}");
            let framed = frame(camera, &view, &piece, 16.0 / 9.0).expect(&at);
            assert!(framed.fits, "{at}");
            let (lo, hi) = extent(&framed.camera, &drawn(&view, &piece), 16.0 / 9.0).unwrap();
            if projection == Projection::Perspective {
                assert!(((lo + hi) * 0.5).abs().max_element() < 1e-2, "{at}: {lo} {hi}");
            }
        }
    }

    /// A piece bigger than the zoomed-out window stops at the limit and says
    /// so, under every projection, even where some of it is behind the eye;
    /// one node fits, nearer.
    #[test]
    fn a_piece_frames_from_one_node_to_past_the_zoom_limit() {
        let far = [LatticePos::new(0, -40, 0), LatticePos::new(0, 40, 0)];
        for projection in [Projection::Cabinet, Projection::Orthographic, Projection::Perspective] {
            let camera = Camera { projection, pitch: 0.9, ..Camera::default() };
            let framed = frame(camera, &view(), &far, 1.0).expect("framed as far as it goes");
            assert_eq!(framed.camera.distance, Camera::MAX_DISTANCE, "{projection:?}");
            assert!(!framed.fits, "{projection:?}");

            let one = frame(camera, &view(), &[LatticePos::new(2, 3, 0)], 1.0).unwrap();
            assert!(one.camera.distance < framed.camera.distance, "{projection:?}");
            assert!(one.fits, "{projection:?}");
        }
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

        // Pitches inside the tolerance but off the node: E a fifth of a cent
        // flat is found below where E sorts, and C a fifth of a cent flat is
        // found across the octave's wrap, below 0.
        let off = [399.8, 700.0, 100.0, 1199.8].map(PitchClass::from_cents);
        let found = lit_positions(&off, &tempered, &view, Vec2::ZERO);
        assert_eq!(found.len(), 4, "{found:?}");
        for class in off {
            assert!(
                found.iter().any(|&p| tempered.matches(class, tempered.pitch_class(p))),
                "{class:?} lights none of {found:?}"
            );
        }
    }
}
