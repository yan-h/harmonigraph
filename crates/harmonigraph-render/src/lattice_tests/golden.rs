//! Byte-exact frames: the one place in this crate where a build is compared
//! against another build.
//!
//! Every other test in this suite is a CLAIM test: it names a property and
//! measures it, so it passes for any picture holding that property. That is
//! the right shape for a dial whose look is still moving, and it is why five
//! shader terms survive the whole suite (#450) and why #453 could move the
//! frame by a mean of 3.3/255 with local swings of −90 while 146 tests stayed
//! green. Without these frames "behaviour-preserving" is an unverifiable
//! claim.
//!
//! The set is two kinds of scene, and the difference is worth keeping
//! straight because it decides what a diff MEANS.
//!
//! **Scenes a feature PR is not supposed to reach** — a node standing in its
//! own shadow, and the resting marker field standing in one node's light. A
//! Shimmer or a Wash change leaves both alone, so a diff here on such a PR is
//! the blast radius being wider than its author believed, which is the one
//! thing the claim tests cannot say.
//!
//! **Scenes the shadow rework (#498) is supposed to move**, each carrying one
//! phenomenon its design changes: a chord's overlapping halos, the live view,
//! a sheet standing behind a node, a name's own strokes meeting at a wide
//! Shadow (#490), a name standing on a node's band rather than in its empty
//! middle (the receiver asymmetry), the same at Render scale 2 (#496), and two
//! names on ONE sheet whose nodes cover each other (#469). These are here to
//! be MEASURED when they move, not to stay still: PRs B and C of #498 each
//! re-baseline them and state what moved.
//!
//! **A changed golden is a stated picture change.** Re-baseline with
//! `HARMONIGRAPH_BLESS=1 cargo test --workspace golden`, look at
//! the contact sheet it names, and say in the PR body what moved and why. The
//! comparison, the bless and the sheet are [`harmonigraph_golden::Gate`]; what
//! is here is the scenes.
//!
//! The frames are Metal-on-this-Mac specific. GitHub Actions is off and
//! `ci.sh` is the only gate, so that costs nothing today; a driver or OS
//! update re-baselines all of them at once, and its signature is every scene
//! moving by a small amount rather than one scene moving by a large one.

use super::fixtures::*;
use crate::*;
use harmonigraph_core::Tuning;
use harmonigraph_scene::Camera;

/// Wide enough that a marker's arms and the halo bridges between nodes are
/// several pixels across, and a multiple of 64 so `readback`'s 256-byte row
/// alignment holds.
const GOLDEN_SIZE: [u32; 2] = [256, 256];

/// One golden frame: a scene, the names standing on it, and what the pane is
/// filled with behind it.
pub(super) struct Shot {
    pub(super) scene: Scene,
    pub(super) labels: LatticeLabels,
    pub(super) clear: wgpu::Color,
}

impl From<Scene> for Shot {
    /// Over black, which is what every fixture in this suite but the
    /// whole-lattice ones is written against.
    fn from(scene: Scene) -> Shot {
        Shot { scene, labels: LatticeLabels::default(), clear: wgpu::Color::BLACK }
    }
}

// ---------------------------------------------------------------------------
// Scenes a feature PR is not supposed to reach
// ---------------------------------------------------------------------------

/// A node standing in its own shadow.
///
/// The subject is a node's own LAYERS and the order they stack in — the rings,
/// the band, the wedge a mark extends — together with what the whole of that
/// ink casts into the light around it, rather than any dial's look.
///
/// A mark is held, so the frame carries the bulge: a node's shape swells over
/// the wedge a mark extends and hugs the rings everywhere else.
fn a_node_in_its_own_shadow() -> Scene {
    let mut scene = layered_node(2, 0.6, true, 0.85);
    // The fixture's ground is white because its own readings are differences
    // against it. A golden is read as an absolute frame instead, and a
    // saturated channel records nothing: over white, 95% of these pixels
    // are pure black or pure white and no shader change can move them.
    scene.background = glam::Vec4::new(0.30, 0.31, 0.36, 1.0);
    // A halo over the whole frame, so the ground outside the node carries a
    // gradient rather than one flat value.
    scene.glow_reach = 4.0;
    scene.glow_strength = 2.0;
    scene
}

/// The resting marker field standing in one node's light.
///
/// This is where a marker's cross meets a node's halo: the ink, and the shadow
/// the cross casts into the light standing under it. The shape of that shadow
/// is what #450's disc-for-a-cross mutation changes and what no claim test in
/// the suite can see, so this frame is the one carrying the acceptance
/// criterion.
fn resting_markers_in_one_light() -> Scene {
    // The dials the marker suite measures shadows at, so the frame shows a
    // marker's shadow rather than a marker sitting in undimmed light.
    shadowed_markers(0.85, 0.5, 1.0)
}

// ---------------------------------------------------------------------------
// Scenes the shadow rework is supposed to move
// ---------------------------------------------------------------------------

/// The chord every whole-lattice golden is lit by — #453's own, so a diff here
/// is read against the A/B that measured the per-element refactor.
///
/// Five notes rather than one: under [`Tuning::default`]'s 12-TET they land on
/// nodes spread across the sheet, so the frame carries halos OVERLAPPING,
/// which is where a shadow model's combining rule shows.
const CHORD: [u8; 5] = [55, 60, 64, 67, 71];

/// The lattice `view` draws at `camera`, with [`CHORD`] held.
///
/// Through `derive_scene` rather than assembled by hand, which is the whole
/// point of the whole-lattice goldens: a Scene written out here is a SECOND
/// answer to what the shell computes, agreeing today and drifting on the first
/// change to `derive.rs`. What these frames are of is the app's picture.
///
/// 12-TET throughout, [`Tuning::default`]'s own, because that is the tuning a
/// bare MIDI chord is IN. A retuned lattice matches a plain integer note only
/// inside `tuning.tolerance`, half a cent — a quarter-comma fifth is 3.1 cents
/// off one and its third 12.4, so the same chord under it lights one node in
/// five and the frame is bare ground. The DAW's lattice gets its notes bent
/// onto its own nodes; nothing here bends them, and a tuning is not a look —
/// no draw path in this crate reads one.
fn lattice(view: &harmonigraph_scene::ViewConfig, camera: Camera) -> Scene {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for note in CHORD {
        tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
    }
    // The Fade off, so one frame is the whole picture rather than a shot of an
    // envelope part way through. The light's own clock is already out: nothing
    // has carried a `GlowStep` here, so every node reads its settled level.
    let frame = harmonigraph_scene::FrameParams { fade_time: 0.0, ..Default::default() };
    // Late enough that `mark_delay` has passed and the melody/bass marks are
    // in the frame — at 0.0 a chord held at 0.0 has earned none of them.
    const NOW: f64 = 1.0;
    let tuning = Tuning::default();
    let derive = |camera| {
        harmonigraph_scene::derive_scene(
            &tracker,
            &tuning,
            view,
            &view.reach(),
            &frame,
            camera,
            None,
            NOW,
        )
    };
    // Aimed at a lit NODE, in a second pass. Where the lit set sits depends on
    // the chord and on the window's own centre, so a camera pointed at the
    // lattice origin frames a different share of it in each scene here, and at
    // this zoom a frame can miss it entirely. At a NODE rather than at the lit
    // set's centroid, which for a scattered set is a position with nothing on
    // it — an empty centre in a frame this close is an empty frame.
    let settled = derive(camera);
    let at = settled
        .nodes
        .iter()
        .filter(|n| n.activation > 0.0)
        .min_by(|a, b| a.world_pos.length_squared().total_cmp(&b.world_pos.length_squared()))
        .expect("the chord must light a node inside the drawn window")
        .world_pos;
    let mut scene = derive(Camera { target: at, ..camera });
    // The ground a lattice pane paints its own rect with, which is what the
    // light lands on in the editor. `derive_scene` already answers this; it is
    // restated because [`on_the_ground_it_clears_to`] reads it back, and a frame
    // whose pane and whose scene disagree is the one thing that helper exists to
    // stop.
    scene.background = harmonigraph_scene::skin::well_color();
    scene
}

/// A frame standing on the ground its own scene names.
///
/// The pair has to agree: `Scene::background` is the ground the lattice pane
/// paints, so a lattice shot over a black pane stands on a ground the app has
/// nowhere on screen — and every whole-lattice frame here is such a shot.
fn on_the_ground_it_clears_to(scene: Scene) -> Shot {
    let ground = scene.background;
    Shot {
        clear: wgpu::Color {
            r: f64::from(ground.x),
            g: f64::from(ground.y),
            b: f64::from(ground.z),
            a: 1.0,
        },
        ..scene.into()
    }
}

/// Where the whole-lattice goldens stand: near enough that a node is tens of
/// pixels across at [`GOLDEN_SIZE`], so a ring and a marker's arms are both
/// several pixels wide.
///
/// [`Camera::DEFAULT_DISTANCE`] is 12, which fits about ten nodes across the
/// frame and leaves each one a dozen pixels — the halos would still be there
/// and nothing inside a node would be readable.
fn near_camera() -> Camera {
    Camera { distance: 4.0, ..Default::default() }
}

/// The fresh view, lit by a chord.
///
/// The reference frame for the family: every dial at the value a fresh blob
/// opens on, so a re-baseline here is the shipped look moving.
fn a_chord_at_the_fresh_view() -> Scene {
    lattice(&harmonigraph_scene::ViewConfig::default(), near_camera())
}

/// The same chord with the light spread wide enough that neighbouring halos
/// meet everywhere.
///
/// #453's fourth A/B scene, and the regime its mechanism 3 lives in: at the
/// fresh Reach the halos barely touch, so the rule two lights combine under is
/// invisible.
///
/// The Strength comes down as the Reach goes up: the light is SCREENED and
/// 12-TET lights every node of a pitch class in the window rather than the
/// five the chord names, so the overlaps run brighter than any one halo. Half
/// the bar is where this frame holds the most, and a golden is chosen for that
/// rather than for looking right — it spans 74 levels between its 5th and 95th
/// percentiles with nothing at either rail, so a change that dims the light
/// and a change that brightens it both have room to show. A frame clipped to
/// white records nothing, which is the same reason `a_node_in_its_own_shadow`
/// does not stand on the white ground its own fixture ships.
fn a_chord_at_a_wide_reach() -> Scene {
    let view = harmonigraph_scene::ViewConfig {
        glow_reach: 4.0,
        glow_strength: 0.5,
        glow_curve: harmonigraph_scene::GlowCurve::default(),
        ..Default::default()
    };
    lattice(&view, near_camera())
}

/// The lattice as Yan's DAW draws it.
///
/// Read out of a live Bitwig project with `./read-plugin-state.py` (the
/// `capture-daw-state` skill) on 2026-08-28. Of everything that capture holds,
/// three fields differ from the fresh view: a Shadow a fifth wider, the Shadow
/// depth at the top of its bar rather than 0.85, and the window centred one
/// fifth along.
///
/// That is the whole of the difference and it earns the frame, because the
/// freeze list and the shadow rework are both judged at these settings and
/// nowhere else — a picture change invisible at the fresh Shadow and obvious at
/// a wider one at full depth is a change that ships.
///
/// The capture's TUNING, zoom and pan are deliberately not taken. The tuning
/// would empty the frame (see [`lattice`]); zoom and pan are navigation state,
/// outside what a capture is read for at all, and his sits at the near end of
/// the zoom's travel where a frame this size holds one node's top half.
pub(super) fn the_live_view() -> Scene {
    let view = harmonigraph_scene::ViewConfig {
        center_threes: 1,
        glow_shadow: 0.196_915_06,
        glow_shadow_depth: 1.0,
        ..Default::default()
    };
    lattice(&view, near_camera())
}

/// The same live view drawn by the DISTANCE row.
///
/// The frame the second family is judged on, and the reason it is the live view
/// rather than a fixture: the two families are calibrated to the same reach and
/// differ in SHAPE, so what a row is worth is a question about the picture Yan
/// is actually looking at. A diff here on a later PR is the distance row moving
/// — which is what a freeze PR is for — and the Spread frames beside it stay
/// still, that being the contract this family arrived under.
fn the_live_view_on_the_distance_row() -> Scene {
    let mut scene = the_live_view();
    scene.glow_shadow_kernel = harmonigraph_scene::ShadowKernel::Distance;
    scene
}

/// The live view's DISTANCE row at the top of the Shadow bar.
///
/// Sigma-relative cell sizing is coarsest here: σ grows while its three-texel
/// representation stays fixed. This frame holds the resulting contours and
/// medial-axis softness at the end of the range where the performance gate is
/// measured.
fn the_live_view_at_the_top_of_the_distance_row() -> Scene {
    let mut scene = the_live_view_on_the_distance_row();
    scene.glow_shadow = harmonigraph_scene::GLOW_SHADOW_MAX;
    scene
}

/// The distance row at the default camera distance and full Shadow depth.
///
/// This is the scale where a node's curved ink is only a few pixels wide on
/// the pane. The frame holds whether its distance cell preserves that contour
/// rather than turning the decay outside it into rays.
fn the_zoomed_out_view_on_the_distance_row() -> Scene {
    let view = harmonigraph_scene::ViewConfig {
        center_threes: 1,
        glow_shadow_depth: 1.0,
        glow_shadow_kernel: harmonigraph_scene::ShadowKernel::Distance,
        ..Default::default()
    };
    lattice(&view, Camera::default())
}

/// A sevens sheet standing behind the home one.
///
/// The depth ordering the whole design turns on: a sheet is not a plane the
/// renderer draws, it is the nodes and resting markers at one sevens step, and
/// what puts them behind is their place in `order`. #459 asked for this frame;
/// #498 needs it because a caster on the home sheet has to darken what stands
/// on the sheet behind it, at whatever depth it darkens the ground.
fn a_sheet_behind_a_node() -> Scene {
    let view = harmonigraph_scene::ViewConfig { extent_sevens: 1, ..Default::default() };
    lattice(&view, near_camera())
}

/// How far the Shadow bar opens on a fresh blob — for the frames that are
/// about something else and want the bar where the picture has it.
fn fresh_shadow() -> f32 {
    harmonigraph_scene::ViewConfig::default().glow_shadow
}

/// Where the name goldens stand, and closer than the claim tests' own
/// `Camera::DEFAULT_DISTANCE`.
///
/// A profile walked along one row reads the same at any zoom; a frame a person
/// looks at does not. At 12 the fixture's node is a quarter of the frame
/// across and the rest is halo, so the glyph and the shadow around it — the
/// whole subject — are a handful of pixels.
const NAME_DISTANCE: f32 = 4.0;

/// One stroke of a golden's name, in NODE RADII, `x` across the band and `y`
/// along it.
///
/// Half the band's own depth across, so a lone stroke stands wholly on ink with
/// its shadow falling off both edges — which is the asymmetry
/// [`a_name_on_a_nodes_band`] is for. Longer than the band is deep along it, so
/// the stroke reads as a stroke rather than a dot at any of these sizes.
///
/// Node radii rather than points, so the shape holds if [`NAME_DISTANCE`] or
/// [`GOLDEN_SIZE`] moves — the Shadow bar is in node radii too, and a name
/// sized in points would slide out from under its own shadow.
const STROKE: glam::Vec2 = glam::Vec2::new(0.22, 0.55);

/// The Shadow [`two_strokes_of_one_name`] stands at, in node radii.
const WIDE_SHADOW: f32 = 0.6;

/// How far two strokes of one name stand apart, in node radii.
///
/// Narrower than two shadows of [`WIDE_SHADOW`] can meet across, or the frame
/// holds two separate shadows and says nothing about how they meet. Both sides
/// are in node radii, which is why the strokes are sized that way rather than
/// in points: the Shadow bar is, so the pair moves together under any camera —
/// and being constants, the reach is checked when the crate is built rather
/// than when the frame is drawn.
const STROKE_GAP: f32 = 0.18;
const _: () = assert!(
    STROKE_GAP < 2.0 * WIDE_SHADOW,
    "two strokes further apart than their shadows reach cast two shadows, not one meeting",
);

/// How large a whole name draws on a lattice frame, as a share of a node's
/// radius on the pane — about what the lattice typesets one at, and sized off
/// the node for [`STROKE`]'s reason.
const NAME_SHARE: f32 = 0.9;

/// The lit node the name frames stand on, framed on the BAND rather than on
/// the node.
///
/// The camera looks at where the name will stand, so the frame is the band's
/// arc through the middle with the node's inside to one side of it and bare
/// ground to the other — which is the comparison the frame is for. Aimed at
/// the node instead, the subject is a dozen pixels off in a corner and most of
/// the frame is the empty middle a name says nothing about.
fn a_named_node(shadow: f32) -> Scene {
    let mut scene = lit_node_and_a_name(1.6, shadow, 1.0);
    scene.camera.distance = NAME_DISTANCE;
    scene.camera.target = name_on_the_band(&scene);
    scene
}

/// `strokes` blocks of ONE name, centred on the middle of the node's octave
/// band and laid out ACROSS it.
///
/// Across rather than along, so a pair straddles the band: the channel between
/// two strokes runs along the band's own arc, and the shadows meeting in it
/// meet over the band's ink at one end and over the ground past its edge at the
/// other. Both receivers in one frame is what makes it worth a golden: one
/// multiply takes the same share off ink as off ground, and only a frame
/// carrying both can show that it does.
///
/// Blocks off the fixture atlas rather than letters: the atlas patch is an
/// opaque square (`text::tests::atlas`), so a stroke has straight sides and
/// the channel between two of them is a shape a reader of the contact sheet
/// can find.
fn strokes_on_the_band(scene: &Scene, strokes: usize) -> LatticeLabels {
    let at = on_screen(scene, GOLDEN_SIZE, name_on_the_band(scene));
    let unit = node_points(scene);
    let (w, h, gap) = (STROKE.x * unit, STROKE.y * unit, STROKE_GAP * unit);
    let span = strokes as f32 * w + (strokes - 1) as f32 * gap;
    let glyphs = (0..strokes)
        .map(|k| {
            let x = at.x - span / 2.0 + k as f32 * (w + gap);
            name_glyph(scene, [x, at.y - h / 2.0, w, h])
        })
        .collect();
    a_name(glyphs)
}

/// A node's own radius on the golden pane, in points — the unit every Shadow
/// in the picture is dialled in, spelled here as `from_scene` derives it.
fn node_points(scene: &Scene) -> f32 {
    scene.node_radius * scene.camera.points_per_world(GOLDEN_SIZE[1] as f32)
}

/// A name standing on the INK of the node it names, at the fresh Shadow.
///
/// Ink is washed with the RAW light, so what a name's shadow does to the band
/// under it is the same share it takes off the ground beside it — the
/// asymmetry #498 set out to remove, and the diff on this frame is where it
/// went.
///
/// The band rather than the middle: a node's middle is ground with a halo
/// standing on it, so a name there says nothing about what a shadow does to
/// INK.
fn a_name_on_a_nodes_band() -> Shot {
    let scene = a_named_node(fresh_shadow());
    let labels = strokes_on_the_band(&scene, 1);
    Shot { labels, ..scene.into() }
}

/// The same name at Render scale 2.
///
/// #496: the field a name's Shadow is measured in is sized at the RENDER
/// scale while the divisor that turns it into points is the DEVICE's, so a
/// name's shadow is suspected of coming out `1/S` of its dialled width while
/// every ring and cross keeps theirs. Nothing in the suite can see it, because
/// no claim test compares one Render scale against another; this frame records
/// what the build actually draws, and the PR that fixes it re-baselines here.
fn a_name_at_render_scale_2() -> Shot {
    let mut shot = a_name_on_a_nodes_band();
    shot.scene.render_scale = 2.0;
    shot
}

/// Two strokes of one name at a WIDE Shadow — #490's repro.
///
/// Between the bowl and the crossbar of a `G`, two strokes of one name cast
/// into the same pixels. A blur of the name's whole ink ADDS there, so the
/// ground between them is darker than beside either stroke alone — the reading
/// #490 asked for, and this frame is where it is on record.
fn two_strokes_of_one_name() -> Shot {
    let scene = a_named_node(WIDE_SHADOW);
    let labels = strokes_on_the_band(&scene, 2);
    Shot { labels, ..scene.into() }
}

/// Two names on ONE sheet, on nodes that cover each other on screen.
///
/// The case only the any-to-any design gets right (#498) and the one #469
/// measured: under an oblique camera two nodes of the same sevens step overlap,
/// so which of them shadows the other is a per-NODE answer. Anything that
/// groups casters by sheet — #497's layers, the sheet-ordered composite — has
/// to leave this pair alone.
///
/// Perspective and steeply pitched, which is what makes the overlap: a sheet's
/// rows are foreshortened by `cos(pitch)` and a node's diameter is half the
/// lattice spacing, so rows start covering each other past about 60°.
///
/// The pair is SEARCHED for rather than named, because which nodes light up is
/// the chord's and the window's answer, and a hand-picked index would silently
/// stop overlapping the first time either moved — the frame would still render
/// and would still be blessed.
pub(super) fn names_overlapping_on_one_sheet() -> Shot {
    let view = harmonigraph_scene::ViewConfig { extent_sevens: 1, ..Default::default() };
    let camera = Camera {
        projection: harmonigraph_scene::Projection::Perspective,
        pitch: 1.1,
        distance: 4.0,
        ..Default::default()
    };
    let scene = lattice(&view, camera);
    let pane = glam::Vec2::new(GOLDEN_SIZE[0] as f32, GOLDEN_SIZE[1] as f32);
    let projector = scene.projector(pane);
    let unit = node_points(&scene);
    let size = NAME_SHARE * unit;
    // How far each node stands along the view, which is what the pass sorts
    // nodes of one sheet by (`order` in lib.rs) — so the pair below is nearer
    // and farther in the same terms the picture is painted in.
    let eye = scene.camera.eye();
    let forward = (scene.camera.target - eye).normalize_or_zero();
    // Every node with ink whose NAME would land whole on the pane. An idle node
    // paints no pixel, so a name on one stands over nothing and says nothing
    // about a shadow landing on ink; a name half off the pane is a fixture
    // measuring its own clipping.
    let named: Vec<(usize, glam::Vec2, f32)> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.activation > 0.0)
        .filter_map(|(i, n)| {
            let at = projector.project(n.world_pos)?;
            let inside = at.cmpge(glam::Vec2::splat(size)).all()
                && at.cmple(pane - glam::Vec2::splat(size)).all();
            inside.then(|| (i, at, (n.world_pos - eye).dot(forward)))
        })
        .collect();
    // A node's own radius on the pane, measured by projecting a point one
    // radius out rather than scaled off the target plane's answer: under
    // perspective a far node draws smaller, which is the whole reason its
    // neighbours crowd it.
    let radius = |i: usize, at: glam::Vec2| {
        let n = &scene.nodes[i];
        let edge = n.world_pos + glam::Vec3::X * scene.node_radius * n.scale;
        projector.project(edge).map_or(0.0, |e| e.distance(at))
    };
    let pair = named
        .iter()
        .enumerate()
        .flat_map(|(k, a)| named[k + 1..].iter().map(move |b| (a, b)))
        .find(|((ia, pa, _), (ib, pb, _))| {
            scene.nodes[*ia].lattice_pos.sevens == scene.nodes[*ib].lattice_pos.sevens
                && pa.distance(*pb) < radius(*ia, *pa) + radius(*ib, *pb)
        });
    let (a, b) = pair.expect(
        "the fixture must put two lit nodes of ONE sheet over each other, both far enough \
         inside the pane to wear a name — widen the pitch, or bring the camera in",
    );
    // Nearer LAST, which is the order the pass paints them in and the whole
    // claim of the frame: the near node's name has to sit on the far node's
    // rings and not the other way round.
    let (far, near) = if a.2 > b.2 { (a, b) } else { (b, a) };
    assert!(
        far.2 > near.2,
        "the pair stands at one depth, so which of them is in front is a tie the picture \
         breaks arbitrarily and the frame claims nothing",
    );
    let glyph =
        |at: glam::Vec2| name_glyph(&scene, [at.x - size / 2.0, at.y - size / 2.0, size, size]);
    let labels =
        names(vec![(far.0 as u32, vec![glyph(far.1)]), (near.0 as u32, vec![glyph(near.1)])]);
    Shot { labels, ..on_the_ground_it_clears_to(scene) }
}

// ---------------------------------------------------------------------------
// Taking the shot
// ---------------------------------------------------------------------------

/// Draw `shot` and hold it against the frame on record.
///
/// A machine with no usable GPU adapter draws nothing and asserts nothing —
/// the same skip the rest of this suite takes.
fn check(name: &str, shot: Shot) {
    let Some(mut shooter) = Shooter::new(GOLDEN_SIZE) else {
        return;
    };
    shooter.clear = shot.clear;
    let actual = shooter.shot_with(&shot.scene, shot.labels);
    harmonigraph_golden::Gate::new(env!("CARGO_MANIFEST_DIR")).check(name, GOLDEN_SIZE, &actual);
}

/// A node standing in its own shadow is byte-identical to the frame on record.
///
/// A node's own layers and the shape of what they cast are what this frame
/// holds, so a diff here on a PR about how any one element LOOKS is reach its
/// author did not intend.
#[test]
fn a_node_in_its_own_shadow_draws_the_frame_on_record() {
    check("node-shadow", a_node_in_its_own_shadow().into());
}

/// The resting marker field in one node's light is byte-identical to the
/// frame on record.
///
/// Carries the shadow's SHAPE, which the four marker-shadow tests each miss
/// for their own reason (#450): a disc substituted for the cross passes all
/// of them and fails this.
#[test]
fn the_resting_marker_field_draws_the_frame_on_record() {
    check("resting-markers-in-one-light", resting_markers_in_one_light().into());
}

/// The fresh view under a chord is byte-identical to the frame on record.
#[test]
fn a_chord_at_the_fresh_view_draws_the_frame_on_record() {
    check("chord-fresh", on_the_ground_it_clears_to(a_chord_at_the_fresh_view()));
}

/// The same chord with the halos meeting everywhere.
#[test]
fn a_chord_at_a_wide_reach_draws_the_frame_on_record() {
    check("chord-wide-reach", on_the_ground_it_clears_to(a_chord_at_a_wide_reach()));
}

/// The lattice at the settings the DAW is actually carrying.
#[test]
fn the_live_view_draws_the_frame_on_record() {
    check("live-view", on_the_ground_it_clears_to(the_live_view()));
}

/// The same view drawn by the distance row.
#[test]
fn the_live_view_on_the_distance_row_draws_the_frame_on_record() {
    check("live-view-distance", on_the_ground_it_clears_to(the_live_view_on_the_distance_row()));
}

/// The coarsest Distance grid at the top of the Shadow bar stays on record.
#[test]
fn the_live_view_at_the_top_of_the_distance_row_draws_the_frame_on_record() {
    check(
        "live-view-distance-top",
        on_the_ground_it_clears_to(the_live_view_at_the_top_of_the_distance_row()),
    );
}

/// The distance row stays smooth when its ink projects to a handful of pixels.
#[test]
fn the_zoomed_out_view_on_the_distance_row_draws_the_frame_on_record() {
    check(
        "zoomed-out-distance",
        on_the_ground_it_clears_to(the_zoomed_out_view_on_the_distance_row()),
    );
}

/// A sevens sheet behind the home one.
#[test]
fn a_sheet_behind_a_node_draws_the_frame_on_record() {
    check("sheet-behind-a-node", on_the_ground_it_clears_to(a_sheet_behind_a_node()));
}

/// A name standing on its node's band.
#[test]
fn a_name_on_a_nodes_band_draws_the_frame_on_record() {
    check("name-on-a-band", a_name_on_a_nodes_band());
}

/// The same name at Render scale 2.
#[test]
fn a_name_at_render_scale_2_draws_the_frame_on_record() {
    check("name-at-render-scale-2", a_name_at_render_scale_2());
}

/// Two strokes of one name at a wide Shadow.
#[test]
fn two_strokes_of_one_name_draw_the_frame_on_record() {
    check("two-strokes-at-a-wide-shadow", two_strokes_of_one_name());
}

/// Two names on one sheet, on nodes that cover each other.
#[test]
fn names_overlapping_on_one_sheet_draw_the_frame_on_record() {
    check("names-overlapping-on-one-sheet", names_overlapping_on_one_sheet());
}
