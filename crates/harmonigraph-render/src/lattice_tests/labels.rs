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
    // Shadow: the fill alone answers the question, and a shadow would spread
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
            // Both silent nodes: nothing has been drawn yet. Two names, two
            // draws — a name is its shadow and then its ink, and the second
            // name's shadow has to land on the first's ink (see
            // [`Draw::Label`]).
            Draw::Label(0, 1, 0),
            Draw::Label(1, 2, 1),
            // The home sheet's own node, then its name. The caster indices skip
            // where they do because a node that ships takes a cell of its own
            // between the two names either side of it.
            Draw::Nodes(0, 1),
            Draw::Label(2, 3, 3),
            // And the near sheet's, after everything.
            Draw::Nodes(1, 2),
            Draw::Label(3, 4, 5),
        ],
        "a name goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
    // Each name's own caster is the box of its own glyph — read through the
    // index the draw carries rather than off the list's order, which the nodes'
    // own cells are in as well.
    assert_eq!(
        call.draws
            .iter()
            .filter_map(|draw| match *draw {
                Draw::Label(_, _, l) => Some(call.casters[l as usize].rect),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            [1.0, 0.0, 1.0, 1.0],
            [2.0, 0.0, 1.0, 1.0],
            [3.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0]
        ],
        "each name casts from its own box",
    );
}

/// Two names landing next to each other in the order from different sheets are
/// two draws, the nearer last.
///
/// On a face-on sevens lattice the two overlap exactly, a sevens node sitting
/// on top of its home node, so which draws last is which is read: the nearer
/// name's shadow lands on the farther name's ink and its ink over both.
///
/// The state is an ordinary one: a home node sounding with a silent node on the
/// sheet in front of it hovered. The hovered node draws nothing and is named
/// all the same, so the two names land side by side in the walk with nothing
/// between them.
#[test]
fn two_adjacent_names_from_different_sheets_draw_the_nearer_last() {
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
        vec![Draw::Nodes(0, 1), Draw::Label(0, 1, 1), Draw::Label(1, 2, 2)],
        "names from different sheets are two draws, the nearer's shadow on the other's ink",
    );
    assert_eq!(
        call.draws.iter().filter(|draw| matches!(draw, Draw::Label(..))).count(),
        2,
        "two names, two draws",
    );
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

/// Where the name stands in [`lit_node_and_a_name`] where the reading is of the
/// GROUND: uv 1 is 1.8 node radii, so the outermost ring reaches 1.57 world
/// units and this is well clear of it, inside a reach that carries light past
/// both. [`name_on_the_band`] is where a reading wants the node's ink instead.
const NAME_AT: glam::Vec3 = glam::Vec3::new(3.0, 0.0, 0.0);

/// The Shadow width the fresh view opens at, for the shots that are about
/// something else and want the bar left where the picture has it.
const FRESH_SHADOW: f32 = 0.16;

/// That name, in the resting field's own grey and at full strength.
fn one_name(scene: &Scene, size: [u32; 2]) -> LatticeLabels {
    name_at(scene, size, NAME_AT)
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
/// This is what parts the shadow a name casts from the painted rim it replaces.
/// A rim is a radius of its own: it answers to no bar, so a lattice dialled to a
/// wide soft shadow drew one everywhere except around the type, where a hard
/// keyline of a fixed couple of points stayed exactly as it was. Two shadows in
/// one picture, and the seam between them at every name.
///
/// A superset is what says the width STRETCHES one shape rather than deepening
/// it: every pixel a narrow Shadow darkens, a wide one darkens too.
///
/// Read as the difference the NAME makes at each width, which is what keeps the
/// claim about the name: the node's own shadow widens on the same bar, so a
/// frame read on its own carries both. Both widths are chosen to leave the
/// node's shadow short of the name — at the wide end it stops half a world unit
/// clear of the ink — so nothing in the narrow set is a pixel the node has
/// already darkened.
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

/// The Shadow depth's floor is one number across a name and a ring:
/// text.wgsl's `KEEP_FLOOR` is lattice.wgsl's `SHADOW_KEEP_FLOOR`.
///
/// Written twice because there is no linkage between shader modules, and the
/// one constant the two shadows still share: the top of the depth bar is a
/// shadow ten stops deep on both, so a name's shadow and a ring's at that
/// setting are the same darkness.
#[test]
fn the_names_shadow_depth_bottoms_out_where_the_rings_does() {
    use crate::shadow::tests::shader_const;
    assert_eq!(
        shader_const(crate::text::TEXT_SRC, "KEEP_FLOOR"),
        shader_const(SHADER_SRC, "SHADOW_KEEP_FLOOR"),
        "the names' floor and the rings' have drifted apart",
    );
}

/// The mid-grey the share tests stand on, as the pane's ground and the
/// Shooter's clear alike.
const GREY: f32 = 0.55;

/// [`lit_node_and_a_name`] with no light, over a mid-grey ground: the picture
/// is the node's ink and the ground, both bright enough that a share taken off
/// either is a reading. The Shooter has to be cleared to the same grey
/// ([`over_grey_clear`]) so the ground is one value across the whole frame.
fn inked_on_grey(shadow: f32, depth: f32) -> Scene {
    let mut scene = lit_node_and_a_name(0.0, shadow, depth);
    scene.background = glam::Vec4::new(GREY, GREY, GREY, 1.0);
    scene
}

/// The Shooter's clear for a scene [`inked_on_grey`] built.
fn over_grey_clear() -> wgpu::Color {
    wgpu::Color { r: f64::from(GREY), g: f64::from(GREY), b: f64::from(GREY), a: 1.0 }
}

/// The brightness of one pixel of `shot`, at `(x, y)` of a `size` frame.
fn bright_at(shot: &[u8], size: [u32; 2], x: u32, y: u32) -> i64 {
    let i = ((y * size[0] + x) * 4) as usize;
    brightness(&shot[i..i + 3])
}

/// A name's shadow takes the same SHARE off its node's ink as off the ground
/// beside it, at the same distance from the name.
///
/// The receiver asymmetry, closed. Ink and ground are both whatever is already
/// in the frame under the name's box, and one multiply lands on both — where a
/// hole cleared to the ground on one curve and a shade dimmed the light on
/// another, and at a quarter Shadow the ink kept 75% where the ground kept 55%.
/// Measured on a stroke standing across the octave band's outer edge, so the
/// same distance out from the stroke is the band's ink on one side and the bare
/// ground on the other.
///
/// No light, so what is under the name depends on nothing the Reach does; the
/// camera in close so the band is tens of pixels deep and the pair of readings
/// stands well inside it.
#[test]
fn a_names_shadow_takes_the_same_share_off_ink_as_off_ground() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.25;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_grey_clear();
    // HALF the depth bar, so that both receivers keep a brightness to take a
    // share of: the node stands its own shadow over the ground beside its band
    // as much as over the band, and at the top of the bar that ground is black.
    let mut scene = inked_on_grey(SHADOW, 0.5);
    // In close, and AIMED at the band's outer edge, so the stroke stands in
    // the middle of the pane with the band's ink to one side and the ground to
    // the other, both tens of pixels wide.
    scene.camera.distance = 4.0;
    scene.camera.target = glam::Vec3::new(scene.outer_outer * scene.marker_unit, 0.0, 0.0);
    let edge = on_screen(&scene, SIZE, scene.camera.target);
    let rect = [edge.x - NAME_SIZE / 2.0, edge.y - NAME_SIZE / 2.0, NAME_SIZE, NAME_SIZE];
    let bare = shooter.shot(&scene);
    let named = shooter.shot_with(&scene, a_name(vec![name_glyph(&scene, rect)]));

    let row = edge.y.round() as u32;
    // The stroke covers the pixels `left..right`; the pair `d` out is the
    // pixel whose CENTRE is `d - 0.5` beyond each edge, so the two stand at
    // one distance from the ink.
    let (left, right) = (rect[0].round() as u32, (rect[0] + rect[2]).round() as u32);
    let mut compared = 0;
    for d in 2..48u32 {
        let (x_in, x_out) = (left - d, right - 1 + d);
        let (bare_in, bare_out) =
            (bright_at(&bare, SIZE, x_in, row), bright_at(&bare, SIZE, x_out, row));
        // Only where the pair is the two RECEIVERS rather than one of them
        // twice — the stroke straddles the band's outer edge, so inward is ink
        // and outward is ground, and they read far apart — and only where each
        // has a brightness to take a share OF, a share of black being no
        // reading.
        if bare_in <= 20 || bare_out <= 20 || (bare_in - bare_out).abs() < 60 {
            continue;
        }
        let share = |shot: &[u8], x: u32, bare: i64| {
            1.0 - bright_at(shot, SIZE, x, row) as f64 / bare as f64
        };
        let (share_in, share_out) = (share(&named, x_in, bare_in), share(&named, x_out, bare_out));
        // A twentieth of the share: the band's ink is dark, so one level of
        // rounding there is a fiftieth on its own.
        assert!(
            (share_in - share_out).abs() < 0.05,
            "{d} px out, the name takes {share_in:.3} of the band's ink and {share_out:.3} of the \
             ground",
        );
        if share_out > 0.1 {
            compared += 1;
        }
    }
    assert!(compared >= 6, "only {compared} pairs of pixels stood inside the shadow on both sides");
}

/// A name is not darkened by its own shadow.
///
/// The shadow is drawn before the ink, and the blend's ink term is not
/// multiplied, so a name's letters are the one thing in the frame its shadow
/// never touches — and the wash they take is the RAW light, which its shadow
/// has not been through. Read at the top of the depth bar against the bottom
/// of it, on the name's whole pixels; the light is on, so a shadow finding its
/// way onto the ink would have something to take.
#[test]
fn a_name_is_not_darkened_by_its_own_shadow() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.3;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The name's whole pixels, read with no light in the picture: what it
    // drew, at the brightest colour in that set, which is its ink laid flat
    // (see `a_name_wears_the_wash_it_stands_in`). Under the light every pixel
    // of the name wears a different wash, so this is where the set is one
    // colour.
    let unlit = lit_node_and_a_name(0.0, SHADOW, 0.0);
    let bare_unlit = shooter.shot(&unlit);
    let named_unlit = shooter.shot_with(&unlit, one_name(&unlit, SIZE));
    let drawn: std::collections::BTreeSet<usize> = (0..bare_unlit.len())
        .step_by(4)
        .filter(|&i| named_unlit[i..i + 4] != bare_unlit[i..i + 4])
        .collect();
    let full: [u8; 3] =
        std::array::from_fn(|c| drawn.iter().map(|&i| named_unlit[i + c]).max().unwrap_or(0));
    let own: Vec<usize> =
        drawn.iter().copied().filter(|&i| named_unlit[i..i + 3] == full).collect();
    assert!(own.len() > 30, "the name covers {} whole pixels of its own", own.len());

    let flat = lit_node_and_a_name(1.6, SHADOW, 0.0);
    let named_flat = shooter.shot_with(&flat, one_name(&flat, SIZE));
    let deep = lit_node_and_a_name(1.6, SHADOW, 1.0);
    let bare_deep = shooter.shot(&deep);
    let named_deep = shooter.shot_with(&deep, one_name(&deep, SIZE));
    let moved = own.iter().filter(|&&i| named_deep[i..i + 3] != named_flat[i..i + 3]).count();
    assert_eq!(moved, 0, "the Shadow depth moved {moved} of the name's own {} pixels", own.len());
    // And the same depth darkens the ground round the name, so there was a
    // shadow here to keep off the ink.
    let dimmed = (0..bare_deep.len())
        .step_by(4)
        .filter(|i| !drawn.contains(i))
        .filter(|&i| brightness(&named_deep[i..i + 3]) < brightness(&bare_deep[i..i + 3]))
        .count();
    assert!(dimmed > 40, "the name's shadow darkened only {dimmed} pixels of ground");
}

/// Two facing strokes of one name cast deeper between them than either casts
/// alone at the same distance.
///
/// #490's crease, closed: a shadow is a blur of the ink and a blur is linear,
/// so the gap between two strokes holds both their ink and is darker than
/// either side of a lone stroke — where a nearest-distance field is a `min`,
/// so the second stroke contributed nothing and the midline was a crease.
///
/// The control is each stroke shot ALONE, and the claim is against the deeper
/// of the two pixel by pixel: a `max` of the two profiles is exactly what a
/// distance field draws, and comparing against one named stroke passes on a
/// column half a pixel off the midline with the `max` explaining the whole
/// difference.
#[test]
fn two_facing_strokes_cast_deeper_between_them_than_either_alone() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.6;
    /// A stroke, and the gap between the pair, in points. The gap is well
    /// under the blur's reach at this Shadow, which the fixture asserts below
    /// rather than assumes.
    const STROKE: [f32; 2] = [8.0, 18.0];
    const GAP: f32 = 8.0;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // Half depth: at the top of the bar the lone stroke's shadow is already
    // within a level of black at the midline, and darker than black is not a
    // reading.
    shooter.clear = over_grey_clear();
    let scene = inked_on_grey(SHADOW, 0.5);
    let at = on_screen(&scene, SIZE, NAME_AT);
    let stroke = |x: f32| name_glyph(&scene, [x, at.y - STROKE[1] / 2.0, STROKE[0], STROKE[1]]);
    let (left, right) = (stroke(at.x - GAP / 2.0 - STROKE[0]), stroke(at.x + GAP / 2.0));
    let bare = shooter.shot(&scene);
    let pair = shooter.shot_with(&scene, a_name(vec![left, right]));
    let left_alone = shooter.shot_with(&scene, a_name(vec![left]));
    let right_alone = shooter.shot_with(&scene, a_name(vec![right]));

    // Down the midline of the gap, through the strokes' middle rows.
    let x = at.x.round() as u32;
    let rows = (at.y - STROKE[1] / 2.0 + 3.0).round() as u32
        ..(at.y + STROKE[1] / 2.0 - 3.0).round() as u32;
    assert!(rows.len() >= 8, "the strokes are {} rows tall", rows.len());
    for y in rows {
        let (bare, pair, left, right) = (
            bright_at(&bare, SIZE, x, y),
            bright_at(&pair, SIZE, x, y),
            bright_at(&left_alone, SIZE, x, y),
            bright_at(&right_alone, SIZE, x, y),
        );
        // Each lone stroke's shadow reaches the midline: the fixture's gap is
        // inside the blur, or the pair is two shadows that never meet.
        assert!(left < bare - 6 && right < bare - 6, "at row {y} a lone stroke's shadow ({left}, {right}) does not reach the midline ({bare})");
        assert!(
            pair < left.min(right) - 6,
            "at row {y} the pair leaves {pair} between the strokes where the deeper stroke alone \
             leaves {}",
            left.min(right),
        );
    }
}

/// A name on a NEARER node shadows a farther node's rings, and a name on the
/// farther node does not shadow the nearer node's.
///
/// The case that is a per-NODE answer and nothing coarser: two nodes of ONE
/// sheet, overlapping on screen under an oblique camera, so anything that
/// groups casters by sheet would have to leave the pair alone. In the
/// painter's order the near node is drawn after the far node's name, so its
/// ink covers that name's shadow; its own name is drawn after everything and
/// lands on both.
///
/// The fixture asserts the overlap and that the far name's shadow WOULD have
/// reached the near node — the far node shot alone with its name darkens the
/// pixels the near node then covers — so the second claim is not a shadow that
/// never got there.
#[test]
fn a_name_on_a_nearer_node_shadows_a_farther_nodes_rings_and_not_the_reverse() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.4;
    /// How far apart the pair stands along the sheet, in world units — under
    /// the pitch below about a drawn disc on screen, so the far node's band
    /// keeps a crescent past the near node's ink.
    const APART: f32 = 4.4;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_grey_clear();
    // The pair, and the same scene holding any subset of it: a shot of one
    // node alone is the same picture with the other left out.
    let scene_of = |along: &[f32]| -> Scene {
        let mut scene = inked_on_grey(SHADOW, 1.0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Perspective,
            pitch: 1.1,
            distance: 9.0,
            target: glam::Vec3::new(0.0, APART / 2.0, 0.0),
            ..Default::default()
        };
        let node = scene.nodes[0];
        scene.nodes.clear();
        for &y in along {
            let mut at = node;
            at.world_pos = glam::Vec3::new(0.0, y, 0.0);
            scene.nodes.push(at);
        }
        rows_per_node(&mut scene);
        scene
    };
    let scene = scene_of(&[0.0, APART]);

    // Which of the two is nearer, in the terms the pass sorts by (`order` in
    // lib.rs), and where each stands on the pane.
    let eye = scene.camera.eye();
    let forward = (scene.camera.target - eye).normalize_or_zero();
    let depth = |i: usize| (scene.nodes[i].world_pos - eye).dot(forward);
    let (near, far) = if depth(0) < depth(1) { (0usize, 1usize) } else { (1, 0) };
    assert!(depth(near) < depth(far), "the pair stands at one depth");
    let centre = |i: usize| on_screen(&scene, SIZE, scene.nodes[i].world_pos);
    // The drawn disc's radius on screen: one quad uv, which is `marker_unit`
    // world units (see `name_on_the_band`).
    let radius = |i: usize| {
        let edge = scene.nodes[i].world_pos + glam::Vec3::X * scene.marker_unit;
        on_screen(&scene, SIZE, edge).distance(centre(i))
    };
    assert!(
        centre(near).distance(centre(far)) < radius(near) + radius(far),
        "the fixture's nodes must overlap on screen: {} apart at radii {} and {}",
        centre(near).distance(centre(far)),
        radius(near),
        radius(far),
    );
    // The near node's own OPAQUE pixels: those it paints the same over black as
    // over the grey, which is its ink and nothing else.
    let alone = scene_of(&[scene.nodes[near].world_pos.y]);
    let over_grey = shooter.shot(&alone);
    shooter.clear = wgpu::Color::BLACK;
    let over_black = shooter.shot(&alone);
    shooter.clear = over_grey_clear();
    let opaque: std::collections::BTreeSet<usize> = (0..over_grey.len())
        .step_by(4)
        .filter(|&i| over_grey[i..i + 4] == over_black[i..i + 4] && over_grey[i + 3] == 255)
        .collect();
    assert!(opaque.len() > 500, "the near node paints {} opaque pixels", opaque.len());
    let index = |p: glam::Vec2| ((p.y.round() as u32 * SIZE[0] + p.x.round() as u32) * 4) as usize;
    let pixel = |i: usize| {
        let px = (i / 4) as u32;
        glam::Vec2::new((px % SIZE[0]) as f32, (px / SIZE[0]) as f32)
    };

    // The far node's own ink: the band it paints, shot alone — the pixels far
    // darker than the ground, which leaves out the faint halo round the band.
    let without = scene_of(&[scene.nodes[far].world_pos.y]);
    let without_bare = shooter.shot(&without);
    let ground = brightness(&[(GREY * 255.0).round() as u8; 3]);
    let far_ink = |i: usize| (brightness(&without_bare[i..i + 3]) - ground).abs() > 150;

    // One stroke, on the far node's band where it comes out from under the
    // near node: the visible pixel of that band nearest to anything the near
    // node paints opaque, and the stroke stood a pixel off it on the far side.
    // As close to the near node as a visible stroke can stand, so its shadow
    // reaches what the near node paints — searched for rather than walked to,
    // because where the two discs meet is a sector gap's or a rim's business.
    let visible: Vec<glam::Vec2> = (0..without_bare.len())
        .step_by(4)
        .filter(|&i| far_ink(i) && !opaque.contains(&i))
        .map(pixel)
        .collect();
    let solid: Vec<glam::Vec2> = opaque.iter().map(|&i| pixel(i)).collect();
    assert!(visible.len() > 100, "the far node shows {} pixels of ink", visible.len());
    let nearest = |v: glam::Vec2| {
        *solid.iter().min_by(|a, b| a.distance(v).total_cmp(&b.distance(v))).expect("opaque")
    };
    let (spot, edge) = visible
        .iter()
        .map(|&v| (v, nearest(v)))
        .min_by(|(v, n), (w, m)| v.distance(*n).total_cmp(&w.distance(*m)))
        .expect("visible");
    let away = (spot - edge).normalize_or(glam::Vec2::Y);
    let at = spot + away * (NAME_SIZE / 2.0 + 1.0);
    assert!(
        far_ink(index(at)) && !opaque.contains(&index(at)),
        "the stroke at {at:?} does not stand on the far node's visible ink",
    );
    assert!(
        at.cmpgt(glam::Vec2::splat(NAME_SIZE)).all()
            && at.cmplt(glam::Vec2::splat(SIZE[0] as f32 - NAME_SIZE)).all(),
        "the stroke at {at:?} stands off the pane",
    );
    let rect = [at.x - NAME_SIZE / 2.0, at.y - NAME_SIZE / 2.0, NAME_SIZE, NAME_SIZE];
    let name =
        |scene: &Scene, node: usize| names(vec![(node as u32, vec![name_glyph(scene, rect)])]);

    let bare = shooter.shot(&scene);
    let near_named = shooter.shot_with(&scene, name(&scene, near));
    let far_named = shooter.shot_with(&scene, name(&scene, far));

    // The near name darkens the far node's rings — its ink, where the near
    // node does not cover it.
    let far_visible = |i: usize| far_ink(i) && !opaque.contains(&i);
    let onto_far = (0..bare.len())
        .step_by(4)
        .filter(|&i| {
            far_visible(i) && brightness(&near_named[i..i + 3]) < brightness(&bare[i..i + 3])
        })
        .count();
    assert!(onto_far > 20, "the near name darkened {onto_far} visible pixels of the far node");

    // The far name leaves the near node's opaque pixels alone, bar a level of
    // rounding: `opaque` is the pixels whose ALPHA reaches 255, and a coverage
    // a thousandth short of 1 lets a thousandth of what is behind through.
    let onto_near = opaque
        .iter()
        .filter(|&&i| (0..4).any(|c| far_named[i + c].abs_diff(bare[i + c]) > 1))
        .count();
    assert_eq!(onto_near, 0, "the far name's shadow reached {onto_near} pixels of the near node");
    // ...though its shadow does land there with the near node out of the way.
    let without_named = shooter.shot_with(&without, name(&without, 0));
    let would_have = opaque
        .iter()
        .filter(|&&i| brightness(&without_named[i..i + 3]) < brightness(&without_bare[i..i + 3]))
        .count();
    assert!(
        would_have > 20,
        "the far name's shadow reaches only {would_have} of the pixels the near node covers, so \
         the fixture is not measuring an occlusion",
    );
}

/// A name's shadow is the same width in POINTS at Render scale 2 as at 1.
///
/// #496: the cells are drawn in the target's pixels and σ is derived in them
/// (`shadow::sigma_px`), so a scale that doubles the pixels doubles σ with
/// them and the shadow lands where it did. The footprint is read at both
/// scales off the composite, which is at the pane's own size either way.
#[test]
fn a_names_shadow_is_the_same_width_in_points_at_render_scale_2() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.3;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_grey_clear();
    let mut footprint = |scale: f32| -> (std::collections::BTreeSet<usize>, f32) {
        let mut scene = inked_on_grey(SHADOW, 1.0);
        scene.render_scale = scale;
        let at = on_screen(&scene, SIZE, NAME_AT);
        let bare = shooter.shot(&scene);
        let named = shooter.shot_with(&scene, one_name(&scene, SIZE));
        let dimmed: std::collections::BTreeSet<usize> = (0..bare.len())
            .step_by(4)
            .filter(|&i| {
                let px = (i / 4) as u32;
                let p = glam::Vec2::new((px % SIZE[0]) as f32, (px / SIZE[0]) as f32);
                // Clear of the ink itself and its resampled edge.
                (p - at).abs().max_element() > NAME_SIZE / 2.0 + 1.5
                    && brightness(&named[i..i + 3]) + 6 < brightness(&bare[i..i + 3])
            })
            .collect();
        let reach = dimmed
            .iter()
            .map(|&i| {
                let px = (i / 4) as u32;
                glam::Vec2::new((px % SIZE[0]) as f32, (px / SIZE[0]) as f32).distance(at)
            })
            .fold(0.0f32, f32::max);
        (dimmed, reach)
    };
    let (at_one, reach_one) = footprint(1.0);
    let (at_two, reach_two) = footprint(2.0);
    assert!(at_one.len() > 100, "the shadow covers {} pixels at scale 1", at_one.len());
    assert!(
        (reach_one - reach_two).abs() <= 1.5,
        "the shadow reaches {reach_one} px at Render scale 1 and {reach_two} at 2",
    );
    let apart = at_one.symmetric_difference(&at_two).count();
    assert!(
        apart * 100 < at_one.len() * 15,
        "the two footprints differ at {apart} of {} pixels",
        at_one.len(),
    );
}

/// A name that casts no shadow — the Shadow's width or its depth at the bottom
/// of the bar — paints its ink and nothing else, and a name is the only thing
/// that puts a NAME's cell in the frame.
///
/// The vacuity the whole atlas rests on: with either bar shut there is no cell,
/// no pass and no box for anything, and the scene pass draws the name exactly
/// as it would with the atlas never built. The light is on, so a stray multiply
/// would have something to take.
#[test]
fn a_name_casting_no_shadow_paints_its_ink_and_nothing_else() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for (shadow, depth) in [(0.3f32, 0.0f32), (0.0, 1.0)] {
        let scene = lit_node_and_a_name(1.6, shadow, depth);
        let at = on_screen(&scene, SIZE, NAME_AT);
        let bare = shooter.shot(&scene);
        let named = shooter.shot_with(&scene, one_name(&scene, SIZE));
        let (mut inside, mut outside) = (0, 0);
        for i in (0..bare.len()).step_by(4) {
            if named[i..i + 4] == bare[i..i + 4] {
                continue;
            }
            let px = (i / 4) as u32;
            let p = glam::Vec2::new((px % SIZE[0]) as f32, (px / SIZE[0]) as f32);
            if (p - at).abs().max_element() <= NAME_SIZE / 2.0 + 1.0 {
                inside += 1;
            } else {
                outside += 1;
            }
        }
        assert!(inside > 30, "at Shadow {shadow}/{depth} the name inked {inside} pixels");
        assert_eq!(
            outside, 0,
            "at Shadow {shadow}/{depth} a name moved {outside} pixels outside its own ink",
        );
    }
    // And the cell count against the name itself: the node and the markers cast
    // too, so what a NAME is worth is the difference the name makes — one cell,
    // and only where there is a name to own it.
    let scene = lit_node_and_a_name(1.6, 0.3, 1.0);
    let cells = |labels| {
        LatticeCallback::from_scene(
            &scene,
            labels,
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            None,
        )
        .casters
        .len()
    };
    let bare = cells(LatticeLabels::default());
    assert_eq!(
        cells(one_name(&scene, SIZE)),
        bare + 1,
        "a name is worth exactly one cell over the {bare} the frame casts without it",
    );
}
