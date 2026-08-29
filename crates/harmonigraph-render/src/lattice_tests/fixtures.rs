//! Scenes, shooters and readings shared by the modules beside this one.

use crate::gpu_harness::{headless_device, readback, render_to_texture};
use crate::*;

/// One marker for the fixtures below.
///
/// `strength` is the whole marker — its ink, its pool and the shadow its cross
/// writes into the light are one number — so a fixture that names it has named
/// all three. `lattice_pos` is not read anywhere in the render path, so the
/// position these markers say is `pos`.
pub(super) fn one_marker(
    pos: glam::Vec3,
    radius: f32,
    color: glam::Vec4,
    strength: f32,
) -> harmonigraph_scene::PlusInstance {
    harmonigraph_scene::PlusInstance {
        lattice_pos: harmonigraph_core::LatticePos::ORIGIN,
        pos,
        radius,
        color,
        strength,
    }
}

/// The band width every fixture in this file sweeps at: a WIDE one, chosen
/// so a band is many pixels across at these render sizes and a probe can
/// walk it. It is deliberately not the fresh view's width, which is 0.64 —
/// fine enough that several bands cross a single node, which is the look the
/// bar's tight end exists for and the wrong regime to measure a sheet's
/// geometry in.
///
/// The gap is a known hole in this suite rather than a property of it: the
/// shipped picture is only ever rendered here at the tight end's control
/// case. Anything that goes wrong at 0.64 alone — the resolve fade closing
/// on the pixel footprint, the crest/trough balance at softness 1 — ships
/// green.
///
/// Named because [`SHIMMER_PROBE_STEP`] is sized off it: the width is a view
/// setting rather than the shader constant it was, so a fixture retuned in
/// one place and not the other is how the probe would come to measure
/// nothing.
pub(super) const PARITY_SHIMMER_WIDTH: f32 = 5.0;

/// A scene exercising every draw path: lit + idle + hovered nodes with
/// octave indicators, and resting markers under them, all overlapping so blend
/// order matters.
pub(super) fn parity_scene() -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut nodes = Vec::new();
    for i in 0..6u32 {
        let f = i as f32;
        // Slots walked inside the DEFAULT Range: a level on a slot the view
        // doesn't show is drawn by no sector, so a scene claiming to exercise
        // every draw path has to sound in the range it renders. Asked of the
        // same layout the scene carries below rather than re-derived from the
        // slot count, which would part company with it the first time the
        // default window moved. The slots a node draws depend on its pitch
        // class, so this is the set EVERY node here has: the highest node's
        // low end, the lowest node's high end.
        let layout = harmonigraph_scene::OctaveLayout::default();
        let (low, _) = layout.slots(0.0);
        let (_, high) = layout.slots(950.0);
        let slot = |k: usize| (low.max(0) as usize) + k % (high - low + 1) as usize;
        let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
        octaves[slot(i as usize)] = 1.0 - f * 0.1;
        octaves[slot(i as usize + 5)] = 0.4;
        nodes.push(harmonigraph_scene::NodeInstance {
            lattice_pos: LatticePos::new(i as i32 - 3, i as i32 % 2, 0),
            // Cluster tightly around the origin so discs overlap and
            // draw order shows in the output.
            world_pos: Vec3::new(f * 0.45 - 1.1, (f % 3.0) * 0.4 - 0.4, f * 0.3 - 0.75),
            color: Vec4::new(0.25 + f * 0.12, 0.55 - f * 0.05, 0.95 - f * 0.1, 1.0),
            activation: if i % 3 == 0 { 1.0 } else { 0.3 + f * 0.1 },
            // Nothing the shader draws reads this — it is the label layer's,
            // and labels are the UI crate's text pass, not the lattice pass.
            departing: false,
            octaves,
            hovered: i == 1,
            on_home: i % 2 == 0,
            // The off-sheet half draws small, so the every-draw-path scene
            // exercises the scaled billboard as well. What every node here
            // knocks out is `parity_scene`'s own Shadow.
            scale: if i % 2 == 0 { 1.0 } else { 0.55 },
            comma: if i % 2 == 0 { 0.0 } else { -27.26 },
            cents: f * 190.0,
            // Exercise the mark paths: one node marked melody, one bass, and
            // one wearing both — on its two lit slots, so the pair is drawn as
            // two extensions rather than as one slice claimed twice.
            melody_slots: if i == 0 || i == 4 { 1 << slot(i as usize) } else { 0 },
            bass_slots: match i {
                2 => 1 << slot(i as usize),
                4 => 1 << slot(i as usize + 5),
                _ => 0,
            },
            melody_level: if i == 0 || i == 4 { 1.0 } else { 0.0 },
            bass_level: if i == 2 || i == 4 { 1.0 } else { 0.0 },
            melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
            bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
            // The lattice pass draws the ring on every node it ships; the
            // gate is the fold's answer and there is no fold here.
            audio_ring: 1.0,
            // A row per node in the order they are built, settled: the light's
            // own clock is the shell's pass and no shell has run here, so this
            // fixture is the picture with nothing carried — which is exactly
            // what a still image of the draw paths wants. The mark the light
            // is sized against is settled on this node's own, for the same
            // reason.
            glow: harmonigraph_scene::GlowStep {
                level: 1.0,
                row: i,
                mix: 1.0,
                marked: f32::from(i == 0 || i == 2 || i == 4),
            },
            trail: 0.0,
        });
    }
    // Two markers, one under a node and one clear of every node, so the pass is
    // exercised both where the nodes composite over it and where it stands
    // alone. Different radii, because the size is per instance.
    let pluses = vec![
        one_marker(Vec3::new(-1.8, -0.6, -0.3), 0.22, Vec4::new(0.16, 0.17, 0.20, 1.0), 0.55),
        one_marker(Vec3::new(0.0, 0.0, 0.0), 0.13, Vec4::new(0.16, 0.17, 0.20, 1.0), 0.4),
    ];
    let glow_rows = nodes.len() as u32;
    Scene {
        nodes,
        camera: harmonigraph_scene::Camera::default(),
        now: 1.25,
        // The ground the lattice stands on, as the app hands it in.
        background: harmonigraph_scene::skin::well_color(),
        // The grey the octave band's unsounding slices draw, at the fresh
        // view's own Ground — most of every node's band in this fixture.
        lattice_ground: harmonigraph_scene::grey_of_lightness(
            harmonigraph_scene::ViewConfig::default().lattice_ground,
        ),
        node_radius: 0.34,
        mark_thickness: 0.09,
        // Off: a single-instant parity image can't depend on which moment
        // of a cycle it lands on.
        pulse_marks: Default::default(),
        // The sweep's own settings, at this suite's measurable values rather
        // than the fresh view's (see `PARITY_SHIMMER_WIDTH`). Inert while the
        // mode above is Off,
        // and stated rather than defaulted because a test that turns a mode
        // ON — every shimmer test builds on this fixture — has to be sweeping
        // something a reader can size against SHIMMER_PROBE_STEP.
        shimmer_speed: 1.6,
        shimmer_width: PARITY_SHIMMER_WIDTH,
        shimmer_intensity: 1.0,
        shimmer_softness: 0.8,
        outer_inner: 0.545,
        outer_outer: 0.795,
        // The band is the outermost ring here, as it is on a fresh node, so it
        // is what the marks stand off — a gap out from its edge.
        rings_outer: 0.795,
        mark_inner: 0.795 + 0.12,
        // The same 0.12 the radii above are spaced by, the fixture standing its
        // layers off each other and cutting its sectors by one padding.
        octave_gap: 0.12,
        // No analyzer: the ring off, under either reading. It is a whole layer
        // more light in the middle of every node, and the sweep and mark
        // measurements here are sized against the picture without it —
        // `the_audio_ring_reads_the_spectrum_around_each_octave`,
        // `the_folded_ring_reads_each_wedge_at_its_own_octave` and the
        // early-out parity's own `ringing` fixture are where it is turned on.
        spectral: harmonigraph_scene::SpectralPaint::silent(),
        // The plain circular division: this scene is about how the draw
        // paths composite, so the indicators are the ones every other
        // setting is a departure from.
        octave_layout: harmonigraph_scene::OctaveLayout::default(),
        pluses,
        // Arms a little over half their length across, the proportion a fresh
        // view draws, so a marker here is the shape every width test is a
        // departure from.
        plus_half_width: 0.275,
        // Square-ended arms, so the fixture's markers are the shape the taper
        // is a departure from.
        plus_taper_start: 1.0,
        // A blue->red sweep across the whole table, so a glyph's color is a
        // reading of which entry it landed on. Spanned off PITCH_LUT_N rather
        // than a literal: `from_fn` sizes itself from the field, so a literal
        // divisor would silently stop covering the table when the constant
        // grows, leaving every entry past it out of gamut and clamping to one
        // color — the fixture would still render, and would still be green.
        pitch_lut: std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            Vec4::new(t, 0.4, 1.0 - t, 1.0)
        }),
        darkest_pitch: 24.0,
        brightest_pitch: 108.0,
        render_scale: 1.0,
        // Parity with the direct-to-egui-pass reference requires bloom off.
        bloom_strength: 0.0,
        // And the node glow with it: it is a pass of its own into a target of
        // its own, composited into the scene pass, which the
        // single-attachment reference path has no pass to composite into.
        glow_reach: 0.0,
        glow_strength: 1.0,
        // The accent's own falloff, which is what every glow test here that
        // does not say otherwise is measuring.
        glow_feather: 0.0,
        // Melded whole, the light two overlapping halos have always made, so a
        // test that says nothing about the Meld is measuring the screen.
        glow_meld: 1.0,
        // The fresh Shadow and the share a lit slice's own ink takes of the
        // light, so a test that says nothing about either measures the fresh
        // picture.
        glow_shadow: 0.16,
        glow_shadow_depth: 0.85,
        glow_wash: 1.0,
        // `node_radius` above through the uv rule both fields are in
        // (`marker_world`), so the span and the arms below read as the quad uv
        // every glow bar is dialled in.
        marker_unit: 0.34 * 1.8,
        glow_blend: 0.5,
        // A row per node, which is what the nodes above are built with.
        glow_rows,
    }
}

/// What the lattice pixel tests share on top of [`crate::gpu_harness`]: the
/// callback resources one lattice pass keeps between frames, and the
/// offscreen target a scene is drawn into.
///
/// It exists because the alternative had spread. Turning a `Scene` into a
/// buffer of pixels is twenty-seven lines that never vary — build the
/// callback, encode, submit, render to a texture, read it back — and every
/// test that wanted pixels carried its own copy of them, fourteen in this
/// file by the time they were counted, byte-identical but for where the pane
/// id came from. Nothing was wrong with any copy; what is wrong is the
/// arithmetic on the next change to that sequence, which then has to land in
/// fourteen places and silently keeps testing something else in the one it
/// misses. #112's mark-cache fix — a resource-lifetime correction between
/// `prepare` and `paint` — is exactly that shape of edit.
///
/// Each test owns its own `Shooter`, and so its own `CallbackResources`:
/// nothing here is shared between tests, which is what lets the pane counter
/// below start from the same place in all of them.
pub(super) struct Shooter {
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) resources: CallbackResources,
    pub(super) format: wgpu::TextureFormat,
    pub(super) size: [u32; 2],
    /// Bumped per shot, because a pane id keys the per-pane buffers inside
    /// `resources`, and a test comparing two pictures wants two of them
    /// rather than one reused. The tests that used to name these by hand all
    /// counted up the same way, and none ever asked for a repeat.
    pub(super) pane: u64,
    /// What the pane is filled with before the callback draws. BLACK, which is
    /// what nearly every fixture here is written against: they read a
    /// DIFFERENCE between two shots, and a ground of zero keeps that difference
    /// the pixels the shot is about.
    ///
    /// A fixture reading an ABSOLUTE frame has to keep this and
    /// [`Scene::background`] agreeing instead: `background` is the ground the
    /// app's own pane paints, so a lattice over a pane cleared to something
    /// else is a frame standing on a colour the app never shows.
    pub(super) clear: wgpu::Color,
}

impl Shooter {
    /// `None` where the machine has no usable GPU — CI containers, mostly.
    /// Every caller returns on it.
    pub(super) fn new(size: [u32; 2]) -> Option<Shooter> {
        let (device, queue) = headless_device()?;
        Some(Shooter {
            device,
            queue,
            resources: CallbackResources::default(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            size,
            pane: 1,
            clear: wgpu::Color::BLACK,
        })
    }

    /// `scene` drawn to a fresh texture over black, read back as RGBA8.
    pub(super) fn shot(&mut self, scene: &Scene) -> Vec<u8> {
        self.shot_with(scene, LatticeLabels::default())
    }

    /// [`shot`](Self::shot), with labels — the layer that carries its own
    /// atlas, and its own reasons to be tested.
    pub(super) fn shot_with(&mut self, scene: &Scene, labels: LatticeLabels) -> Vec<u8> {
        self.pane += 1;
        self.draw(scene, labels)
    }

    /// The frame AFTER the last shot, on the same pane rather than a picture
    /// of its own.
    ///
    /// One thing survives a frame here and it is a node's light: the ink strip
    /// keeps each row's colour, and a row's next reading is mixed into what it
    /// already held (`harmonigraph_scene::GlowStep::mix`). That is exactly what
    /// every other shot's fresh pane exists to keep out — a fixture is settled
    /// unless it says otherwise — so a test that wants the carrying has to ask
    /// for it.
    pub(super) fn shot_again(&mut self, scene: &Scene) -> Vec<u8> {
        self.draw(scene, LatticeLabels::default())
    }

    pub(super) fn draw(&mut self, scene: &Scene, labels: LatticeLabels) -> Vec<u8> {
        // A lit node's ink-strip row is read back by IDENTITY, out of a strip
        // `glow_rows` tall, so a row past the end is not a small error in a
        // fixture: what an out-of-range `textureLoad` returns is the BACKEND's
        // choice. Metal clamps, so the node silently reads row 0 and wears
        // another node's colour while the test goes on passing; a backend that
        // returns zero instead makes the same test vacuous, passing with the
        // draw it is about deleted. Either way the fixture measures something
        // other than what it says, and says nothing when it stops measuring at
        // all — so the bookkeeping is asserted here, once, for every shot.
        //
        // `rows_per_node` is the helper that gets this right; a fixture that
        // pushes a node by hand has to call it.
        for (i, node) in scene.nodes.iter().enumerate() {
            assert!(
                node.glow.level <= 0.0 || node.glow.row < scene.glow_rows,
                "node {i} claims ink-strip row {} of a strip {} tall — see `rows_per_node`",
                node.glow.row,
                scene.glow_rows,
            );
        }
        let size = self.size;
        let vec_size = egui::vec2(size[0] as f32, size[1] as f32);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
        let cb = LatticeCallback::from_scene(scene, labels, vec_size, self.format, self.pane, None);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let bufs =
            cb.prepare(&self.device, &self.queue, &screen, &mut encoder, &mut self.resources);
        self.queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let resources = &self.resources;
        let tex =
            render_to_texture(&self.device, &self.queue, size, self.format, self.clear, |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: size,
                    },
                    pass,
                    resources,
                );
            });
        readback(&self.device, &self.queue, &tex, size)
    }
}

/// How many pixels of two shots of one size differ at all.
pub(super) fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
}

/// One pixel's brightness, as the plain sum of its channels — a reading
/// rather than a colorimetric luminance, and every caller compares two of
/// them rather than asking what the number means on its own.
pub(super) fn brightness(px: &[u8]) -> i64 {
    px[0] as i64 + px[1] as i64 + px[2] as i64
}

/// All the light in one shot (see [`brightness`]).
pub(super) fn total_light(px: &[u8]) -> i64 {
    px.chunks(4).map(brightness).sum()
}

/// The slot mask naming middle C's octave — the one the node below sounds
/// in, and so the one a mark can link back to.
pub(super) const MIDDLE_C: u32 = 1 << harmonigraph_scene::MIDDLE_C_SLOT;

/// `L*` of one pixel, off the curve `harmonigraph_scene::color` authors the
/// ramp on rather than a copy of it.
///
/// Real colorimetry where the rest of this file reads a channel sum, because
/// the tests below are claims about how bright a thing LOOKS, compared across
/// colors that differ in hue as well: a sum weights a blue channel like a green
/// one and would call a violet ring and a yellow one the same brightness.
/// Shared with the authoring code and not restated here, so a reading is in the
/// units the ramp is dialled in — a second copy of the constants could drift
/// and would then agree with itself and disagree with the picture.
pub(super) fn lightness(px: &[u8]) -> f64 {
    let v = |b: u8| f64::from(b) / 255.0;
    harmonigraph_scene::color::lightness_of_encoded(v(px[0]), v(px[1]), v(px[2]))
}

/// A color's steady shot and the eight swept ones taken over it.
pub(super) type Shots = (Vec<u8>, Vec<Vec<u8>>);

/// Whether one shot's sweep moves this pixel: further from its steady self than
/// a byte's rounding at some moment of the period.
///
/// MOVED and not brightened, which is the whole of what an exposure changes
/// here. The sheet is a ratio fitted to the layer's own headroom
/// (`shimmer_light`), so where a color has room the sweep reads as light added
/// and where it has none it reads as shade between crests — and a color at the
/// top of a channel takes ALL of it as shade. Asking for a brighter pixel would
/// find the sheet at one end of the ramp and miss it at the other, which is the
/// difference these tests exist to measure rather than to filter out.
pub(super) fn swept(shot: &Shots, i: usize) -> bool {
    let base = lightness(&shot.0[i * 4..i * 4 + 4]);
    shot.1.iter().any(|f| (lightness(&f[i * 4..i * 4 + 4]) - base).abs() > 1.0)
}

/// Bloom must add light (halo energy over the bloom-off output) —
/// and only when asked: strength 0 keeps the parity test above valid.
/// One big centered node, sounding, with one octave slot lit: a clean
/// backdrop for measuring how much of the picture a mark actually
/// covers. parity_scene deliberately overlaps its nodes, which hides
/// most of a mark behind whatever draws in front of it.
pub(super) fn single_marked_node(melody_slots: u32, bass_slots: u32) -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
    // Middle C's slot: the marks link back to a sector, so the note has to
    // sound in one the view actually shows.
    octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
    let mut scene = parity_scene();
    scene.nodes = vec![harmonigraph_scene::NodeInstance {
        lattice_pos: LatticePos::ORIGIN,
        world_pos: Vec3::ZERO,
        color: Vec4::new(0.35, 0.55, 0.85, 1.0),
        activation: 1.0,
        // Held at full, so neither end of the envelope is running.
        departing: false,
        octaves,
        hovered: false,
        on_home: true,
        scale: 1.0,
        comma: 0.0,
        cents: 0.0,
        melody_slots,
        bass_slots,
        // A mark draws at the level its own note is at; these stand in for
        // a freshly-held note.
        melody_level: f32::from(melody_slots != 0),
        bass_level: f32::from(bass_slots != 0),
        // Distinct hues so the both-ends check below can tell the two
        // marks apart; in the app these are the marked SECTORS' colors,
        // which are the same color wherever the two name one sector.
        melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
        bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
        // The lattice pass draws the ring on every node it ships; the
        // gate is the fold's answer and there is no fold here.
        audio_ring: 1.0,
        // Lit and settled on the strip's first row: one node, nothing carried,
        // and the light sized against whichever ends this fixture is wearing.
        glow: harmonigraph_scene::GlowStep {
            level: 1.0,
            row: 0,
            mix: 1.0,
            marked: f32::from((melody_slots | bass_slots) != 0),
        },
        trail: 0.0,
    }];
    // BLACK, which is the colour `Shooter::shot` clears to, so the pane and the
    // ground the scene names are one value: every reading taken off this fixture
    // is of the node's own ink or of the light around it, and a ground lifted
    // off black would sit under both.
    scene.background = Vec4::new(0.0, 0.0, 0.0, 1.0);
    // One node, so one row — the strip follows the scene it is handed.
    scene.glow_rows = 1;
    scene.pluses.clear();
    // Fill a good share of the frame, so the measurements below are
    // about the mark's design rather than about pixel quantization.
    scene.node_radius = 1.1;
    // The one uv both fields are in (`marker_world`): a marker's unit IS the
    // node's radius through this factor, so a scene that sets one and not the
    // other is a scene `derive_scene` cannot build — and every Shadow in it is two
    // different world distances, one for a ring and one for a cross.
    scene.marker_unit = scene.node_radius * 1.8;
    scene
}

/// Hand every node in a scene its own row of the ink strip, and size the strip
/// to them.
///
/// What the shell's own pass does with a map from node to row
/// (`panes::glow_fade` in harmonigraph-ui), reduced to what a fixture needs: a
/// scene assembled by hand is one frame with nothing carried, so a row per node
/// in the list's own order is both unique and stable. It is [`derive_scene`]'s
/// own answer, and the reason a fixture has to restate it is that replacing
/// `scene.nodes` replaces the rows with copies of one.
///
/// A row is read back by identity, so two nodes sharing one is not a subtle
/// wrong: both write it and both read whichever won.
pub(super) fn rows_per_node(scene: &mut Scene) {
    for (row, node) in scene.nodes.iter_mut().enumerate() {
        node.glow.row = row as u32;
    }
    scene.glow_rows = scene.nodes.len() as u32;
}

/// The slot beside middle C's, as a mask — a second sector for the two ends to
/// land on separately.
///
/// Taken off the layout the fixture actually draws rather than named: a mark on
/// a slot outside the drawn ring has no sector to extend and no angle to be
/// drawn at, so it would read as the mark having vanished.
pub(super) fn slot_beside_middle_c() -> u32 {
    let (low, high) = harmonigraph_scene::OctaveLayout::default().slots(0.0);
    let c = harmonigraph_scene::MIDDLE_C_SLOT as i32;
    let beside = if c < high { c + 1 } else { c - 1 };
    assert!(
        (low..=high).contains(&beside) && beside != c,
        "the fresh wheel draws {low}..={high}, which has no second slot beside {c}",
    );
    1 << beside
}

/// How far either side of a pitch the fixture's synthetic partial reaches, in
/// cents.
///
/// A BAND rather than a single bucket, and 40¢ rather than the 3.125¢ one
/// bucket spans, because the measurement below is made in pixels: the ring is
/// an annulus about 20 px from the node's centre in a 256 px shot, so a wedge
/// of the fresh five-octave wheel is some 25 px of arc, and one bucket at the
/// probe's 200¢ Range would be an eighth of a pixel of it. Symmetric, so the
/// lit arc's centroid is the band's own centre pitch whatever the Range.
pub(super) const PARTIAL_HALF_CENTS: f32 = 40.0;

/// The padding `ringing_node` stands its layers off each other by — see there.
/// Spent on BOTH of the node's axes in these fixtures, radially between the
/// layers and angularly between the sectors, which is what the view's two gap
/// bars are free to dial apart: a probe reading a radius wants the layers
/// pixels apart, and one reading a sector wants the seams pixels wide.
pub(super) const PROBE_GAP: f32 = 0.12;

/// Where the probe stacks BEGIN, and it is the node's own centre: a radius read
/// off one of these pictures is then a width, or a sum of widths and gaps, with
/// no offset under it. Stated rather than inherited for the same reason every
/// other size here is — a fresh view stands its stack well out from the centre
/// (see [`ViewConfig::ring_inner`](harmonigraph_scene::ViewConfig)), and the
/// probe widths, deliberately wide so a pixel reading can tell one layer's edge
/// from the next, do not fit in what that leaves.
pub(super) const PROBE_INNER: f32 = 0.0;

/// The octave band's width for the probes below, standing in for the fresh
/// view's own (see [`ViewConfig::band_width`](harmonigraph_scene::ViewConfig))
/// the same way [`PROBE_GAP`] stands in for the gap: the band is the outermost
/// ring the stack has to fit, so it is the layer a retune of anything INSIDE
/// it pushes off the quad edge, and a band the stack has refused draws nothing
/// for a pixel reading to find.
pub(super) const PROBE_BAND_WIDTH: f32 = 0.163_084_63;

/// The angular padding the layered probes slice their wedges at, standing in
/// for the fresh view's own (see
/// [`ViewConfig::octave_gap`](harmonigraph_scene::ViewConfig)): a reading is
/// taken across a wedge's own arc, and a slicing dialled wide enough eats the
/// arc it is taken over.
pub(super) const PROBE_OCTAVE_GAP: f32 = 0.05;

/// The Range these fixtures read their partials against, standing in for the
/// fresh view's own (see
/// [`ViewConfig::spectral_ring_range`](harmonigraph_scene::ViewConfig)) the
/// same way [`PROBE_GAP`] stands in for the gap: a window dialled narrow
/// enough leaves a detune too small for a 256 px shot to resolve, which is a
/// property of the shot's resolution rather than of the Range being tested.
pub(super) const PROBE_RANGE: f32 = 200.0;

/// One node wearing both rings at the probe's own widths: `held` lighting an
/// octave of the band, and a synthetic partial at absolute MIDI `sounding` in
/// the analyzer's grid for the audio ring to find.
///
/// The probe's stack rather than the fresh view's, because what the claims
/// below need is two annuli far enough apart for a 256 px shot to tell one
/// from the other — a proportion, not the shipped one. Inheriting the fresh
/// widths would tie every reading here to an aesthetic that moves whenever a
/// dialled-in look is captured, and it moves the readings the wrong way:
/// [`PROBE_BAND_WIDTH`] is the outermost ring, so a retune of the audio ring
/// INSIDE it can push the band off the quad edge, and a refused
/// band leaves these measurements nothing to find. That the shipped stack
/// draws three visible layers inside the quad is held where it belongs, on the
/// fresh view itself, by `harmonigraph_scene`'s
/// `the_fresh_node_stacks_three_visible_layers_inside_the_quad`.
///
/// The PADDING is the probe's for a second reason: the Ring gap is what
/// separates every layer of a node, and a gap of the order the fresh view
/// carries is under three pixels on the 52-px node this renders, where the two
/// annuli's anti-aliased edges meet inside it. A wider gap measures the
/// geometry rather than the edge softness.
///
/// The ramp is a plain black-to-white one rather than a gradient, so a pixel's
/// brightness IS the level the shader read out of the grid and the differences
/// below measure the reading rather than a hue.
pub(super) fn ringing_node(held: Option<usize>, sounding: Option<f32>, range: f32) -> Scene {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = single_marked_node(0, 0);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    // Fully present whatever is lit, so the band's ghost ring is the same in
    // every shot and drops out of the differences below.
    node.activation = 1.0;
    if let Some(slot) = held {
        node.octaves[slot] = 1.0;
    }
    // The probe's stack: the two rings land far enough apart to be measured in
    // pixels, at radii no capture of a dialled-in look moves.
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        ..fresh.clone()
    }
    .rings();
    scene.outer_inner = rings.band.0;
    scene.outer_outer = rings.band.1;
    scene.rings_outer = rings.outer;
    scene.mark_inner = rings.mark_inner;
    scene.octave_gap = PROBE_GAP;

    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    paint.lut = std::array::from_fn(|k| {
        let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
        glam::Vec4::new(t, t, t, 1.0)
    });
    (paint.inner, paint.outer) = rings.audio;
    paint.range = range;
    if let Some(pitch) = sounding {
        for (bucket, level) in paint.levels.iter_mut().enumerate() {
            let cents = (harmonigraph_scene::bucket_pitch(bucket) - pitch) * 100.0;
            if cents.abs() <= PARTIAL_HALF_CENTS {
                *level = 255;
            }
        }
    }
    paint.color_levels.clone_from(&paint.levels);
    scene.spectral = paint;
    scene
}

/// Per-pixel brightness of one shot less another's — how a wedge is separated
/// from the ghost ring it is drawn over, both shots carrying the same ghosts.
pub(super) fn light_over(shot: &[u8], base: &[u8]) -> Vec<f64> {
    shot.chunks(4)
        .zip(base.chunks(4))
        .map(|(a, b)| (brightness(a) - brightness(b)).max(0) as f64)
        .collect()
}

/// Where a shot's light sits about the center of the FRAME, which is about
/// where a node at the world origin draws.
///
/// "About" is all it has to be. Every claim below compares two of these, and
/// both are read from the same place with the node in the same spot, so a
/// center a few pixels out moves them together and cancels — which is what
/// keeps this from needing the camera's projection written out a second time.
pub(super) struct Light {
    /// Total brightness; 0 when nothing drew.
    pub(super) weight: f64,
    /// The nearest and furthest a pixel worth seeing sits from that center, in
    /// pixels.
    pub(super) near: f64,
    pub(super) far: f64,
    /// The direction of the brightness-weighted centroid, in radians on the
    /// image's own axes — every claim compares two of these, so which way the
    /// screen's y runs never has to be settled.
    pub(super) angle: f64,
}

/// How bright a pixel must be to count toward [`Light::near`]/[`Light::far`]:
/// past a wedge's antialiased fringe, which trails off over a couple of levels
/// and would otherwise put the extent a pixel either way.
pub(super) const RING_LIT: f64 = 24.0;

pub(super) fn light_about_center(weights: &[f64], size: [u32; 2]) -> Light {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let (mut weight, mut near, mut far) = (0.0, f64::INFINITY, 0.0f64);
    let (mut sx, mut sy) = (0.0, 0.0);
    for (i, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        weight += w;
        sx += w * x;
        sy += w * y;
        if w >= RING_LIT {
            let r = x.hypot(y);
            near = near.min(r);
            far = far.max(r);
        }
    }
    Light { weight, near, far, angle: sy.atan2(sx) }
}

/// The short way round from `b` to `a`, in degrees, SIGNED on the image's own
/// axes.
///
/// Which sign means "clockwise on screen" is deliberately never settled here.
/// Every claim that needs a direction calibrates it from the picture itself —
/// the octave band's own wedges, an octave apart, are a known rise in pitch —
/// so nothing below depends on which way the shot's y runs.
pub(super) fn signed_apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI { d - std::f64::consts::TAU } else { d }.to_degrees()
}

/// The short way round between two angles, in degrees.
pub(super) fn angle_apart(a: f64, b: f64) -> f64 {
    signed_apart(a, b).abs()
}

/// The audio ring's width for the layered probe, standing in for the fresh
/// view's own (see [`ViewConfig::spectral_ring_width`](harmonigraph_scene::ViewConfig))
/// the same way [`PROBE_GAP`] stands in for its gap: a ring dialled thinner
/// leaves too few pixels either side of it for a per-layer reading to pick one
/// edge out of the next.
pub(super) const PROBE_RING_WIDTH: f32 = 0.3;

/// The stack the layered probes are measured against: three layers at the
/// probe's own widths and wider padding, so a pixel reading can tell one
/// layer's edge from the next.
///
/// Every width the stack is built from is stated rather than inherited. What
/// these probes need is a node wearing all three layers with room between
/// them; a capture of a dialled-in look is free to dial any of them to a
/// hairline, and each one left inheriting is a way for a reading to fail on a
/// change that has nothing to do with what it measures.
pub(super) fn layered_rings() -> harmonigraph_scene::RingStack {
    let fresh = harmonigraph_scene::ViewConfig::default();
    harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        octave_gap: PROBE_OCTAVE_GAP,
        ..fresh
    }
    .rings()
}

/// One node wearing all three layers at [`layered_rings`]'s radii: `melody`
/// names the slot its mark extends (0 for no mark), `ring` how much of its audio
/// ring the view's Gate leaves it, `band` whether the octave band is on, and
/// `shadow` the Shadow it casts at (0 for none).
///
/// The Shadow is the VIEW's, so it is set on the scene rather than on the node —
/// every node a fixture adds beside this one casts at the same width.
///
/// The ground is WHITE where the app stands on the pane's own panel: a reading
/// taken as "what changed when the shadow was turned on" then has the whole
/// range of a channel to move in instead of a few levels over black.
///
/// The node is drawn small enough for its shadow to fit in the frame — the blur
/// reaches a third of a node past a mark that already stands outside every
/// ring.
pub(super) fn layered_node(melody: u32, ring: f32, band: bool, shadow: f32) -> Scene {
    let rings = layered_rings();
    let mut scene = single_marked_node(melody, 0);
    scene.background = glam::Vec4::ONE;
    scene.node_radius = 1.4;
    scene.octave_gap = PROBE_GAP;
    scene.mark_thickness = rings.mark_thickness;
    // The audio ring drawn off an all-zero grid: the ramp's floor across the
    // whole annulus, which is ink at a known radius and all this asks of it.
    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    paint.lut = std::array::from_fn(|_| glam::Vec4::new(1.0, 1.0, 1.0, 1.0));
    (paint.inner, paint.outer) = rings.audio;
    scene.spectral = paint;
    (scene.outer_inner, scene.outer_outer) = if band { rings.band } else { (0.0, 0.0) };
    // The view's own cursor either way: which layer it landed on is the band
    // with one and the audio ring without, and `ring` is a per-NODE answer that
    // leaves it where it is.
    scene.rings_outer = if band { rings.band.1 } else { rings.audio.1 };
    scene.mark_inner = scene.rings_outer + rings.gap;
    scene.glow_shadow = shadow;
    scene.nodes[0].audio_ring = ring;
    scene
}

/// How far that probe moves the node, in world units: half the band width
/// the fixtures sweep at, so a move ACROSS the bands lands it on a very
/// different part of the sweep rather than back where it started.
///
/// Derived from [`PARITY_SHIMMER_WIDTH`] rather than written as 2.5, because
/// the width is a SETTING now and the fixture picks it. Retuned by hand the
/// two would drift apart silently, and a step that came out a whole number of
/// widths would move the node back onto the phase it started at — the probe
/// would then report a shimmer defect for a shimmer that is working, which is
/// the same trap `the_probe_moves_along_the_angle_the_shader_sweeps` keeps
/// the ANGLE out of.
pub(super) const SHIMMER_PROBE_STEP: f32 = PARITY_SHIMMER_WIDTH * 0.5;

/// Every node idle: no note, no marks, no octaves — the state most of a
/// lattice is in most of the time, and the state in which a NODE paints
/// nothing. So every test below expects these nodes to be culled, and the
/// fixture exists to make "nothing to draw" easy to ask for.
///
/// A culled node is not a blank frame, and the difference is the trap here:
/// what says a position is there is the MARKER standing at it, which this
/// fixture carries (`a_silent_lattice_ships_no_nodes_and_still_draws_its_markers`
/// pins that it must). Since a marker draws ink and casts a shadow of its own,
/// an idle shot is not an empty frame. A test wanting a genuinely bare one
/// clears `pluses` for itself.
///
/// The AUDIO RING has to be off for that to hold, and it is:
/// [`parity_scene`] carries a silent [`harmonigraph_scene::SpectralPaint`].
/// The ring is a window onto one shared spectrum rather than a level a node
/// carries, so with it on every node in the window paints one and there is no
/// such thing as an idle node to cull.
///
/// `on_home` and `trail` are still set, on different cycles, and neither is
/// read by anything in THIS crate: no `GpuInstance` carries them, and the
/// markers and the labels both arrive already built. They stay because a
/// fixture whose two sets coincide cannot tell a predicate that reads one
/// from a predicate that reads the other, and the cull is where such a
/// predicate would go — but nothing here separates them today, so treat the
/// cycles as room left rather than as coverage.
pub(super) fn idle_scene() -> Scene {
    let mut scene = parity_scene();
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.hovered = false;
        node.on_home = i % 2 == 0;
        node.trail = if i % 3 == 0 { 0.8 } else { 0.0 };
    }
    scene
}

/// The lattice a marker's shadow is measured on: one node lighting the whole
/// pane, and four markers standing in that light.
///
/// The only light in the frame is the node's, so a pixel the markers darken is
/// a pixel where a marker held a node's halo off — the melded field.
pub(super) fn shadowed_markers(depth: f32, shadow: f32, taper_start: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // A halo across the whole pane, so every marker has light to hold off
    // wherever it stands.
    scene.glow_reach = 4.0;
    scene.glow_strength = 2.0;
    scene.glow_feather = 1.0;
    scene.glow_shadow = shadow;
    scene.glow_shadow_depth = depth;
    scene.plus_taper_start = taper_start;
    // Four markers out where the node's halo still reaches and the node's own
    // shadow does not. The distance is the fixture's one delicate number: a
    // marker standing inside the node's own shadow would have that shadow
    // multiplied in beside its own, and the darkness measured would be both.
    scene.pluses = [(2.6f32, 0.0f32), (-2.6, 0.0), (0.0, 2.6), (0.0, -2.6)]
        .into_iter()
        .map(|(x, y)| one_marker(glam::Vec3::new(x, y, 0.0), 0.4, scene.lattice_ground, 1.0))
        .collect();
    scene
}

/// ONE square-ended marker standing out in the node's halo, with the Shadow
/// dialled to `shadow` and its depth switched by `depth`.
///
/// [`shadowed_markers`] with its four crosses replaced by one, so a reading can
/// walk out from a single cross along the centre row and meet nothing else. The
/// marker keeps that fixture's distance from the node ([`LONE_OFFSET`]): a
/// cross standing inside the node's own shadow would have both multiplied into
/// the same pixels, and what was measured would be the pair.
///
/// It stands to the node's RIGHT, which is where a reading walks: away from the
/// node, so the half of the frame being measured holds no other ink and no
/// other shadow.
pub(super) fn lone_shadowed_marker(arm: f32, shadow: f32, depth: f32) -> Scene {
    lone_tapered_marker(arm, shadow, depth, 1.0)
}

/// [`lone_shadowed_marker`] with the arms' taper dialled too: `taper_start` is
/// where an arm stops being solid, as a share of its length, 1 a square end.
pub(super) fn lone_tapered_marker(arm: f32, shadow: f32, depth: f32, taper_start: f32) -> Scene {
    let mut scene = shadowed_markers(depth, shadow, taper_start);
    scene.pluses =
        vec![one_marker(glam::Vec3::new(LONE_OFFSET, 0.0, 0.0), arm, scene.lattice_ground, 1.0)];
    scene
}

/// How far along the centre row [`lone_shadowed_marker`]'s cross stands from the
/// node lighting it, in world units — [`shadowed_markers`]' own distance, and
/// delicate for that fixture's reason.
pub(super) const LONE_OFFSET: f32 = 2.6;

/// The amplitude of the `k`th angular harmonic of a profile — how much of it is
/// a ripple that goes round the turn exactly `k` times.
pub(super) fn harmonic(profile: &[f64], k: usize) -> f64 {
    let n = profile.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in profile.iter().enumerate() {
        let a = std::f64::consts::TAU * k as f64 * i as f64 / n;
        re += v * a.cos();
        im += v * a.sin();
    }
    2.0 * (re * re + im * im).sqrt() / n
}

/// The name's own fixture: a lit node at the origin and a light wide enough to
/// reach past every ring it paints.
///
/// The arrangement is `a_resting_marker_wears_the_wash_it_stands_in`'s, and
/// deliberately: the name tests are that cross's claims asked of a name, so
/// what they are read against has to be the same picture with the cross swapped
/// for a glyph.
pub(super) fn lit_node_and_a_name(reach: f32, shadow: f32, depth: f32) -> Scene {
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
    scene.glow_shadow_depth = depth;
    // The markers away: a cross casts a shadow of its own into the frame the
    // name tests read.
    scene.pluses.clear();
    scene
}

/// How wide a fixture's glyph is drawn, in points. Its atlas patch is 8 texels
/// square and opaque (`text::tests::atlas`), so at this size it is a solid
/// block of ink several pixels across — and a pixel inside it carries the
/// label's colour exactly, which is what the wash is read on.
pub(super) const NAME_SIZE: f32 = 12.0;

/// Where a name stands to be read against a node's INK rather than against the
/// ground: the middle of the octave band, on the +x axis.
///
/// The empty middle is the one place a name over a node says nothing about ink
/// — a node paints none there, so a name standing in it is a name over the
/// ground the rings stand around. Derived off the
/// scene's own band radii rather than written as a number, because a fixture
/// that retunes the stack would otherwise leave the name standing where the
/// band no longer is: the ring radii are in quad uv, whose 1 is
/// [`Scene::marker_unit`] world units.
pub(super) fn name_on_the_band(scene: &Scene) -> glam::Vec3 {
    let uv = (scene.outer_inner + scene.outer_outer) * 0.5;
    assert!(uv > 0.0, "the fixture must wear an octave band for a name to stand on");
    glam::Vec3::new(uv * scene.marker_unit, 0.0, 0.0)
}

/// Where `world` lands on a `size` pane, in points.
pub(super) fn on_screen(scene: &Scene, size: [u32; 2], world: glam::Vec3) -> glam::Vec2 {
    scene
        .projector(glam::Vec2::new(size[0] as f32, size[1] as f32))
        .project(world)
        .expect("the fixture stands in front of the camera")
}

/// One stroke of a fixture's name: the atlas's opaque block filling `rect`, in
/// the resting field's own grey.
pub(super) fn name_glyph(scene: &Scene, rect: [f32; 4]) -> GlyphInstance {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let ink = scene.lattice_ground;
    GlyphInstance {
        rect,
        fill: [byte(ink.x), byte(ink.y), byte(ink.z), 255],
        // The shadow's strength, which is the whole of what the glow pass reads
        // off this colour (`fs_glyph_glow`).
        rim: [0, 0, 0, 255],
        ..crate::text::tests::glyph()
    }
}

/// One name per run handed over: the node it names, and its glyphs.
pub(super) fn names(runs: Vec<(u32, Vec<GlyphInstance>)>) -> LatticeLabels {
    LatticeLabels {
        labels: runs
            .iter()
            .map(|(node, glyphs)| Label { node: *node, glyphs: glyphs.len() as u32 })
            .collect(),
        glyphs: runs.into_iter().flat_map(|(_, glyphs)| glyphs).collect(),
        atlas: Some(crate::text::tests::atlas()),
        marks: Some(crate::text::tests::mark_sheet()),
        slide: SlideAxis::default(),
    }
}

/// [`names`] with every glyph on the scene's first node.
pub(super) fn a_name(glyphs: Vec<GlyphInstance>) -> LatticeLabels {
    names(vec![(0, glyphs)])
}

/// [`a_name`] as a single [`NAME_SIZE`] glyph standing at `world`.
pub(super) fn name_at(scene: &Scene, size: [u32; 2], world: glam::Vec3) -> LatticeLabels {
    let at = on_screen(scene, size, world);
    let rect = [at.x - NAME_SIZE / 2.0, at.y - NAME_SIZE / 2.0, NAME_SIZE, NAME_SIZE];
    a_name(vec![name_glyph(scene, rect)])
}
