//! The hole a node cuts in everything drawn behind it.

use super::fixtures::*;
use crate::*;

/// How far the light in `weights` reaches from the centre of the frame within
/// `cone` degrees of `toward`, in pixels.
///
/// One direction's radius, where [`Light::far`] is the largest over every
/// direction at once — which is the whole question about a shape that is no
/// longer a circle.
fn far_toward(weights: &[f64], size: [u32; 2], toward: f64, cone: f64) -> f64 {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let mut far = 0.0f64;
    for (i, &w) in weights.iter().enumerate() {
        if w < RING_LIT {
            continue;
        }
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        if angle_apart(y.atan2(x), toward) <= cone {
            far = far.max(x.hypot(y));
        }
    }
    far
}

/// A node's clearing is the node's own SHAPE one reach out, so a melody mark
/// pushes the hole out over the wedge it extends and nowhere else.
///
/// The circle this replaces is sized to hold the node whichever direction it
/// reaches furthest in, and a mark reaches a whole strip further than the rings
/// do: a marked node cleared a gap wider than itself all the way round, so a
/// hole that says "this node is in front of that one" said it about a ring of
/// empty lattice too. That is visible exactly where the clearing is for — over
/// the resting markers and the sheets behind — and invisible in the node's own
/// picture, which is why every reading here is off the difference the gutter
/// makes rather than off the node.
#[test]
fn a_clearing_bulges_over_the_mark_and_hugs_the_rings_everywhere_else() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();
    let mark_out = rings.mark_inner + rings.mark_thickness;

    let bare_plain = gpu.shot(&clearing_node(0, 1.0, true, 0.0));
    let holed_plain = gpu.shot(&clearing_node(0, 1.0, true, CLEAR_REACH));
    let bare_marked = gpu.shot(&clearing_node(MIDDLE_C, 1.0, true, 0.0));
    let holed_marked = gpu.shot(&clearing_node(MIDDLE_C, 1.0, true, CLEAR_REACH));

    // Which way the mark points, taken off the picture: the marked node over the
    // same node with its mark off.
    let mark = light_about_center(&light_over(&bare_marked, &bare_plain), SIZE);
    assert!(mark.weight > 0.0, "the mark drew nothing to aim at");
    let away = mark.angle + std::f64::consts::PI;

    let plain = light_over(&holed_plain, &bare_plain);
    let marked = light_over(&holed_marked, &bare_marked);
    // A cone inside the wedge the mark extends — a full-size slice of the fresh
    // wheel is 55 degrees, so ±27 — and wide enough to hold the rounding the
    // dilation puts on its corners.
    const CONE: f64 = 15.0;
    let plain_far = far_toward(&plain, SIZE, mark.angle, CONE);
    let marked_far = far_toward(&marked, SIZE, mark.angle, CONE);
    let plain_back = far_toward(&plain, SIZE, away, CONE);
    let marked_back = far_toward(&marked, SIZE, away, CONE);
    assert!(plain_far > 0.0 && marked_far > 0.0, "no clearing to measure");

    // Every length below is in pixels, so the picture calibrates itself: the
    // unmarked hole IS the rings' edge one reach out, and that is a uv the
    // stack states.
    let scale = plain_far / (rings.outer + CLEAR_REACH) as f64;
    let want = (mark_out - rings.outer) as f64 * scale;
    eprintln!(
        "toward the mark {plain_far:.1} -> {marked_far:.1} px, away {plain_back:.1} -> \
         {marked_back:.1} px; the strip is {want:.1} px at {scale:.1} px/uv",
    );
    assert!(
        (marked_far - plain_far - want).abs() < 2.0,
        "the mark pushed the hole out {:.1} px over its own wedge, not the {want:.1} px \
         its strip stands past the rings",
        marked_far - plain_far,
    );
    // The other half of the claim, and the one the circle fails: a mark on one
    // octave is not a wider node.
    assert!(
        (marked_back - plain_back).abs() < 1.5,
        "the hole is {:.1} px wider away from the mark than the unmarked node's, \
         so the mark widened the clearing all the way round",
        marked_back - plain_back,
    );
}

/// A mark's wedge can run past a HALF turn, and `sector_distance` has to stay
/// exact when it does.
///
/// `MIN_SPAN` rules out a slice that is a whole turn and nothing narrower, so a
/// wheel of one full-size octave between two extras at their minimum cuts a
/// slice most of the way round — a shape the Octaves bar can be dialled into.
/// Past a half-aperture of pi/2 the sector's two edge half-planes stop
/// intersecting in front of the wedge and start intersecting behind it, which
/// is the case a naive `max` of two half-planes gets wrong: it would take the
/// clearing back to the rings across the far side of a wedge that covers it.
///
/// Measured off the wedge's own middle rather than a fixed angle, so it does
/// not matter which slot the wheel made the wide one. Only the covering half of
/// the claim is made here: the extras leave a gap narrower than twice the
/// reach, so a hole this wide legitimately has no direction left in which it
/// hugs the rings. That half is
/// `a_clearing_bulges_over_the_mark_and_hugs_the_rings_everywhere_else`, on a
/// wheel whose slices are 55 degrees.
#[test]
fn a_clearing_over_a_wedge_past_a_half_turn_covers_the_whole_wedge() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();
    let mark_out = rings.mark_inner + rings.mark_thickness;

    // One full-size octave and two extras at MIN_EXTRA_SIZE, so the full one
    // takes 1/(1 + 2*0.1) of the turn.
    let wheel =
        harmonigraph_scene::octave_layout(1, 60.0, 1, harmonigraph_scene::MIN_EXTRA_SIZE, 1.0);
    let widest = (1..=wheel.span)
        .map(|j| wheel.bounds[j as usize] - wheel.bounds[j as usize - 1])
        .fold(0.0f32, f32::max);
    eprintln!(
        "span {} of {} full + {} extras, widest slice {:.1} deg",
        wheel.span,
        wheel.count,
        wheel.extras,
        widest.to_degrees(),
    );
    assert!(
        widest > std::f32::consts::PI,
        "the fixture has to cut a slice past a half turn to be testing anything; \
         the widest is {:.1} deg",
        widest.to_degrees(),
    );
    let lopsided = |melody: u32, gutter: f32| -> Scene {
        let mut scene = clearing_node(melody, 1.0, true, gutter);
        scene.octave_layout = wheel;
        scene
    };

    let bare_plain = gpu.shot(&lopsided(0, 0.0));
    let holed_plain = gpu.shot(&lopsided(0, CLEAR_REACH));
    let bare_marked = gpu.shot(&lopsided(MIDDLE_C, 0.0));
    let holed_marked = gpu.shot(&lopsided(MIDDLE_C, CLEAR_REACH));

    // Which way the wide wedge points, taken off the picture as the marked
    // node's own extra ink.
    let mark = light_about_center(&light_over(&bare_marked, &bare_plain), SIZE);
    assert!(mark.weight > 0.0, "the mark drew nothing to aim at");

    let plain = light_over(&holed_plain, &bare_plain);
    let marked = light_over(&holed_marked, &bare_marked);
    // Narrow, because the two extras between them hold only what the full
    // slice leaves and the far reading has to stay inside that.
    const CONE: f64 = 8.0;
    let scale = far_toward(&plain, SIZE, mark.angle, CONE) / (rings.outer + CLEAR_REACH) as f64;
    let strip = (mark_out - rings.outer) as f64 * scale;

    // The wedge's middle, then a quarter turn off it, then most of the way out
    // to its edge — the last two past the half-aperture where a wedge stops
    // being an intersection of two half-planes in front of itself.
    for turn in [0.0_f64, 90.0, 150.0] {
        let toward = mark.angle + turn.to_radians();
        let grew = far_toward(&marked, SIZE, toward, CONE) - far_toward(&plain, SIZE, toward, CONE);
        eprintln!(
            "{turn:.0} deg off the wedge's middle: the hole grew {grew:.1} px, want {strip:.1}",
        );
        assert!(
            (grew - strip).abs() < 2.0,
            "{turn:.0} degrees off the middle of a {:.0}-degree wedge the hole grew {grew:.1} px, \
             not the {strip:.1} px its strip stands past the rings — the wedge does not \
             reach its own edge",
            widest.to_degrees(),
        );
    }
}

/// The fresh view's own stack, which unlike [`clearing_rings`] is seated a good
/// way OFF the node's centre ([`ViewConfig::ring_inner`](harmonigraph_scene::ViewConfig))
/// and so leaves the empty middle the readings below are about.
///
/// The probe stack seats itself at 0, where the innermost ring is a disc and a
/// node has no middle at all — which is what the other clearing tests want,
/// their readings being about a layer's thickness. Nothing here reads one.
fn middled_rings() -> harmonigraph_scene::RingStack {
    harmonigraph_scene::ViewConfig::default().rings()
}

/// [`clearing_node`] re-seated on [`middled_rings`], so the node it stages has
/// an empty middle.
fn middled_node(melody: u32, ring: f32, band: bool, gutter: f32) -> Scene {
    let rings = middled_rings();
    let mut scene = clearing_node(melody, ring, band, gutter);
    (scene.spectral.inner, scene.spectral.outer) = rings.audio;
    (scene.outer_inner, scene.outer_outer) = if band { rings.band } else { (0.0, 0.0) };
    scene.rings_outer = if band { rings.band.1 } else { rings.audio.1 };
    scene.mark_inner = scene.rings_outer + rings.gap;
    scene.mark_thickness = rings.mark_thickness;
    // No markers. The ray sweep below reads the picture for anything lit, and a
    // marker sitting just outside the hole is a lit sample past its rim with
    // bare pane between — which reads as a gap in the hole and is nothing of
    // the kind. What the clearing cuts out of the marker field is a separate
    // claim, and `a_node_wearing_only_an_audio_ring_clears_around_it` is where
    // the added light is measured against it.
    scene.pluses.clear();
    scene
}

/// The clearing is the node's RINGS dilated by the reach, and it is a HOLE
/// across them: continuous from the innermost ring's inner edge outward, over
/// the gaps between one ring and the next and between one sector and the next,
/// and stopping one reach short of that inner edge rather than running on to
/// the node's centre.
///
/// Both halves fail differently, and one reading catches both.
///
/// Filled INWARD, a node is an opaque disc the size of its outermost ring: a
/// node standing anywhere in that circle is erased down to the ground and the
/// light over it, however far off the front node's own rings it stands. That is
/// most of what a node covers, the fresh middle being over half its radius, and
/// it is the picture the clearing exists to avoid rather than to make.
///
/// Cut back to the INK instead, the lattice would show through every gap on the
/// node, which reads as neither a hole nor a node.
///
/// RADIALLY, which is the direction this sweep reads and the one the Shadow closes:
/// the padding between one ring and the next is a length the Shadow outreaches at
/// the fixture's own settings, so the hole is one band across the whole stack.
/// The ANGULAR direction is the other question and is answered the other way —
/// the hole is cut back to the slices there, and a padding wide enough to
/// outreach the Shadow opens it (`a_wide_octave_gap_opens_the_hole_in_the_sectors
/// _the_node_leaves_empty`). This fixture sits at the shipped Octave gap, where
/// the gaps are a fraction of a Shadow wide and the hole closes over every one.
///
/// So the two numbers are where the hole STARTS and whether it has a gap in it.
/// The first is read off the clearing's own footprint — the difference the
/// gutter makes, over a ground the fixture paints white — and calibrated in the
/// same picture, its outer edge being a uv the stack states. The second is read
/// along rays out of the node's centre, the one sweep that does not need to
/// know where the rim is in each direction: everything from the hole's inner
/// edge out to whatever the ray last found is the node or the ground, and a
/// dark sample with light beyond it is a hole in the hole.
#[test]
fn a_clearing_covers_every_ring_and_leaves_the_middle_alone() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = middled_rings();
    // Everything a node can wear, and then the same node with its audio ring
    // gated shut — which leaves the octave band the innermost thing drawn, and
    // so a middle wider by the whole of the ring's slot.
    for (name, melody, ring, first, last) in [
        ("every layer", MIDDLE_C, 1.0, rings.inner, rings.mark_inner + rings.mark_thickness),
        ("the octave band alone", 0, 0.0, rings.band.0, rings.band.1),
    ] {
        let bare = gpu.shot(&middled_node(melody, ring, true, 0.0));
        let holed = gpu.shot(&middled_node(melody, ring, true, CLEAR_REACH));
        let hole = light_about_center(&light_over(&holed, &bare), SIZE);
        assert!(hole.far > 8.0, "{name}: no clearing to read, {:.1} px", hole.far);

        // Pixels per uv, off the hole's own outer edge: the furthest the
        // clearing reaches is the outermost layer this node wears, one reach
        // out. Taken from the picture rather than from the camera, exactly as
        // `a_clearing_bulges_over_the_mark_and_hugs_the_rings_everywhere_else`
        // does.
        let scale = hole.far / (last + CLEAR_REACH) as f64;
        let want = (first - CLEAR_REACH) as f64 * scale;
        eprintln!(
            "{name}: the hole runs {:.1}..{:.1} px, the rings {:.1}..{:.1} px at \
             {scale:.1} px/uv",
            hole.near,
            hole.far,
            first as f64 * scale,
            last as f64 * scale,
        );
        assert!(
            (hole.near - want).abs() < 2.5,
            "{name}: the hole starts {:.1} px out, not the {want:.1} px that is one reach \
             inside the innermost ring — at 0 it is filled to the node's centre",
            hole.near,
        );

        let (cx, cy) = ((SIZE[0] - 1) as f64 / 2.0, (SIZE[1] - 1) as f64 / 2.0);
        // Half-pixel steps: a step of a whole one can straddle the rim and read
        // a gap the picture does not have.
        let steps = (hole.far * 2.0).ceil() as usize;
        let lit = |r: f64, a: f64| -> bool {
            let x = (cx + r * a.cos()).round();
            let y = (cy + r * a.sin()).round();
            let i = (y as usize * SIZE[0] as usize + x as usize) * 4;
            brightness(&holed[i..i + 4]) >= 24
        };
        // From two pixels inside the hole's own inner edge, which is where the
        // sweep's claim starts — the middle beneath it is the picture behind
        // showing through, and is the assertion above.
        let from = ((want + 2.0).max(0.0) * 2.0) as usize;
        for turn in 0..360 {
            let a = (turn as f64).to_radians();
            let Some(rim) = (from..=steps).rev().map(|s| s as f64 / 2.0).find(|&r| lit(r, a))
            else {
                continue;
            };
            // Stopping two pixels short of the rim, which is the hole's own
            // anti-aliased edge: out there a lobe's angular boundary and the
            // rounding of a sample to a pixel can disagree, so a single lit
            // sample sits past a dark one and reads as a gap the picture does
            // not have. Everything a gap in the hole would actually be is
            // inside this.
            let gap = (from..=((rim - 2.0).max(0.0) * 2.0) as usize)
                .map(|s| s as f64 / 2.0)
                .find(|&r| !lit(r, a));
            assert!(
                gap.is_none(),
                "{name}: at {turn} degrees the picture is dark {:.1} px out and lit \
                 again at {rim:.1} px — the clearing has a gap across its rings",
                gap.unwrap_or_default(),
            );
        }
    }
}

/// Mean added light over the pixels between `lo` and `hi` from the centre of the
/// frame — how STRONGLY a ring of the clearing is cleared, where
/// [`far_toward`] and [`Light::far`] answer how far it reaches.
fn light_in_band(weights: &[f64], size: [u32; 2], lo: f64, hi: f64) -> f64 {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let (mut sum, mut n) = (0.0, 0usize);
    for (i, &w) in weights.iter().enumerate() {
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        let r = x.hypot(y);
        if r >= lo && r <= hi {
            sum += w;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// The audio ring is worn node by node, so its part of the clearing is too: a
/// node the Gate has closed clears only the core it is left with, and one part
/// way through its fade clears the ring's WHOLE hole at part of its strength.
///
/// Width from the layer, level from the layer's own fade — the same division the
/// note's clearing has always run on, now per layer. The two halves are separate
/// claims and they fail differently. A hole sized by the fade would sweep
/// outward across the lattice as a ring arrives, which is the "node retreating"
/// look the reach is deliberately held against; a hole at full strength from the
/// first frame would pop.
#[test]
fn a_clearing_follows_the_audio_ring_its_node_wears() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();

    // The band off, so the audio ring is the layer the stack's cursor landed on
    // and this node's own gate is what its hole answers to.
    let hole = |gpu: &mut Shooter, ring: f32| -> (Vec<f64>, f64) {
        let bare = gpu.shot(&clearing_node(0, ring, false, 0.0));
        let holed = gpu.shot(&clearing_node(0, ring, false, CLEAR_REACH));
        let cleared = light_over(&holed, &bare);
        let far = light_about_center(&cleared, SIZE).far;
        (cleared, far)
    };
    let (worn, worn_far) = hole(&mut gpu, 1.0);
    let (closed, closed_far) = hole(&mut gpu, 0.0);
    let (half, half_far) = hole(&mut gpu, 0.5);

    let scale = worn_far / (rings.audio.1 + CLEAR_REACH) as f64;
    eprintln!(
        "ring worn {worn_far:.1} px, closed {closed_far:.1}, half {half_far:.1}, \
         at {scale:.1} px/uv",
    );
    // The band is off and the note is silent, so a node the gate closed is
    // wearing nothing at all and has nothing to clear. That is the per-layer
    // split at its limit: a hole sized to what the VIEW has on would still cut
    // a ring-sized gap here, around ink nobody drew.
    assert!(
        closed_far < 2.0,
        "a node the gate closed still clears {closed_far:.1} px, with no layer on it",
    );
    assert!(
        (half_far - worn_far).abs() < 2.0,
        "a ring half way in clears {half_far:.1} px where a whole one clears \
         {worn_far:.1} — the hole is sized by the fade instead of by the layer",
    );

    // Read where only the ring's own clearing lands: outside the ring's ink, so
    // the node paints nothing there and the added light IS the hole, and inside
    // the reach, so a hard-edged clearing covers all of it.
    let (lo, hi) = (rings.audio.1 as f64 * scale + 3.0, worn_far - 3.0);
    let (lit, dim, none) = (
        light_in_band(&worn, SIZE, lo, hi),
        light_in_band(&half, SIZE, lo, hi),
        light_in_band(&closed, SIZE, lo, hi),
    );
    eprintln!("past the ring, {lo:.1}..{hi:.1} px: worn {lit:.0}, half {dim:.0}, closed {none:.0}");
    assert!(
        lit > 0.0 && (dim / lit - 0.5).abs() < 0.05,
        "half a ring cleared {dim:.0} of the {lit:.0} a whole one does",
    );
    assert!(none < lit * 0.02, "a closed gate cleared {none:.0} past a ring it is not wearing");
}

/// A node wearing NOTHING BUT an audio ring clears around it — the case the
/// whole per-layer split is for.
///
/// The ring is a window onto the spectrum rather than a level a node carries, so
/// a node nobody played wears one wherever the view's Gate lets it. That is ink,
/// and ink with no hole under it reads as painted ON the lattice rather than in
/// front of it: the marker under it shows through the ring, and so do the
/// sheets behind.
///
/// The other half is what such a node must NOT clear. Its band and its core are
/// drawn at the note's level, which is nothing, so a hole sized to the layers
/// the VIEW has on would clear a band-sized gap around ring-sized ink — the
/// same "wider than the node" failure the marks had, arrived at from the other
/// direction.
#[test]
fn a_node_wearing_only_an_audio_ring_clears_around_it() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();

    // Silent: no note, no octaves, no marks. What is left is the ring the gate
    // hands it, at `ring`, and the band is ON so the "clears what the view draws
    // rather than what this node draws" failure has room to show.
    let silent = |ring: f32, gutter: f32| -> Scene {
        let mut scene = clearing_node(0, ring, true, gutter);
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        scene
    };
    let bare = gpu.shot(&silent(1.0, 0.0));
    let holed = gpu.shot(&silent(1.0, CLEAR_REACH));
    let cleared = light_over(&holed, &bare);
    let hole = light_about_center(&cleared, SIZE);
    assert!(hole.weight > 0.0, "a node wearing an audio ring cleared nothing at all");

    // Calibrated on a node that IS played, where the hole is the band's and the
    // band's outer edge is a uv the stack states.
    let played_bare = gpu.shot(&clearing_node(0, 1.0, true, 0.0));
    let played = gpu.shot(&clearing_node(0, 1.0, true, CLEAR_REACH));
    let played_far = light_about_center(&light_over(&played, &played_bare), SIZE).far;
    let scale = played_far / (rings.band.1 + CLEAR_REACH) as f64;
    let want = (rings.audio.1 + CLEAR_REACH) as f64 * scale;
    eprintln!(
        "ring alone clears {:.1} px (want {want:.1}), a played node {played_far:.1} \
         (band {:.1}), at {scale:.1} px/uv",
        hole.far,
        (rings.band.1 + CLEAR_REACH) as f64 * scale,
    );
    assert!(
        (hole.far - want).abs() < 2.0,
        "a node wearing only its ring cleared {:.1} px, not the {want:.1} px that ring \
         reaches — a band nobody is drawing is in the hole",
        hole.far,
    );

    // ...and with the gate closed there is no ink and nothing to clear around.
    // That half is the CULL's, and it is asked of the cull directly: a node
    // with no note, no marks and no ring ships no instance, so its reach never
    // reaches the shader. Two shots would prove nothing here — the node is gone
    // from both, and comparing two empty images passes whatever the shader
    // does. The shader's own half of the answer, a closed gate falling back to
    // the layers that ARE drawn, is measured in
    // `a_clearing_follows_the_audio_ring_its_node_wears`.
    let quiet = LatticeCallback::from_scene(
        &silent(0.0, CLEAR_REACH),
        LatticeLabels::default(),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        31,
        None,
    );
    assert!(
        quiet.instances.is_empty(),
        "a node with no note and no ring still shipped, carrying a reach that would \
         clear a hole around ink nobody draws",
    );
    // Its knockout goes with it. That half ships off a list of its own and so
    // could be culled apart from the ink it belongs to, which would put a hole
    // in the marker field around a node drawing nothing.
    assert!(
        quiet.clearings.is_empty(),
        "a node the cull dropped still shipped the hole it would have cleared",
    );
}

/// An octave band the stack switched off — the empty pair (0, 0) — paints
/// NOTHING, rather than a dot at the node's centre.
///
/// `glyph_band` is two soft edges multiplied together, and at inner == outer
/// they cross instead of cancelling: a pixel at the node's centre is half inside
/// each, so the layer answers a quarter coverage where the whole point of the
/// pair was that there is no layer. It is the one radius pair whose arithmetic
/// draws a shape, which is why the shader gates the band on `band_out > band_in`
/// rather than trusting the geometry to say off by drawing nothing.
///
/// Measured against a frame with NO node in it, because the artifact is what a
/// node with every layer off still paints. Differencing two shots that both
/// carry the band — how `a_mark_stands_off_the_outermost_ring_the_node_draws`
/// reads the mark out — is exactly what cannot see this: the dot is in both and
/// cancels.
#[test]
fn a_band_dialled_off_paints_no_dot_at_the_nodes_centre() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Every layer of the node off: no core, no audio ring, no marks, and the
    // octave band at the empty pair the stack hands over when it cannot fit the
    // layer. What is left for the node to draw is nothing — and with `node`
    // false, the same frame with no node in it at all, which is the ground.
    //
    // The padding is 0 because the sector gaps all CONVERGE on the node's
    // centre, which is where the artifact is: at the fresh gap they eat most of
    // it, and the dot this is looking for shows at its own size.
    let collapsed = |node: bool| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.mark_thickness = 0.0;
        scene.rings_outer = 0.0;
        scene.octave_gap = 0.0;
        (scene.outer_inner, scene.outer_outer) = (0.0, 0.0);
        scene.spectral = harmonigraph_scene::SpectralPaint::silent();
        if !node {
            scene.nodes.clear();
        }
        scene
    };

    // Any deviation at all, rather than a threshold: what the node is allowed
    // to paint here is nothing, so the ground IS the answer, pixel for pixel.
    // The dot is dim as well as small — six pixels at up to 5/255 over black,
    // which is under a fifth of what `light_about_center` calls lit — so a
    // brightness floor would read it as absent.
    let ground = gpu.shot(&collapsed(false));
    let painted = gpu.shot(&collapsed(true));
    let px = differing_pixels(&painted, &ground);
    let light = light_about_center(&light_over(&painted, &ground), SIZE);
    eprintln!("a node with every layer off: {px} px, {:.0} of light", light.weight);
    assert_eq!(px, 0, "a node with every layer off painted {px} px, {:.0} of light", light.weight,);

    // Not a vacuous fixture: hand the same node a real annulus and the same
    // measurement finds it. Without this, a node that drew nothing for some
    // other reason — discarded, off screen, black on black — would read as the
    // empty pair being honoured.
    let mut drawn = collapsed(true);
    (drawn.outer_inner, drawn.outer_outer) = (0.4, 0.7);
    drawn.rings_outer = 0.7;
    let band = light_about_center(&light_over(&gpu.shot(&drawn), &ground), SIZE);
    assert!(band.weight > 0.0, "the fixture paints no band even when it is given one");
}

/// Drawing a home node's knockout as a separate pass paints the picture the one
/// draw it was split out of painted.
///
/// The split is a premultiplied over factored in two — `ground*g` at alpha `g`,
/// then the ink at alpha `a`, against the `ink + ground*g*(1-a)` at alpha
/// `a + g*(1-a)` a single draw writes — so it owes the same picture rather than
/// a similar one, and the markers it makes room for are what it is spent on.
///
/// What it does NOT owe is the same bytes: an 8-bit target rounds once per
/// composite, so a fragment written in two steps lands within a step or two of
/// one written in one. That is the whole of the difference measured here, and
/// the bound is what says so — a term dropped or double-counted moves a pixel
/// by far more than the target can round it by.
///
/// The reference is the same node one hair off the home sheet, which draws
/// WHOLE because only the home sheet is split. Under an orthographic camera
/// looking down the sevens axis that hair moves nothing on screen: the
/// billboard is built on `cam_right`/`cam_up`, both perpendicular to it, and
/// the depth test is `Always`. The pair with no clearing at all checks that,
/// and it is not decoration — under the fixture's own perspective camera the
/// hair is a change of SIZE, and the test measured 2256 px of it.
#[test]
fn splitting_a_clearing_off_its_node_paints_the_same_picture() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let build = |z: f32, gutter: f32| -> Scene {
        let mut scene = clearing_node(0, 1.0, true, gutter);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.background = glam::Vec4::new(0.05, 0.05, 0.07, 1.0);
        scene.glow_reach = 3.0;
        scene.glow_strength = 2.0;
        scene.glow_feather = 1.0;
        // A WIDE fade on the hole, which is where the split has anything to get
        // wrong: at full coverage a clearing repaints the ground it already
        // stands on, so only the band where it is partial can tell one
        // application from two. The fixture's own default is a hairline.
        scene.glow_shadow_soft = 0.3;
        // Nothing between the halves, so the only thing under test is whether
        // the two of them add up.
        scene.pluses.clear();
        // Something BEHIND for the hole to hide, without which the clearing is
        // invisible and this measures nothing: it repaints the ground it is
        // already standing on, so painting it twice is a no-op and a term
        // counted twice would pass. The sheet behind is what the hole is for.
        // Bigger, so its rings land out in the fade the hole ends with. A
        // behind node the same size as this one hides entirely under it, and
        // then the clearing has nothing to cover in the band where its coverage
        // is partial — which is the only band where painting it twice differs
        // from painting it once.
        // It clears at the same Shadow this one does, that being the view's — its
        // own hole lands on the ground behind it and moves nothing either shot
        // reads, both shots carrying it identically.
        let mut behind = scene.nodes[0];
        behind.world_pos.z = -0.6;
        behind.scale = 2.5;
        behind.on_home = false;
        scene.nodes.push(behind);
        let node = &mut scene.nodes[0];
        node.world_pos.z = z;
        // The size a home node draws at, kept while the depth moves: a sheet
        // off home is drawn smaller, and this is measuring the split rather
        // than the sevens layer.
        node.scale = 1.0;
        scene
    };
    let worst = |a: &[u8], b: &[u8]| -> i32 {
        (0..a.len()).map(|i| (a[i] as i32 - b[i] as i32).abs()).max().unwrap_or(0)
    };
    let split = gpu.shot(&build(0.0, CLEAR_REACH));
    let whole = gpu.shot(&build(0.001, CLEAR_REACH));

    // The depth hair on its own, with no clearing for either to draw.
    let flat_home = gpu.shot(&build(0.0, 0.0));
    let flat_off = gpu.shot(&build(0.001, 0.0));
    assert_eq!(
        worst(&flat_home, &flat_off),
        0,
        "the reference is not the same node: the depth hair moved the picture on its own",
    );
    // Non-vacuous: the node has to be painting a clearing for the split to have
    // anything to be exact about.
    let cleared =
        (0..split.len()).step_by(4).filter(|&i| split[i..i + 4] != flat_home[i..i + 4]).count();
    assert!(cleared > 100, "the fixture clears only {cleared} pixels; nothing is under test");

    assert!(
        worst(&split, &whole) <= 2,
        "the split moved a channel by {} over the {cleared} pixels it clears, which is more \
         than laying one fragment down in two steps can round it by",
        worst(&split, &whole),
    );
}

/// How far apart the two nodes of the probe below stand, in world units, and
/// the two reaches their hole is read at.
///
/// A shade over one node radius (`clearing_node` draws at 1.4), which is the
/// spacing that puts the near node's own ink clear of the far node's while its
/// CLEARING still lands well inside it. Closer and the two inks overlap, so
/// what a reading counts is occlusion rather than the hole; further and the
/// smaller reach never arrives.
const BESIDE_APART: f32 = 1.2;

const BESIDE_REACHES: [f32; 2] = [0.15, 0.35];

/// A node's clearing hides the rings of the node BESIDE IT on its own sheet,
/// and follows its bar while doing it.
///
/// Two nodes on one sheet overlap wherever the lattice is orbited — the sheet
/// foreshortens under billboards that do not — and the hole the nearer one cuts
/// is the whole of what tells the two apart there. Batching every home
/// knockout ahead of every home ink leaves each hole under all of that ink,
/// which is the bar going quiet on the picture it is most wanted in, and it is
/// the shape of the order this walks (`Draw`).
///
/// Read as the FAR node's own ink, taken where the near node paints nothing:
/// the ground is the shot's own clear colour, so a clearing standing over empty
/// lattice paints what is already there and the only pixels a reading can count
/// are ink the hole took. Two reaches rather than one, because a knockout that
/// lands but does not follow its bar is a hole that cannot be dialled — which
/// is a picture indistinguishable from no hole at the one setting anybody
/// checks.
///
/// With the GLOW OFF, and that is load-bearing rather than tidiness: the Shadow is
/// the light's row of uniforms (`Uniforms::misc11` in harmonigraph-render), and
/// every other row under the glow is zeroed whole where the Reach is 0. Zeroed
/// with them this row would take every node's hole off with the light — sheets
/// interpenetrating, the marker field uncut — and nothing in a picture with no
/// glow in it would say why. Both reaches here are measured at a Reach and a
/// Strength of 0, so that is the frame this is read in.
#[test]
fn a_node_clears_the_rings_of_the_node_beside_it_on_its_own_sheet() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Both nodes clear at the same reach, the Shadow being the view's, and only
    // the NEAR one's hole can move the reading: the far node's own lands on the
    // shot's black ground, which is what a clearing paints there anyway.
    let build = |gap: f32, beside: bool| -> Scene {
        let mut scene = clearing_node(0, 1.0, true, gap);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // BLACK, which is the colour `Shooter::shot` clears to: a clearing
        // repaints the ground, so over empty lattice it lands on the ground it
        // is already standing on and changes nothing. The light is off for the
        // same reason — a clearing paints the light back (`node_paint`), and a
        // pixel it puts back is a pixel this cannot tell from one it took.
        scene.background = glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        scene.glow_reach = 0.0;
        scene.glow_strength = 0.0;
        scene.pluses.clear();
        if beside {
            let mut near = scene.nodes[0];
            near.world_pos.x += BESIDE_APART;
            scene.nodes.push(near);
        }
        scene
    };
    let alone = gpu.shot(&build(0.0, false));
    let flat = gpu.shot(&build(0.0, true));
    // The far node's ink and nothing else: lit where it stands alone, and
    // untouched by the near node standing beside it with its hole switched
    // off. Anything the near node already covers is occlusion, which is not
    // what the clearing is for and would read the same with none.
    let own: Vec<usize> = (0..alone.len())
        .step_by(4)
        .filter(|&i| brightness(&alone[i..i + 4]) > 24 && flat[i..i + 4] == alone[i..i + 4])
        .collect();
    assert!(
        own.len() > 500,
        "the fixture leaves only {} px of the far node for a hole to take",
        own.len(),
    );
    let mut taken = |gutter: f32| -> usize {
        let holed = gpu.shot(&build(gutter, true));
        own.iter().filter(|&&i| holed[i..i + 4] != flat[i..i + 4]).count()
    };
    let [near, far] = BESIDE_REACHES;
    let (short, long) = (taken(near), taken(far));
    assert!(
        short > 100,
        "a Shadow of {near} took {short} px of the node beside it, out of the {} px \
         of it standing in the open",
        own.len(),
    );
    assert!(
        long > short,
        "the hole does not follow its bar: {short} px taken at a reach of {near} and \
         {long} at {far}",
    );
}

/// How wide an Octave gap the sweep below opens the hole at, and the Shadow it
/// cuts that hole with.
///
/// The pair is what makes the reading possible at all: a slice's own gap has to
/// outreach the hole around the ink beside it, or the two lobes of the clearing
/// either side of a boundary meet across it and the hole is a closed annulus
/// however empty the sector is. Half the Octave gap against one Shadow is that
/// condition (`slice_gap_half` and `standoff_coverage` in lattice.wgsl), so the
/// gap is dialled to a good deal more than twice the Shadow rather than to the two
/// being close.
///
/// The Shadow is narrower than [`CLEAR_REACH`] for the same reason, and it is the
/// only fixture here that does not take the file's own reach: at that reach the
/// Octave gap this asks for is past what the bar can be dragged to.
const SLICE_OCTAVE_GAP: f32 = 0.40;
const SLICE_CLEAR_GAP: f32 = 0.08;

/// At a wide Octave gap a node's hole OPENS in the sectors it is empty in: what
/// it hides is the ink it draws, and not the annulus that ink is drawn in.
///
/// A ring is slices with gaps between them. Measured off the closed annulus the
/// hole is one unbroken band whatever the padding, so a node dialled to a wide
/// Octave gap lays a solid ring of ground across everything behind it in the
/// very sectors it is painting nothing — a shadow cast by ink that is not there,
/// and worst exactly where the picture behind matters most, since a wide gap is
/// dialled to let that picture through. The walk that cuts it is
/// `slice_gap_distance`, shared with the standoff so the hole and the shadow
/// over it open together.
///
/// Two radii, and they are different questions. The INK is read across the
/// band's own middle, where a slice is opaque and a gap is bare — that is which
/// sectors the node is drawing. The HOLE is read just outside the band's outer
/// edge, where the node paints nothing at any angle so the clearing is the only
/// thing that can be there; inside the band the ink is composited over its own
/// hole (`node_paint`) and covers it.
///
/// The claim is the implication and not a correlation: every angle the hole is
/// open at is an angle the node inks nothing at. The converse is deliberately
/// NOT claimed — the hole is the ink DILATED by the Shadow, so it closes over the
/// narrow end of each gap and is open across fewer angles than the ink is
/// absent from. A test asserting both would be asserting the reach is zero.
///
/// The closed-gap shot is what keeps it from passing on a hole that is simply
/// broken: with the padding at 0 the ring is one annulus, and the hole has to be
/// unbroken all the way round.
#[test]
fn a_wide_octave_gap_opens_the_hole_in_the_sectors_the_node_leaves_empty() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();
    let build = |octave_gap: f32, gap: f32| -> Scene {
        let mut scene = clearing_node(0, 0.0, true, gap);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.octave_gap = octave_gap;
        scene.pluses.clear();
        scene
    };

    for (name, octave_gap, want_open) in
        [("a closed gap", 0.0, false), ("a wide gap", SLICE_OCTAVE_GAP, true)]
    {
        let bare = gpu.shot(&build(octave_gap, 0.0));
        let holed = gpu.shot(&build(octave_gap, SLICE_CLEAR_GAP));
        let added = light_over(&holed, &bare);
        let hole = light_about_center(&added, SIZE);
        assert!(hole.far > 8.0, "{name}: no clearing to read, {:.1} px", hole.far);
        // Pixels per uv off the hole's own outer edge, as every reading in this
        // file takes it: the furthest the clearing reaches is the band's outer
        // edge one Shadow out.
        let scale = hole.far / (rings.band.1 + SLICE_CLEAR_GAP) as f64;
        let ink_r = 0.5 * (rings.band.0 + rings.band.1) as f64 * scale;
        // Two fifths of a Shadow past the band, which is inside the solid part of
        // the hole and outside every layer the node draws.
        let hole_r = (rings.band.1 + 0.4 * SLICE_CLEAR_GAP) as f64 * scale;

        let (cx, cy) = ((SIZE[0] - 1) as f64 / 2.0, (SIZE[1] - 1) as f64 / 2.0);
        let at = |px: &[f64], r: f64, a: f64| -> f64 {
            let x = (cx + r * a.cos()).round() as usize;
            let y = (cy + r * a.sin()).round() as usize;
            px[y * SIZE[0] as usize + x]
        };
        let ink: Vec<f64> = bare.chunks(4).map(|p| brightness(p) as f64).collect();

        let mut open = 0;
        let mut over_ink = Vec::new();
        for turn in 0..360 {
            let a = (turn as f64).to_radians();
            // A hole clears to WHITE over a black shot, so a cleared angle is
            // most of a channel and an open one is none of it; half is far from
            // either and reads the same on any rounding.
            if at(&added, hole_r, a) < 0.5 * 3.0 * 255.0 {
                open += 1;
                if at(&ink, ink_r, a) > 24.0 {
                    over_ink.push(turn);
                }
            }
        }
        assert!(
            over_ink.is_empty(),
            "{name}: the hole is open at {} degrees the node inks a slice at ({over_ink:?}), \
             so what opened is a bite out of a slice rather than the sector beside it",
            over_ink.len(),
        );
        if want_open {
            assert!(
                (20..340).contains(&open),
                "{name}: the hole is open at {open} of 360 angles, which is a ring that is \
                 either whole or gone rather than one cut by its slices",
            );
        } else {
            assert_eq!(
                open, 0,
                "{name}: with no padding the ring is one annulus, and the hole broke anyway",
            );
        }
    }
}

/// How far off the home sheet the reading below puts its second node, as the
/// size factor `derive_scene` hands a node one step out
/// ([`ViewConfig::sevens_size`](harmonigraph_scene::ViewConfig)).
///
/// Well under 1, where the view ships AT 1: a factor near it makes the two
/// answers below agree to within the pixel grid's own wobble, and the fresh view
/// is the one setting at which this claim cannot be read at all.
const OFF_SHEET_SIZE: f32 = 0.5;

/// A node's hole SHRINKS with the node: it is a share of the node's radius, so
/// an off-sheet node clears in proportion to what it draws.
///
/// The Shadow is dialled as a fraction of a node's radius, which is what quad uv is
/// — the same unit the Ring gap and the Octave gap beside it read in — and the
/// hole is that reach around the node's own ink. So the whole picture a node
/// draws scales as one thing, and a sheet stepped back is the home sheet drawn
/// smaller rather than a smaller node wearing a full-size margin.
///
/// The alternative is a constant width ON SCREEN, which is what the retired
/// Clearance was: the shader divided the setting by the node's size factor, so a
/// half-size node cleared a full-size gap. Two arguments for the share, and the
/// second is the one that decides it. A gap that does not shrink reads as a
/// property of the sheet rather than of the node, and at a small `sevens_size`
/// it is most of what an off-sheet node covers — the ink is a quarter of the
/// area and the margin round it is the rest. And the Shadow is ONE length for the
/// hole and the shadow over it (`glow_shadow` in lattice.wgsl); the shadow was
/// always the node's own share, so a hole in screen units would be the two
/// disagreeing on every sheet but the home one.
///
/// Read as a RATIO of the two holes' outer radii rather than against a computed
/// radius, so nothing here has to know the camera: both shots are the same node
/// under the same projection, and the size factor is the only thing between
/// them.
#[test]
fn an_off_sheet_node_clears_in_proportion_to_its_own_size() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let build = |scale: f32, gap: f32| -> Scene {
        let mut scene = clearing_node(0, 0.0, true, gap);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.pluses.clear();
        scene.nodes[0].scale = scale;
        scene
    };
    let far_of = |gpu: &mut Shooter, scale: f32| -> f64 {
        let bare = gpu.shot(&build(scale, 0.0));
        let holed = gpu.shot(&build(scale, CLEAR_REACH));
        light_about_center(&light_over(&holed, &bare), SIZE).far
    };
    let home = far_of(&mut gpu, 1.0);
    let off = far_of(&mut gpu, OFF_SHEET_SIZE);
    assert!(home > 20.0, "the home node's hole is only {home:.1} px; there is nothing to scale");

    let want = f64::from(OFF_SHEET_SIZE);
    let got = off / home;
    // A pixel either side of each radius, which is what a hole's own edge lands
    // on: the ratio carries both, and at this size factor that is a couple of
    // percent against the fifty the constant-width answer would be out by.
    let slack = 2.0 / home + 2.0 * want / home;
    assert!(
        (got - want).abs() < slack,
        "the off-sheet hole reaches {off:.1} px against the home sheet's {home:.1}, a ratio \
         of {got:.3} where the node is drawn at {want} — a hole held to a constant width on \
         screen comes out ABOVE the size factor, the margin being the one part of the \
         picture that did not shrink",
    );
}
