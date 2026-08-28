//! Note names: which one draws, over what, and in which order.

use super::fixtures::*;
use crate::gpu_harness::{headless_device, readback, render_to_texture};
use crate::*;

/// Where the pair of nodes below stands, in world units. Off-center in both
/// axes on purpose: a label is drawn into the pane's own pass, and a mapping
/// that flipped or transposed that pane would still land on the right pixel
/// in the middle of the picture.
const STACK_AT: glam::Vec2 = glam::Vec2::new(0.7, -0.5);

/// The pane this scene is drawn into. Bigger than the text fixtures, because
/// this one is about a node's RINGS rather than about a glyph: at the real
/// ratio of node radius to lattice spacing, 64 points across puts the whole
/// node inside a couple of pixels.
const SCENE_SIZE: [u32; 2] = [256, 256];

/// Two nodes on the same pixels, one sevens step apart, and nothing else.
/// Face-on and orthographic, which is the arrangement that puts one node
/// squarely behind another; a sheared or orbited view only spreads the same
/// overlap out.
///
/// Node 0 is the NEARER of the two, so it is the one drawn last.
fn one_node_behind_another() -> Scene {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // Big enough that the octave band — the one thing this node paints — is
    // several pixels deep and reaches full coverage in the middle of itself.
    // A band's two soft edges are screen-constant, so on a small node they
    // overlap and the annulus never becomes opaque, which is the one property
    // the covering claim below is read off.
    scene.node_radius = 0.5;
    let mut near = scene.nodes[0];
    near.world_pos = STACK_AT.extend(0.0);
    near.activation = 1.0;
    near.scale = 1.0;
    near.hovered = false;
    near.on_home = true;
    let mut far = near;
    far.world_pos = STACK_AT.extend(-1.0);
    far.on_home = false;
    scene.nodes = vec![near, far];
    // The markers would draw across the same pixels, and whether ONE covers a
    // label is a separate question that this fixture cannot answer twice.
    scene.pluses.clear();
    scene
}

/// A node in front covers the label of the node behind it, the way it covers
/// that node itself — the LETTERS of that label and its drawn marks alike.
///
/// This is the whole feature in one picture, and it is end to end on purpose:
/// the name is drawn inside the lattice's own scene pass, at its node's place
/// in the back-to-front order, so what covers a name is whatever the pass
/// draws after it. Nothing short of running that pass checks it.
///
/// Read as three renders of one scene — the picture with no label, with the
/// FAR node's name, and with the same glyph on the NEAR node, which puts it
/// after everything. The third is what makes the first assertion mean
/// something: a mapping that put the glyph anywhere but on the node would
/// leave the "covered" picture looking exactly right and this one looking
/// blank.
///
/// Run twice over: once for a letter off egui's atlas, once for a drawn mark
/// off the marks' own sheet. The second is #207 — a mark that reaches this
/// pass is covered by the arithmetic that covers everything else in it, so
/// there is nothing mark-shaped in the assertions, and that is the claim. A
/// mark on the painter instead passes none of them: it is drawn over the
/// finished picture, so the disc in front of it never gets a chance.
#[test]
fn a_nearer_node_covers_the_label_of_the_node_behind() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let projector = scene.projector(glam::Vec2::new(points.x, points.y));

    // The pixel both discs are painted on, which is where the labels go and
    // where they are read back.
    let on =
        projector.project(scene.nodes[0].world_pos).expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());
    assert!(
        (x - points.x / 2.0).abs() > 8.0 && (y - points.y / 2.0).abs() > 8.0,
        "the fixture's nodes must sit off-center, at ({x}, {y}) of {points:?}",
    );

    // One glyph, `off` points to the right of that pixel, named by `node`. No
    // standoff: the fill alone answers the question, and a shadow would spread
    // the reading over pixels nothing is being asked about.
    let picture = |instance: GlyphInstance, off: f32, label: Option<u32>| -> Vec<u8> {
        let (glyphs, labels) = match label {
            Some(node) => (
                vec![GlyphInstance { rect: [x + off - 4.0, y - 4.0, 8.0, 8.0], ..instance }],
                vec![Label { node, glyphs: 1 }],
            ),
            None => (Vec::new(), Vec::new()),
        };
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                node_points: 0.0,
                atlas: Some(crate::text::tests::atlas()),
                marks: Some(crate::text::tests::mark_sheet()),
                slide: SlideAxis::default(),
            },
            points,
            format,
            9,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let texture = render_to_texture(
            &device,
            &queue,
            SCENE_SIZE,
            format,
            wgpu::Color::TRANSPARENT,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SCENE_SIZE,
                    },
                    pass,
                    &resources,
                );
            },
        );
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    // What the probe pixel reads. The glyph is white and everything under it
    // is not, so one channel is the whole of what "how much name is here"
    // means.
    const NEAR: u32 = 0;
    const FAR: u32 = 1;
    /// Points out from the node's centre to the middle of its octave band,
    /// where the annulus is opaque; to that band's own fading edge; and to
    /// bare quad past every ring, where the node paints nothing at all.
    const ON_RING: f32 = 15.0;
    const ON_EDGE: f32 = 18.0;
    const PAST_RING: f32 = 30.0;

    for (what, instance) in
        [("a letter", crate::text::tests::glyph()), ("a drawn mark", crate::text::tests::mark())]
    {
        let at = |off: f32, label: Option<u32>| -> u8 {
            let frame = picture(instance, off, label);
            let i = (((y as u32) * SCENE_SIZE[0] + (x + off) as u32) * 4) as usize;
            frame[i + 1]
        };

        // On the octave band, which is opaque: the far node's name is gone,
        // exactly gone — this is compositing, not a mask, so "under an opaque
        // ring" is the picture with no label in it at all.
        let (bare_ring, under, over) =
            (at(ON_RING, None), at(ON_RING, Some(FAR)), at(ON_RING, Some(NEAR)));
        assert_eq!(under, bare_ring, "{what} under an opaque ring must leave no trace of itself",);
        assert!(
            over.abs_diff(bare_ring) > 32,
            "{what} drawn after the ring must be plainly visible on it, \
             got {over} against a bare ring's {bare_ring} — if these agree the \
             glyph is not landing on the node and the assertion above is vacuous",
        );

        // Across the band's own fading edge, where the difference between
        // covering and cutting shows: the name dims by exactly what the ring
        // took, rather than being taken out whole or left alone.
        let (bare_edge, under_edge, over_edge) =
            (at(ON_EDGE, None), at(ON_EDGE, Some(FAR)), at(ON_EDGE, Some(NEAR)));
        assert!(
            under_edge > bare_edge && under_edge < over_edge,
            "over the ring's fading edge {what} must dim rather than vanish: \
             {under_edge} against {bare_edge} bare and {over_edge} drawn on top",
        );

        // And past every ring the node draws — still inside its own quad, and
        // painted with nothing at all — a name is left exactly alone. A node
        // covers what it PAINTS and no more of its billboard than that.
        let (bare_out, under_out, over_out) =
            (at(PAST_RING, None), at(PAST_RING, Some(FAR)), at(PAST_RING, Some(NEAR)));
        assert!(
            over_out.abs_diff(bare_out) > 32,
            "the probe past the rings must be somewhere {what} shows at all: {over_out} \
             against {bare_out}",
        );
        assert!(
            under_out.abs_diff(over_out) <= 3,
            "past every ring it draws, a node must leave {what} alone: {under_out} \
             against {over_out} drawn on top",
        );
    }
}

/// A label is drawn immediately after the node it names, counted over the
/// instances that actually SHIP.
///
/// Two things that are easy to get right by accident and wrong in the
/// picture. The cull drops a node that can paint nothing, and such a node can
/// still carry a name — a hovered idle one draws no disc and is named all the
/// same — so a label's place is not its node's index in the sorted list. And
/// the labels arrive in the scene's order, which is not the order they are
/// drawn in.
#[test]
fn a_label_takes_its_own_nodes_place_in_the_order() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.pluses.clear();
    let node = |z: f32, activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, z),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: z == 0.0,
        ..scene.nodes[0]
    };
    // In the scene's own order: the near sheet first, then two silent nodes
    // behind everything, then the home sheet. Drawn back to front that is
    // hush, hush, home, near — so nothing here is in the order it is drawn,
    // and the two silent ones ship no instance at all.
    scene.nodes = vec![node(1.0, 1.0), node(-1.0, 0.0), node(-1.0, 0.0), node(0.0, 1.0)];
    let (near, hush_a, hush_b, home) = (0u32, 1u32, 2u32, 3u32);

    // A glyph per label, told apart by where it claims to be.
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0), glyph(2.0), glyph(3.0)],
            labels: [near, hush_a, hush_b, home].map(|node| Label { node, glyphs: 1 }).to_vec(),
            node_points: 0.0,
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        11,
        None,
    );

    assert_eq!(call.instances.len(), 2, "only the two sounding nodes ship an instance");
    assert_eq!(
        call.draws,
        vec![
            // Both silent nodes: nothing has been drawn yet, and two names on
            // one sheet are one uninterrupted draw.
            Draw::Label(0, 2, 0),
            // The home sheet's own node — its knockout, its disc, its name.
            Draw::Clearing(0),
            Draw::Nodes(0, 1),
            Draw::Label(2, 3, 1),
            // And the near sheet's, after everything.
            Draw::Nodes(1, 2),
            Draw::Label(3, 4, 2),
        ],
        "a name goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
    // One box per DRAW and not per name, and the merged draw's box spans both
    // of the names in it: the hole is cut once over whatever that draw covers,
    // which is what stops two names' holes compounding where they overlap.
    assert_eq!(
        call.gutters.iter().map(|g| g.rect).collect::<Vec<_>>(),
        vec![[1.0, 0.0, 2.0, 1.0], [3.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0]],
        "the merged run's box holds both its names and each other box holds one",
    );
}

/// Two names landing next to each other in the order from different sheets are
/// two draws, the nearer last — a merge is for one sheet only.
///
/// Adjacent names merge into a single rim-then-fill draw, which is what stops
/// two neighbouring letters darkening each other's ink where their rims
/// overlap. Across a sheet boundary that is the wrong answer: the nearer name's
/// rim is meant to separate its glyphs from the farther name's fill, and on a
/// face-on sevens lattice the two overlap exactly, a sevens node sitting on top
/// of its home node.
///
/// The state is an ordinary one: a home node sounding with a silent node on the
/// sheet in front of it hovered. The hovered node draws nothing and is named
/// all the same, so the two names land side by side in the walk with nothing
/// between them to break the run.
#[test]
fn two_adjacent_names_from_different_sheets_draw_the_nearer_last_and_apart() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.pluses.clear();
    let node = |z: f32, activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, z),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: z == 0.0,
        ..scene.nodes[0]
    };
    // The near sheet's node FIRST in the scene, so that lattice order and depth
    // order disagree: drawn back to front it is home, then near. The home node
    // ships and the silent near one does not, so the two names come out of the
    // walk with nothing between them.
    scene.nodes = vec![node(1.0, 0.0), node(0.0, 1.0)];
    let (near, home) = (0u32, 1u32);
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0)],
            labels: [near, home].map(|node| Label { node, glyphs: 1 }).to_vec(),
            node_points: 0.0,
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        17,
        None,
    );

    assert_eq!(call.instances.len(), 1, "only the sounding home node ships an instance");
    assert_eq!(
        call.draws,
        vec![Draw::Clearing(0), Draw::Nodes(0, 1), Draw::Label(0, 1, 0), Draw::Label(1, 2, 1)],
        "names from different sheets are two draws, the nearer's rim on the other's fill",
    );
    assert_eq!(call.gutters.len(), 2, "two draws that did not merge cut two holes");
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 0.0],
        "the home sheet's name draws first and the nearer sheet's over it",
    );
}

/// A name on a node that ships no disc still draws over the markers standing
/// behind it, not under them.
///
/// The cull is what makes this worth pinning: a node that paints nothing ships
/// no instance, so it moves nothing in the buffers, and every scheme that gives
/// a name its place by counting instances has such a node's name landing on the
/// same number as whatever was drawn before it. Reading a side off that number
/// files the name with the markers rather than after them, and they are then
/// painted over it. The walk has no number to read: the name is emitted where
/// the node is, so it lands after every draw that came before.
///
/// The state is the plugin's resting one, which is what makes it worth a test
/// of its own: stock view, nothing played, hover any node. An idle node draws
/// nothing at all, and a hovered node is named whether or not it draws.
#[test]
fn a_culled_home_nodes_name_draws_over_the_markers_behind_it() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // The stock resting state: one sheet, and an untouched node painting
    // nothing at all, which is every idle node.
    let node = |activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, 0.0),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: true,
        ..scene.nodes[0]
    };
    // The hovered one is silent and first, so it is culled before anything at
    // all has shipped — the case where a count has nothing to distinguish.
    scene.nodes = vec![node(0.0), node(1.0)];
    let glyph = GlyphInstance { rect: [0.0, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph],
            labels: vec![Label { node: 0, glyphs: 1 }],
            node_points: 0.0,
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        13,
        None,
    );

    assert!(!scene.pluses.is_empty(), "the fixture needs a marker for the name to be covered BY");
    let named = call.draws.iter().position(|d| matches!(d, Draw::Label(..)));
    let marked = call.draws.iter().position(|d| matches!(d, Draw::Pluses(..)));
    assert!(
        marked < named && named.is_some(),
        "a culled home node's name draws after the markers behind it: {:?}",
        call.draws,
    );
}

/// A label is not in the bloom: the light the bloom adds to a frame is the
/// same whether the names are drawn or not.
///
/// Two halves, and the second is the one that is easy to miss. A name in the
/// bloom input GLOWS, which is the obvious half — white type well over the
/// bright pass's threshold, haloed like a lit node. It also takes a BITE out
/// of the halo of the node it covers, by standing where that node's own
/// bright pixels were, so a node dims as a name crosses it. Both are what a
/// second colour attachment buys, and both show up here as the same
/// difference.
///
/// Measured as bloom LIGHT rather than as pixels, since that is the thing
/// that must not change: the composite adds `bloom * strength` on top of the
/// scene, so rendering each arrangement with the bloom on and off and
/// subtracting isolates exactly the term under test. The scene itself does
/// change with a label in it — that is the feature — so comparing frames
/// directly would be comparing the label to itself.
#[test]
fn a_label_adds_no_light_through_the_bloom() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let on = scene
        .projector(glam::Vec2::new(points.x, points.y))
        .project(scene.nodes[0].world_pos)
        .expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());

    // The near node's name, right on the node — over its brightest pixels,
    // which is where a label robs a halo of the most light.
    let label = |on: bool| -> (Vec<GlyphInstance>, Vec<Label>) {
        if !on {
            return (Vec::new(), Vec::new());
        }
        (
            vec![GlyphInstance {
                rect: [x - 6.0, y - 6.0, 12.0, 12.0],
                ..crate::text::tests::glyph()
            }],
            vec![Label { node: 0, glyphs: 1 }],
        )
    };
    let frame = |labelled: bool, bloom: f32| -> Vec<u8> {
        let (glyphs, labels) = label(labelled);
        // A fresh fixture per frame: `Scene` is not `Clone`, and the only
        // thing that varies is the strength the composite reads.
        let mut scene = one_node_behind_another();
        scene.bloom_strength = bloom;
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                node_points: 0.0,
                atlas: Some(crate::text::tests::atlas()),
                marks: None,
                slide: SlideAxis::default(),
            },
            points,
            format,
            13,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let clear = wgpu::Color::TRANSPARENT;
        let texture = render_to_texture(&device, &queue, SCENE_SIZE, format, clear, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SCENE_SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    let (lit, plain) = (frame(true, 1.0), frame(true, 0.0));
    let (bare_lit, bare_plain) = (frame(false, 1.0), frame(false, 0.0));

    // Non-vacuous first: there has to BE bloom, and it has to reach the
    // pixels the label covers — otherwise this passes on a frame with no
    // light in it to move.
    let light = |on: &[u8], off: &[u8], i: usize| on[i] as i32 - off[i] as i32;
    let near_label = |i: usize| {
        let row = SCENE_SIZE[0] as usize;
        let (px, py) = (((i / 4) % row) as f32, ((i / 4) / row) as f32);
        (px - x).abs() <= 10.0 && (py - y).abs() <= 10.0
    };
    let peak = (0..lit.len())
        .filter(|i| i % 4 != 3 && near_label(*i))
        .map(|i| light(&bare_lit, &bare_plain, i))
        .max()
        .unwrap_or(0);
    assert!(peak > 8, "the fixture must bloom where the label sits, peak {peak}");

    // And the label changes none of it. Saturated channels are skipped: the
    // glyph is white, so where it lands the composite is already at 255 with
    // the bloom off and the added light has nowhere to go — a clamp, not a
    // reading.
    let (mut worst, mut at) = (0i32, 0usize);
    for i in (0..lit.len()).filter(|i| i % 4 != 3) {
        if lit[i] == 255 || bare_lit[i] == 255 {
            continue;
        }
        let d = light(&lit, &plain, i) - light(&bare_lit, &bare_plain, i);
        if d.abs() > worst.abs() {
            (worst, at) = (d, i);
        }
    }
    assert!(
        worst.abs() <= 1,
        "a label changed the bloom by {worst}/255 at pixel ({}, {}) — it is in the bright \
         pass's input, so it glows and eats the halo of the node it covers",
        (at / 4) % SCENE_SIZE[0] as usize,
        (at / 4) / SCENE_SIZE[0] as usize,
    );
}

/// The name's own fixture: a lit node at the origin and a light wide enough to
/// reach past every ring it paints.
///
/// The arrangement is `a_resting_marker_wears_the_wash_it_stands_in`'s, and
/// deliberately: the tests below are that cross's claims asked of a name, so
/// what they are read against has to be the same picture with the cross swapped
/// for a glyph.
fn lit_node_and_a_name(reach: f32, shadow: f32, depth: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.glow_reach = reach;
    scene.glow_strength = 1.5;
    scene.glow_feather = 1.0;
    scene.glow_shadow = shadow;
    // The fade the whole width of the shadow, which is the pairing the bar
    // ships in and the one the width means anything simple at: `glow_shadow_soft`
    // is clamped to the width by `ViewConfig::sanitize`, so a fixture that left
    // it alone would narrow its own fade as it widened its shadow.
    scene.glow_shadow_soft = shadow;
    scene.glow_shadow_depth = depth;
    // The markers away: a cross would write a standoff of its own into the
    // layer both tests below read.
    scene.pluses.clear();
    scene
}

/// Where the name stands in that fixture: uv 1 is 1.8 node radii, so the
/// outermost ring reaches 1.57 world units and this is well clear of it, inside
/// a reach that carries light past both.
const NAME_AT: glam::Vec3 = glam::Vec3::new(3.0, 0.0, 0.0);

/// How wide that name's one glyph is drawn, in points. Its atlas patch is 8
/// texels square and opaque (`text::tests::atlas`), so at this size it is a
/// solid block of ink several pixels across — and a pixel inside it carries the
/// label's colour exactly, which is what the wash is read on.
const NAME_SIZE: f32 = 12.0;

/// The Shadow width the fresh view opens at, for the shots that are about
/// something else and want the bar left where the picture has it.
const FRESH_SHADOW: f32 = 0.16;

/// That name, in the resting field's own grey and at full strength.
fn one_name(scene: &Scene, size: [u32; 2]) -> LatticeLabels {
    name_at(scene, size, NAME_AT)
}

/// [`one_name`]'s glyph, standing wherever the caller puts it.
fn name_at(scene: &Scene, size: [u32; 2], world: glam::Vec3) -> LatticeLabels {
    let at = scene
        .projector(glam::Vec2::new(size[0] as f32, size[1] as f32))
        .project(world)
        .expect("the name stands in front of the camera");
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let ink = scene.lattice_ground;
    LatticeLabels {
        glyphs: vec![GlyphInstance {
            rect: [at.x - NAME_SIZE / 2.0, at.y - NAME_SIZE / 2.0, NAME_SIZE, NAME_SIZE],
            fill: [byte(ink.x), byte(ink.y), byte(ink.z), 255],
            // The shadow's strength, which is the whole of what the glow pass
            // reads off this colour (`fs_glyph_glow`).
            rim: [0, 0, 0, 255],
            ..crate::text::tests::glyph()
        }],
        labels: vec![Label { node: 0, glyphs: 1 }],
        // How large a node draws here, which is the unit the Shadow bars are
        // dialled in and so the whole of what gives the name's standoff a size.
        // Taken off the camera exactly as the pane takes it
        // (`TextBatch::lattice_labels`), a fixture that made its own answer up
        // being one that could agree with the shader while disagreeing with the
        // picture.
        node_points: scene.node_radius * scene.camera.points_per_world(size[1] as f32),
        atlas: Some(crate::text::tests::atlas()),
        marks: Some(crate::text::tests::mark_sheet()),
        slide: SlideAxis::default(),
    }
}

/// A name wears the light it stands in, exactly as a resting cross does.
///
/// The pair is one field: a position shows a name or a marker and never both
/// (`derive_pluses`), so a name that did not take the wash would read as a hole
/// punched in the halo at precisely the positions a cross reads as standing in
/// it — the handover between the two visible as the light flinching rather than
/// as one shape replacing another.
///
/// Read against the glow OFF, which is the one setting that takes the wash out
/// of the picture and leaves everything else where it was.
#[test]
fn a_name_wears_the_wash_it_stands_in() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let bare = shooter.shot(&lit_node_and_a_name(0.0, FRESH_SHADOW, 0.0));
    let unlit = lit_node_and_a_name(0.0, FRESH_SHADOW, 0.0);
    let off = shooter.shot_with(&unlit, one_name(&unlit, SIZE));
    // The name's own pixels: what it drew where the node had left the frame
    // black. Its ink is laid down flat and premultiplied, so a pixel it covers
    // completely carries that colour exactly and the brightest value in the set
    // IS the colour — every other one is a fraction of it across the edge.
    let drawn: Vec<usize> = (0..bare.len())
        .step_by(4)
        .filter(|&i| bare[i..i + 4] == [0u8, 0, 0, 255] && off[i..i + 4] != bare[i..i + 4])
        .collect();
    let full: [u8; 3] =
        std::array::from_fn(|c| drawn.iter().map(|&i| off[i + c]).max().unwrap_or(0));
    let name: Vec<usize> = drawn.into_iter().filter(|&i| off[i..i + 3] == full).collect();
    assert!(name.len() > 30, "the name covers {} whole pixels of its own", name.len());

    let lit = lit_node_and_a_name(1.6, FRESH_SHADOW, 0.0);
    let worn = shooter.shot_with(&lit, one_name(&lit, SIZE));
    let lifted =
        name.iter().filter(|&&i| brightness(&worn[i..i + 3]) > brightness(&off[i..i + 3])).count();
    assert_eq!(
        lifted,
        name.len(),
        "the light lifted {lifted} of the name's {} pixels: the name is not wearing the light \
         it stands in",
        name.len(),
    );
    let dimmed = name.iter().filter(|&&i| (0..3).any(|c| worn[i + c] < off[i + c])).count();
    assert_eq!(dimmed, 0, "the wash took light off {dimmed} of the name's {} pixels", name.len());
    let by_wash = name
        .iter()
        .map(|&i| (0..3).map(|c| worn[i + c].abs_diff(off[i + c])).max().unwrap())
        .max()
        .unwrap();
    assert!(
        by_wash > 20,
        "the fixture's wash moves the name by {by_wash}; there is nothing here to measure",
    );
}

/// A name holds the light off itself, on the Shadow bars a node's rings and a
/// marker's cross are held off by.
///
/// The other half of the pair above, and the whole of why a name needs no
/// painted rim: what keeps a halo from swallowing the type is a shape in the
/// LIGHT, written in the light's own pass, so it stands off whatever reaches
/// there — a neighbour's halo as readily as the named node's.
///
/// Measured on the GROUND around the name rather than on the name itself, the
/// ink being washed by the raw light and so unmoved by its own shadow, exactly
/// as a cross's is. The light it stands off out there is the NODE's, the name
/// standing well clear of every ring that node paints, so what is read is a
/// shadow cast on somebody else's light and not a name dimming its own.
#[test]
fn a_name_holds_the_light_off_the_ground_it_stands_on() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let (ground, dimmed) = shadowed_ground(&mut shooter, FRESH_SHADOW, 1.0);
    assert!(
        ground.len() > 1000,
        "the fixture must leave ground for the shadow to land on, not {}",
        ground.len(),
    );
    assert!(
        dimmed.len() > 40,
        "a name at a full Shadow depth dimmed only {} of the {} pixels its ink never reaches",
        dimmed.len(),
        ground.len(),
    );
}

/// What a name takes off the light around it at one setting of the Shadow: the
/// ground its ink never covers, and which of those pixels it darkened.
///
/// The pair at a depth of 0 is asserted here rather than left to a caller, and
/// it is what says the darkening measured is the STANDOFF: with the Shadow shut
/// nothing a name draws may take light off anything, so a reading that survives
/// this is not the glyph pass finding some other way to darken the picture.
///
/// The footprint is read at depth 0, where a name writes ink and nothing else.
/// It does not move with the depth, so one reading answers for every shot at
/// that width.
fn shadowed_ground(
    shooter: &mut Shooter,
    shadow: f32,
    depth: f32,
) -> (Vec<usize>, std::collections::BTreeSet<usize>) {
    const SIZE: [u32; 2] = [256, 256];
    let mut shot = |depth: f32, named: bool| -> Vec<u8> {
        let scene = lit_node_and_a_name(1.6, shadow, depth);
        let labels = if named { one_name(&scene, SIZE) } else { LatticeLabels::default() };
        shooter.shot_with(&scene, labels)
    };
    let flat_bare = shot(0.0, false);
    let flat = shot(0.0, true);
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    let flat_dimmed = ground
        .iter()
        .filter(|&&i| brightness(&flat[i..i + 3]) < brightness(&flat_bare[i..i + 3]))
        .count();
    assert_eq!(flat_dimmed, 0, "a name took light off the ground at a Shadow depth of 0");

    let deep_bare = shot(depth, false);
    let deep = shot(depth, true);
    let dimmed = ground
        .iter()
        .copied()
        .filter(|&i| brightness(&deep[i..i + 3]) < brightness(&deep_bare[i..i + 3]))
        .collect();
    (ground, dimmed)
}

/// The Shadow's WIDTH says how far a name's shadow reaches, on the same bar it
/// says it to a node's rings and a marker's cross.
///
/// This is what parts the standoff a name casts from the painted rim it
/// replaces. A rim is a radius of its own: it answers to no bar, so a lattice
/// dialled to a wide soft shadow drew one everywhere except around the type,
/// where a hard keyline of a fixed couple of points stayed exactly as it was.
/// Two shadows in one picture, and the seam between them at every name.
///
/// A superset is what says the width STRETCHES one shape rather than deepening
/// it: every pixel a narrow Shadow darkens, a wide one darkens too.
///
/// Read as the difference the NAME makes at each width, which is what keeps the
/// claim about the name: the node's own standoff widens on the same bar and the
/// shade layer is a `max`, so a frame read on its own says which of two shadows
/// won rather than how far this one reaches. Both widths are chosen to leave the
/// node's standoff short of the name — at the wide end it stops half a world unit
/// clear of the ink — so nothing in the narrow set is a pixel the node has
/// already claimed.
#[test]
fn a_names_shadow_reaches_as_far_as_its_width_says() {
    const SIZE: [u32; 2] = [256, 256];
    const NARROW: f32 = 0.1;
    const WIDE: f32 = 0.3;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let narrow = shadowed_ground(&mut shooter, NARROW, 1.0).1;
    let wide = shadowed_ground(&mut shooter, WIDE, 1.0).1;
    assert!(!narrow.is_empty(), "the narrow Shadow must cast a shadow at all");
    assert!(
        wide.len() > narrow.len() * 2,
        "widening the Shadow from {NARROW} to {WIDE} shadowed {} against {}",
        wide.len(),
        narrow.len(),
    );
    let missed = narrow.difference(&wide).count();
    assert_eq!(missed, 0, "the wider Shadow left {missed} of the narrow shadow's pixels lit");
}

/// One Shadow, two shaders: the curve a name's standoff is cast on is the curve
/// a ring's and a cross's are cast on, constant for constant.
///
/// It is written twice — `standoff_coverage` and `gap_shade` in lattice.wgsl,
/// their copies in text.wgsl — because the two draws are two shader modules and
/// neither can call into the other. This is what that costs. Every number under
/// the curve is tuned, and a copy of one drifting is a name whose shadow is a
/// different shape from the shadow beside it at some setting of a bar that
/// carries both, which no single picture makes obvious.
///
/// The BODIES cannot be compared this way — the lattice reads its terms off a
/// node's uv and the glyph pass off `Locals` — so what is pinned is the
/// arithmetic's constants, which is where drift would actually land.
#[test]
fn the_names_shadow_is_the_rings_own_curve() {
    let value = |src: &str, name: &str, what: &str| -> String {
        let prefix = format!("const {name}: f32 = ");
        src.lines()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("{what} no longer defines {name}"))
            .trim_end_matches(';')
            .to_owned()
    };
    for name in [
        "SHADOW_TAIL",
        "SHADOW_STOP",
        "SHADOW_SHAPE_RIND",
        "SHADOW_SHAPE_PLAIN",
        "SHADOW_SOFT_FLOOR",
        "SHADOW_KEEP_FLOOR",
    ] {
        assert_eq!(
            value(SHADER_SRC, name, "the lattice"),
            value(crate::text::TEXT_SRC, name, "the names"),
            "the lattice's {name} and the names' have drifted apart",
        );
    }
}

/// A name knocks a hole in what was drawn before it, the way its node does —
/// its own node's RINGS included, those being drawn immediately under it.
///
/// The reading is what makes this a covering claim and not a dimming one. A
/// hole is a premultiplied over of the GROUND at its own coverage, so every
/// pixel it touches lands BETWEEN the picture with no name in it and that
/// ground. Ink that merely darkened what it stood on would fail that on the
/// first pixel where the ring is darker than the ground it stands over, and a
/// name that painted a halo of its own would fail it everywhere.
///
/// The Reach is 0, so no light stands anywhere and the ground is one value for
/// the whole frame — `Scene::background`, which is what `node_paint` and
/// `fs_fill_lit` both clear to. With light in the picture the same claim needs
/// the field read back per pixel, which is the shader's own arithmetic restated
/// as a test.
///
/// The control is the same frame with no name in it, at the SAME Shadow.
/// Everything else in the picture moves with that bar — the node's own hole,
/// the standoff over it — so two shots at two Shadows say nothing about the
/// name; two shots at one, differing only in whether the glyph ships, say all
/// of it.
#[test]
fn a_name_covers_the_rings_it_stands_on() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.6;
    /// How far a node's own billboard reaches, in node radii —
    /// in lattice.wgsl. The outer bound on anything a node paints, and so on
    /// anything a hole cut in that node can be read against.
    const NODE_QUAD: f32 = 1.6;
    /// Where the name stands: on the node's own octave band rather than in the
    /// empty middle, which is the one place a hole can be READ. A node's own
    /// clearing has already cleared its middle to the ground, so a name there
    /// paints the ground over the ground and the picture cannot tell.
    const ON_THE_BAND: glam::Vec3 = glam::Vec3::new(1.0, 0.0, 0.0);
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The name in the node's own middle, which is where the lattice puts one.
    let mut shots = |shadow: f32| -> (Scene, Vec<u8>, Vec<u8>) {
        let scene = lit_node_and_a_name(0.0, shadow, 1.0);
        let bare = shooter.shot(&scene);
        let named = shooter.shot_with(&scene, name_at(&scene, SIZE, ON_THE_BAND));
        (scene, bare, named)
    };

    // The name's own INK, taken at a Shadow of 0 where a name paints that and
    // nothing else. It does not move with the bar, so one reading answers for
    // both shots.
    let (_, flat_bare, flat) = shots(0.0);
    let ink: std::collections::BTreeSet<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] != flat_bare[i..i + 4]).collect();
    assert!(ink.len() > 40, "the fixture's name must land on the pane, not {} pixels", ink.len());

    let (scene, bare, named) = shots(SHADOW);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let ground = [byte(scene.background.x), byte(scene.background.y), byte(scene.background.z)];
    // Where the node's own billboard reaches, in pixels: uv 1 is 1.8 node
    // radii, so nothing the node paints stands outside this and a hole inside
    // it is a hole in the node.
    let radius = scene.node_radius * scene.camera.points_per_world(SIZE[1] as f32) * NODE_QUAD;
    let centre = scene
        .projector(glam::Vec2::new(SIZE[0] as f32, SIZE[1] as f32))
        .project(glam::Vec3::ZERO)
        .expect("the node stands in front of the camera");
    let (mut touched, mut on_the_node) = (0usize, 0usize);
    for i in (0..named.len()).step_by(4) {
        if ink.contains(&i) || named[i..i + 4] == bare[i..i + 4] {
            continue;
        }
        touched += 1;
        for c in 0..3 {
            let (was, now, to) = (bare[i + c] as i32, named[i + c] as i32, ground[c]);
            assert!(
                now >= was.min(to) - 2 && now <= was.max(to) + 2,
                "a name moved a pixel outside its own ink to {now}, which is not between \
                 the {was} it stood on and the {to} a hole clears to",
            );
        }
        let px = (i / 4) as u32;
        let (x, y) = ((px % SIZE[0]) as f32, (px / SIZE[0]) as f32);
        if (x - centre.x).hypot(y - centre.y) <= radius {
            on_the_node += 1;
        }
    }
    assert!(touched > 250, "a name at Shadow {SHADOW} cleared only {touched} pixels");
    assert!(
        on_the_node > 150,
        "a name must clear the node it stands on, and only {on_the_node} of {touched} \
         cleared pixels were inside the node's own billboard",
    );

    // And with the Shadow shut it clears nothing at all: `ink` above IS the
    // difference the name makes at 0, so a hole there would be counted into it
    // and this asserts that set is the glyph's own footprint and no larger.
    let outside: Vec<usize> = ink
        .iter()
        .copied()
        .filter(|&i| {
            let px = (i / 4) as u32;
            let (x, y) = ((px % SIZE[0]) as f32, (px / SIZE[0]) as f32);
            (x - centre.x).hypot(y - centre.y) > radius
        })
        .collect();
    assert!(
        outside.is_empty(),
        "at a Shadow of 0 a name must paint its ink and nothing else, and {} pixels of it \
         landed outside the node it names",
        outside.len(),
    );
}

/// The contour a name's shadow stands off from is where its ink's coverage
/// says, and how DEEP that shadow is owes that coverage nothing.
///
/// Two blocks, one solid and one whose outer ring is half covered — which is
/// what a rasterizer reports for any edge falling mid-texel. They seed the
/// field at the same texels, so the whole of their difference is where inside
/// those texels each one's edge lies: half a texel, and the second's shadow is
/// the first's read half a texel further out.
///
/// Spending that coverage on the shadow's HEIGHT is what this rules out, and it
/// is not a fine distinction: coverage runs the whole of `[INK_FLOOR, 1]` along
/// any contour the texel grid does not run parallel to, so two neighbouring
/// texels of one curve cast shadows a factor of two apart. Each owns a wedge of
/// the plane that widens as it goes, which draws bright and dark rays fanning
/// out of every letter and a hard seam wherever two strokes of one stand near
/// each other.
///
/// A BLOCK is what makes that measurable. An axis-aligned edge puts one
/// coverage in every texel along it, so the error stops being a fan — which no
/// single pixel is the place to read — and becomes one factor over a whole
/// scanline. The curve is the picture the artefact shows up in; the block is
/// the picture it can be measured in.
///
/// The name stands at the node's own centre, in the flat middle of its light:
/// the reading is the SHARE of the light taken, so the ground under it has to
/// be one value for a scanline rather than a gradient the share would carry.
#[test]
fn a_names_shadow_is_cast_from_its_contour_and_not_from_its_alpha() {
    const SIZE: [u32; 2] = [384, 384];
    /// Narrow enough that the shadow's whole ramp stands well inside the node's
    /// rings, where its own standoff — the same bar, and a `max` against this
    /// one — would otherwise be the deeper of the two and take the reading.
    const SHADOW: f32 = 0.2;
    /// The block, in points, which the pane draws one to a pixel: the patch is
    /// 8 texels square, so at 8 points its contour reaches the field as sharp as
    /// the one epaint rasterizes.
    const BLOCK: f32 = 8.0;
    /// How far out the two profiles are compared, in pixels from the solid
    /// block's edge. Clear of the ink itself at the near end, and short of where
    /// the ramp has run out at the far end.
    const BAND: std::ops::RangeInclusive<i32> = 2..=20;
    /// What lies between the two contours, in pixels. A solid ring's edge
    /// stands half a texel out from the texels' centres and [`HALF`]'s stands
    /// on them, so the second block is the first shrunk by exactly this — and
    /// its shadow is the first's read this much further out.
    const SHIFT: f32 = 0.5;
    /// The half-covered ring, as a byte. Over `INK_FLOOR` rather than at it, so
    /// the ring seeds the field: a texel under the floor is no seed at all, and
    /// the contour would jump a whole texel inward instead of half of one.
    const HALF: u8 = 128;

    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let mut scene = lit_node_and_a_name(2.5, SHADOW, 1.0);
    // Big enough that the ramp above is tens of pixels wide, which is what puts
    // the half texel this measures well inside one.
    scene.node_radius = 2.6;
    let centre = scene
        .projector(glam::Vec2::new(SIZE[0] as f32, SIZE[1] as f32))
        .project(glam::Vec3::ZERO)
        .expect("the node stands in front of the camera");
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let ink = scene.lattice_ground;
    let block = |atlas: crate::FontAtlas| LatticeLabels {
        glyphs: vec![GlyphInstance {
            rect: [centre.x - BLOCK / 2.0, centre.y - BLOCK / 2.0, BLOCK, BLOCK],
            fill: [byte(ink.x), byte(ink.y), byte(ink.z), 255],
            rim: [0, 0, 0, 255],
            ..crate::text::tests::glyph()
        }],
        labels: vec![Label { node: 0, glyphs: 1 }],
        node_points: scene.node_radius * scene.camera.points_per_world(SIZE[1] as f32),
        atlas: Some(atlas),
        marks: Some(crate::text::tests::mark_sheet()),
        slide: SlideAxis::default(),
    };
    let bare = shooter.shot(&scene);
    let solid = shooter.shot_with(&scene, block(crate::text::tests::atlas()));
    let half = shooter.shot_with(&scene, block(crate::text::tests::edged_atlas(HALF)));

    // Sampled between pixels, because half a texel is the quantity: rounding to
    // the nearer one would round away exactly what the two shots differ by.
    let at = |shot: &[u8], x: f32| -> f32 {
        let (x0, y0) = (x.floor() as u32, centre.y.floor() as u32);
        let fx = x - x0 as f32;
        let read = |shot: &[u8], x: u32| {
            let i = ((y0 * SIZE[0] + x) * 4) as usize;
            brightness(&shot[i..i + 3]) as f32
        };
        let ground = read(&bare, x0) * (1.0 - fx) + read(&bare, x0 + 1) * fx;
        let lit = read(shot, x0) * (1.0 - fx) + read(shot, x0 + 1) * fx;
        (ground - lit) / ground
    };

    let edge = centre.x + BLOCK / 2.0;
    let (mut worst, mut own, mut shifted) = (0.0f32, 0.0f32, 0.0f32);
    for d in BAND {
        let x = edge + d as f32;
        let (a, b) = (at(&solid, x + SHIFT), at(&half, x));
        worst = worst.max((b - a).abs());
        shifted += (b - a).abs();
        own += (b - at(&solid, x)).abs();
    }
    // The band spans the whole ramp, which is what makes a disagreement over it
    // a disagreement about the profile rather than about its tail.
    let (near, far) =
        (at(&solid, edge + *BAND.start() as f32), at(&solid, edge + *BAND.end() as f32));
    assert!(near > 0.8 && far < 0.2, "the band runs {near:.2} to {far:.2}, which is no ramp");
    // A twentieth of the share. A height scaled by the ring's coverage is worth
    // 0.16 of it at the worst pixel of this band, and the contour's own half
    // texel 0.01, so the threshold has either side of it by a factor of three.
    assert!(
        worst < 0.05,
        "a half-covered edge cast a shadow {worst:.3} of the light away from the same edge \
         drawn solid, over and above the half texel between them",
    );
    // And the half texel is a real shift and not a rounding: what the ring's
    // coverage buys is a contour inside its own texel, so the shifted profile
    // has to be the nearer of the two by a clear margin.
    assert!(
        shifted * 2.0 < own,
        "the half-covered block's shadow sits {shifted:.3} from the solid block's read half a \
         texel out and {own:.3} from it read where it stands — the contour is not moving",
    );
}
