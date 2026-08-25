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
    near.gutter = 0.0;
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
    // rim: the fill alone answers the question, and a rim would spread the
    // reading over pixels nothing is being asked about.
    let bare = [TextRing::default(); 2];
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
                rings: bare,
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
            rings: [TextRing::default(); 2],
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
        call.seams,
        vec![
            // Both silent nodes: nothing has been drawn yet, and two labels
            // at one seam are one uninterrupted draw. They are behind the
            // home sheet, so they are also behind the markers.
            GlyphSeam { at: 0, start: 0, count: 2, after_pluses: false, sheet: 0 },
            // The home sheet's own name, after its disc.
            GlyphSeam { at: 1, start: 2, count: 1, after_pluses: true, sheet: 1 },
            // And the near sheet's, after everything.
            GlyphSeam { at: 2, start: 3, count: 1, after_pluses: true, sheet: 2 },
        ],
        "a label goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
}

/// Two names sharing a seam from different sheets draw nearer-last, and as
/// two draws — the one tie `at` and the side of the markers cannot break.
///
/// The last node of one sheet to ship and a node culled at the head of the
/// next sheet land on one `at`, the cull having moved nothing; with both
/// sheets on the same side of the markers, nothing else in the seam tells
/// them apart. The sort's fallback is then the order the labels ARRIVE in,
/// which is the scene's node order — lattice order, fixed whichever way the
/// camera is turned — so the farther sheet's name draws over the nearer's
/// whenever its node happens to come later in the lattice. And the two merge
/// into one rim-then-fill draw, so the nearer name's rim no longer separates
/// its glyphs from the farther name's fill where the two overlap, which on a
/// face-on sevens lattice they do exactly: a sevens node sits on top of its
/// home node.
///
/// The state is an ordinary one: a home node sounding with a silent node on
/// the sheet in front of it hovered. The hovered node draws nothing and is
/// named all the same.
#[test]
fn two_names_on_one_seam_from_different_sheets_draw_the_nearer_last_and_apart() {
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
    // The near sheet's node FIRST in the scene, so that lattice order and
    // depth order disagree: drawn back to front it is home, then near; the
    // home node ships and the silent near one does not, so both seams sit at
    // 1, past the markers.
    scene.nodes = vec![node(1.0, 0.0), node(0.0, 1.0)];
    let (near, home) = (0u32, 1u32);
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0)],
            labels: [near, home].map(|node| Label { node, glyphs: 1 }).to_vec(),
            rings: [TextRing::default(); 2],
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
    assert!(
        call.seams.iter().all(|s| s.at == 1 && s.after_pluses)
            && call.seams.iter().map(|s| s.count).sum::<u32>() == 2,
        "the two names share a seam, on the same side of the markers — the fixture \
         has to produce the tie for the assertions below to be about anything: {:?}",
        call.seams,
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 0.0],
        "the home sheet's name draws first and the nearer sheet's over it",
    );
    assert_eq!(
        call.seams.iter().map(|s| (s.start, s.count)).collect::<Vec<_>>(),
        vec![(0, 1), (1, 1)],
        "names from different sheets are two draws, the nearer's rim on the other's fill",
    );
}

/// A name on a node that ships no disc still draws over the markers, not
/// under them — the case where the two runs meet at the same number.
///
/// `pluses_at` is where the markers go, counted over the instances that SHIP.
/// The sheets behind the home one draw before them and the home sheet after,
/// so the boundary between the two runs is exactly `pluses_at`. A node that
/// paints nothing ships nothing, which leaves its seam sitting on the
/// boundary rather than past it: with every node on the home sheet — the
/// stock `extent_sevens: 0` — the far run is empty, `pluses_at` is 0, and the
/// first home node to be culled takes seam 0 as well. Reading the side off
/// `at > pluses_at` then files that node's name with the sheets BEHIND the
/// markers, and they are painted over the name.
///
/// The state is the plugin's resting one, which is what makes it worth a
/// test of its own: stock view, nothing played, hover any node. An idle node
/// draws nothing at all, and a hovered node is named whether or not it
/// draws.
#[test]
fn a_culled_home_nodes_name_draws_over_the_markers_it_shares_a_seam_with() {
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
    // The hovered one is silent and first, so it is culled before anything
    // has shipped and its seam is 0 — the same number as `pluses_at`.
    scene.nodes = vec![node(0.0), node(1.0)];
    let glyph = GlyphInstance { rect: [0.0, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph],
            labels: vec![Label { node: 0, glyphs: 1 }],
            rings: [TextRing::default(); 2],
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
    assert_eq!(call.pluses_at, 0, "with one sheet there is nothing to draw before the pluses");
    assert_eq!(
        call.seams,
        vec![GlyphSeam { at: 0, start: 0, count: 1, after_pluses: true, sheet: 0 }],
        "a home node's name draws after the pluses even when the cull leaves it on pluses_at",
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
                rings: [TextRing::default(); 2],
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
