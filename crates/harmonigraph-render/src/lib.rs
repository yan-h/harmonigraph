//! The wgpu lattice renderer, packaged as an egui paint callback.
//!
//! The same code path runs in both shells: the standalone harness (eframe
//! with the wgpu backend) and the plugin editor (egui-baseview with the wgpu
//! backend). A pane that wants to show the lattice allocates a rect and
//! adds [`lattice_paint_callback`] to the painter; pipelines and buffers are
//! created lazily on first paint and cached in egui-wgpu's
//! `CallbackResources`.
//!
//! Rendering model: one instanced draw of camera-facing quads (billboards),
//! sorted back-to-front on the CPU, rendered in `prepare()` into a per-pane
//! offscreen color + depth target and composited into the egui pass in
//! `paint()` as one textured quad (blit.wgsl). Owning the pass is what
//! makes the render-scale option (super/sub-sampling) possible, and gives
//! post-processing (bloom etc.) a texture to read; the depth buffer is
//! written (pass-through `Always` test, so draw order still composites
//! exactly as it would without the offscreen pass) but not yet read by
//! anything. `offscreen_composite_matches_direct_draw` in the tests pins
//! down that this path matches drawing straight into the egui pass.
//!
//! The node NAMES are drawn in that same pass, each at its own node's place
//! in the order (see [`LatticeLabels`]) — so a nearer node covers the name of
//! the node behind it by ordinary alpha blending, exactly as it covers the
//! sheet behind it. They arrive as glyphs, from the same collector the rest
//! of the UI's text goes through; what differs is which pass they land in,
//! and so that they inherit its render scale. They do NOT reach the bloom:
//! the pass carries a second colour attachment holding the picture without
//! them, and the bright pass reads that (see [`Offscreen::nodes_view`]).
//!
//! With the `hot-reload` feature (enabled by the standalone harness), the
//! .wgsl file is watched on disk and the pipeline rebuilds on save —
//! validated first, so a broken edit logs an error and keeps the old
//! pipeline instead of crashing. Release plugin builds keep `include_str!`
//! only.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use harmonigraph_scene::Scene;

/// The piano roll's own callback — a different picture with the same
/// problem, solved the same way. It shares this crate's wgpu version, buffer
/// helpers and [`BloomChain`]; the lattice's offscreen target and depth buffer
/// are beside the point for a flat ribbon.
mod roll;
pub use roll::{roll_paint_callback, RollAxes, RollInstance};

/// A halo alone, over marks a pane drew for itself — the third caller of
/// [`BloomChain`], and the one that draws no picture of its own.
mod glow;
pub use glow::{glow_paint_callback, GlowDot};

/// Label text, for the same reason the roll has its own callback: what a
/// label costs is the rim, and the rim was the text drawn again once per
/// stamp.
mod text;
pub use text::{text_paint_callback, FontAtlas, GlyphInstance, SlideAxis, TextRing};

/// The lattice's own labels: the glyphs of every node name it wants drawn,
/// and which node each of them belongs to.
///
/// These do NOT go through [`text_paint_callback`]. A node name is drawn
/// inside the lattice's own scene pass, at its node's place in the back-to-
/// front order, so a nearer node covers a name behind it by ordinary alpha
/// blending — the same way it covers the sheet behind it. That is the whole
/// reason this arrives here rather than as a pass of its own over the
/// finished picture, where no amount of masking can reconstruct what is
/// BEHIND a name at a pixel.
///
/// One thing follows from the pass it lands in, and it is visible: the
/// offscreen target is sized at `Scene::render_scale`, so text drawn into it
/// is rasterized at that size and resampled by the composite. At 1 that is
/// nothing; at 0.5 a name is as soft as the lattice under it, where it used
/// to stay native-resolution whatever the picture did.
///
/// The bloom does NOT follow, though it would from a single-attachment pass:
/// see [`Offscreen::nodes_view`], which is the copy the bright pass reads.
#[derive(Default)]
pub struct LatticeLabels {
    /// Every glyph of every label, one label's glyphs contiguous, in the
    /// order [`labels`](Self::labels) names them.
    ///
    /// Rects are in the PANE's own points — the callback rect's top-left
    /// corner is the origin — because the pass they are drawn in is the
    /// pane's, not the screen's.
    pub glyphs: Vec<GlyphInstance>,
    /// One entry per label, naming its node and how many of `glyphs` are
    /// its own.
    pub labels: Vec<Label>,
    /// The halo's two rings, as [`text_paint_callback`] takes them.
    pub rings: [TextRing; 2],
    /// egui's font atlas, on the frames the renderer's mirror of it is
    /// stale. `None` on the rest, which is nearly all of them.
    pub atlas: Option<FontAtlas>,
    /// And the drawn marks' own sheet, on the frames it has moved.
    pub marks: Option<FontAtlas>,
    /// The axis these names travel along, for the reconstruction filter — see
    /// [`SlideAxis`]. `Across` here is the default for want of an answer: an
    /// orbiting camera moves a node name both ways at once.
    pub slide: SlideAxis,
}

/// One label: which node it names, and how many glyphs it is.
///
/// The node is an index into `Scene::nodes` rather than a position, because
/// what a label needs from its node is its place in the DRAW ORDER — which
/// the callback works out for itself, sorting and culling as it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub node: u32,
    pub glyphs: u32,
}

// Shells name texture formats through this re-export so every crate agrees
// on the wgpu version.
pub use egui_wgpu::wgpu;

const SHADER_SRC: &str = include_str!("shaders/lattice.wgsl");
const BLIT_SRC: &str = include_str!("shaders/blit.wgsl");

/// What the bloom multiplies its blurred quarter by, out of whatever the bar
/// or a saved blob hands over: below zero is off, and above the ceiling is
/// the ceiling.
///
/// One function rather than a bound at each place a strength is read, because
/// the lattice, the piano roll and the spiral's dots take the SAME number and
/// the whole claim [`BloomChain`] rests on is that it means one halo in every
/// picture. A bound applied to one of them alone is a light a node has that its
/// ribbon does not, which is a difference between them that says nothing.
pub fn bloom_strength(raw: f32) -> f32 {
    raw.clamp(0.0, 4.0)
}

/// Depth format of the offscreen pass. Written for future depth-reading
/// effects; the scene pipelines test `Always` so it never affects output.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Clamp on the render-scale view setting, over whatever the UI offers.
const RENDER_SCALE_RANGE: (f32, f32) = (0.25, 4.0);

/// Entry points a (re)loaded shader must provide. The `_scene` pair is the
/// two-attachment form the offscreen pass draws through; the bare pair is
/// the single-attachment one the parity test's reference path uses.
#[cfg(any(test, feature = "hot-reload"))]
const REQUIRED_ENTRY_POINTS: &[&str] =
    &["vs_main", "fs_main", "fs_main_scene", "vs_edge", "fs_edge", "fs_edge_scene"];

/// Watches the shader source on disk (dev builds only). The first sighting
/// of the file only records a baseline mtime; edits after launch trigger
/// reloads.
#[cfg(feature = "hot-reload")]
struct ShaderWatcher {
    path: std::path::PathBuf,
    mtime: Option<std::time::SystemTime>,
    next_check: std::time::Instant,
}

#[cfg(feature = "hot-reload")]
impl ShaderWatcher {
    fn new() -> Self {
        ShaderWatcher {
            path: std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/lattice.wgsl"
            )),
            mtime: None,
            next_check: std::time::Instant::now(),
        }
    }


    /// Returns the new shader source when the file changed since last poll.
    fn poll(&mut self) -> Option<String> {
        let now = std::time::Instant::now();
        if now < self.next_check {
            return None;
        }
        self.next_check = now + std::time::Duration::from_millis(500);

        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()?;
        match self.mtime.replace(mtime) {
            None => None, // baseline; the baked shader is current
            Some(previous) if previous == mtime => None,
            Some(_) => std::fs::read_to_string(&self.path).ok(),
        }
    }
}

/// Parse + validate WGSL and check our entry points exist, so a bad edit
/// never reaches wgpu's panicking error handler. Also exercised by a unit
/// test against the baked-in source: plugin builds have no hot-reload, so
/// without that test a broken commit would first surface as a crash inside
/// a DAW at first paint.
#[cfg(any(test, feature = "hot-reload"))]
fn validate_wgsl(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("{e:?}"))?;
    for required in REQUIRED_ENTRY_POINTS {
        if !module.entry_points.iter().any(|ep| ep.name == *required) {
            return Err(format!("missing entry point `{required}`"));
        }
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    /// x unused — it carried the scene clock, which no shader stage reads any
    /// more: the shimmer was its one consumer and it now takes that clock
    /// already multiplied by its speed and reduced onto one cycle
    /// (`misc8.x`), which is the only form in which an f32 can carry a song
    /// position honestly. Retired rather than refilled WITH the clock: two
    /// spellings of the same time in the same buffer is how a later pattern
    /// picks the one that stair-steps.
    /// y: base node radius (world units); z unused;
    /// w unused — it carried the node style, the core orb's paint, from when
    /// the core had more than one. A retired slot rather than a repack, which
    /// would renumber the ones around it for nothing (as with `misc4.y`).
    misc: [f32; 4],
    /// x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    /// (the shader converts its screen-pixel AA softness to render
    /// pixels with it); w: bloom strength, which blit.wgsl reads (this
    /// slot is NOT free). The dots style maps a dot's pitch through x/y
    /// to index `pitch_lut`.
    misc2: [f32; 4],
    /// x: core radius in quad UV units (0 turns the core off); y/z: the
    /// outer octave layer's inner/outer band radii (same units), which the
    /// scene hands over either as z > y or as the empty pair (0, 0) that says
    /// the layer is off — the shader gates the band on z > y rather than
    /// assuming it (`glyph_band`'s two soft edges cross instead of cancelling
    /// at z == y, painting a dot at the node's centre); w: the outer edge of
    /// the outermost RING the node draws (`Scene::rings_outer`), which
    /// `node_rim` and the mark strip stand off, so neither has to know
    /// which layer was the last one on. None of the four is free.
    misc3: [f32; 4],
    /// Pitch->color lookup for the dots octave style (see harmonigraph_scene's
    /// `pitch_ramp_lut`), matching the node disc gradient.
    pitch_lut: [[f32; 4]; harmonigraph_scene::PITCH_LUT_N],
    /// x: core solidity (0 = soft glow, 1 = solid orb), the single axis the
    /// core layer runs on; y/z/w unused — y carried the outer solidity, now
    /// fixed crisp in the shader, and z/w the idle marker's radius and
    /// style, from when an unlit node drew a placeholder. Retired in place,
    /// like `misc.x`. (The blit pipeline binds only the head of this
    /// buffer, so trailing fields are safe to add here.)
    misc4: [f32; 4],
    /// x: grid line thickness as a multiple of the shader's built-in grid
    /// width; y unused (a retired slot rather than a repack, like `misc4.y`);
    /// z: padding inside the octave layer in quad UV units — the gap between
    /// neighbouring sectors AND between the band and the marks;
    /// w: how far a melody/bass mark reaches past the band, same units, where
    /// 0 means no marks (so this slot is NOT free — `mark_extension` reads it,
    /// and the octave layer gates the marks on it). Every earlier misc slot is
    /// spoken for, so the grid's knob starts a new one — safe per the note on
    /// `misc4`.
    misc5: [f32; 4],
    /// x/y unused — they carried the trail's mark style and strength, from
    /// when a memory was a change to the idle marker rather than a kept note
    /// name. Retired in place.
    /// z: the sevens knockout's fade width, in the uv of a full-size node
    /// (`Scene::sevens_soft`); w: the melody/bass marks' shimmer pattern
    /// (0 off, then one index per pattern — see `Pulse::shader_index`).
    misc6: [f32; 4],
    /// The ground the lattice is drawn onto — the pane fill this pass gets
    /// composited over — as the sevens knockout's target color. Without it
    /// the gutter can only knock out to black, which on this skin is
    /// several shades DARKER than the pane and so reads as a blob sitting
    /// on the picture rather than as a hole through it. See
    /// `Scene::background`.
    background: [f32; 4],
    /// The unlit ground a node's two rings stand on (`Scene::lattice_ground`): the
    /// neutral grey the OCTAVE band's silent slices are, and the colour a
    /// sounding one's pitch is painted over as it fades.
    ///
    /// A slot of its own beside `background` rather than three of the retired
    /// scalars: it is a colour, the buffer's other colour has one, and a grey
    /// split across the seam between two vec4s would be read by nothing that
    /// wanted the halves apart. The audio ring's own copy of it is `t` = 0 of
    /// `spectral_lut` below, baked on the CPU from the same `L*`.
    lattice_ground: [f32; 4],
    /// The wheel's pitch axis. x: octaves one turn is cut into
    /// (`OctaveLayout::span`); y: the MIDI pitch at the top of every node
    /// (`OctaveLayout::center`).
    ///
    /// No slot range and no per-node angle: both depend on the node's own
    /// pitch class — which of its octaves are the ones nearest the center, and
    /// how far its ring is turned to put them on their pitches — so the shader
    /// derives them per node from these two.
    ///
    /// z/w: the audio ring's inner and outer radius in quad UV units
    /// (`SpectralPaint::inner` / `::outer`), both 0 when the ring is off — the
    /// shader draws nothing for an empty annulus, so the toggle reaches it as
    /// geometry. They ride here rather than in a slot of their own because the
    /// ring is the same wheel at smaller radii: the shader reads the span and
    /// the center beside them for every wedge it draws.
    misc7: [f32; 4],
    /// The shimmer's knobs (see
    /// `Scene::shimmer_speed`). x: how far the sheet has travelled, in world
    /// units, already reduced onto one cycle of the pattern — the SPEED does
    /// not reach the shader, having been spent producing this (see
    /// `Scene::shimmer_slide`);
    /// y: the pattern's period in world units, strictly positive; z: how deep
    /// the light is (0 none, 1 the tuned depth); w: how gradually it arrives
    /// across the period (0 a crest, 1 a cosine).
    ///
    /// One slot for the set rather than the free `misc7.w` plus a new one:
    /// they are read together in one function, and splitting them across the
    /// seam between two vec4s buys nothing but a second place to look.
    misc8: [f32; 4],
    /// `OctaveLayout::bounds` — the angle from a ring's seam to each of its
    /// slice boundaries, the same table for every node — four to a row, which
    /// is how a uniform array is laid out anyway.
    oct_bounds: [[f32; 4]; 3],
    /// The audio ring's knobs (see `harmonigraph_scene::SpectralPaint`).
    /// x: how many cents of spectrum one wedge of the ring spans, read only
    /// when y is 0; y: 1 where each wedge is ONE reading taken at its own
    /// octave's pitch (`SpectralReading::Fold`) rather than a window spread
    /// across it (`::Spectrum`); z/w unused.
    misc9: [f32; 4],
    /// The FREQUENCY colour scheme's ramp — the analyzer's own gradient
    /// (`SpectrumConfig::spectrogram_gradient`) through `pitch_ramp_lut`, the
    /// same gradient the spectrogram's cells and the Spiral pane's segments
    /// are read off, with its silent end moved onto the node's own ground
    /// (`harmonigraph_scene::ring_gradient`) so a level reads as the same light
    /// over a grey ground as it does over their black one.
    /// Indexed by a LEVEL where `pitch_lut` beside it is indexed by a pitch,
    /// which is the whole difference between the two schemes.
    spectral_lut: [[f32; 4]; harmonigraph_scene::PITCH_LUT_N],
    /// The analyzer's loudness at every bucket of its pitch grid, a byte
    /// apiece, sixteen to a row (see `SPECTRUM_WORDS`).
    ///
    /// In the uniform buffer rather than a texture, and it is 3.8 KB of it.
    /// What a texture would buy is a sampler's own bilinear read; what it
    /// costs is a bind-group entry on every lattice pipeline, a texture per
    /// SURFACE — the docked pane and the Render preview both draw a lattice in
    /// one frame, and a `write_texture` is ordered ahead of the shared encoder
    /// egui-wgpu submits, so one shared texture would hand both panes whichever
    /// spectrum was written last (the trap `mirror_sheets` documents) — and a
    /// second upload path beside the uniforms, which are already per pane and
    /// already carry a lookup table of their own. The interpolation is two
    /// unpacks and a mix.
    spectrum: [[u32; 4]; SPECTRUM_WORDS],
}

/// Rows of four `u32` the analyzer's grid packs into: sixteen levels to a row.
///
/// A byte per bucket, little-endian within each word, exactly as the octave
/// levels are packed into a vertex attribute — one packing convention in the
/// renderer rather than two.
const SPECTRUM_WORDS: usize = harmonigraph_scene::SPECTRAL_BUCKETS.div_ceil(16);

// Two fixed-size GPU homes, one per table, and both are exact rather than
// one-sided ceilings — the uploads below fill every entry unconditionally, so
// a SMALLER constant in harmonigraph-scene reads off the end of a shorter
// array and panics on the first frame, which a ceiling would wave through.
//
// 3 u32 words hold 12 packed levels, one byte per octave slot.
const _: () = assert!(harmonigraph_scene::OCTAVE_SLOTS == 11);
// `oct_bounds`'s 3 vec4s hold 12 boundary angles, and the layout needs one per
// slice plus the closing one — so a span of 11 is the ceiling, which is also
// every MIDI octave there is. Raising MAX_SPAN in harmonigraph-scene is what
// would break this.
const _: () = assert!(harmonigraph_scene::MAX_SPAN as usize + 1 == 12);

// The shader declares `pitch_lut` with a literal length; keep the two in
// lockstep so the uniform buffer and the WGSL agree. `spectral_lut` beside it
// is the same length by construction — one gradient table shape, two tables.
const _: () = assert!(harmonigraph_scene::PITCH_LUT_N == 64);

// The analyzer's grid, which lattice.wgsl also declares as literals
// (SPECTRUM_BUCKETS, BUCKETS_PER_SEMITONE, SPECTRUM_MIN_MIDI, and the length
// of the `spectrum` array). A mismatch here is a ring reading the wrong
// buckets at the wrong pitches, which draws a plausible picture of nothing —
// so the numbers are asserted rather than trusted to stay in step.
const _: () = assert!(harmonigraph_scene::SPECTRAL_BUCKETS == 3828);
const _: () = assert!(harmonigraph_scene::SPECTRAL_BUCKETS_PER_SEMITONE == 32);
const _: () = assert!(harmonigraph_scene::SPECTRAL_AXIS.0 == 15.486_82);
const _: () = assert!(SPECTRUM_WORDS == 240);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuInstance {
    world_pos: [f32; 3],
    color: [f32; 4],
    /// x: activation, y: melody mark level, z: bass mark level (see
    /// lattice.wgsl). The mark levels ride with the activation rather than in
    /// a vertex attribute of their own: all three are levels the same node
    /// draws at, read together by the layers that draw it.
    params: [f32; 3],
    /// Per-octave activation, 8 bits per slot, little-endian packed
    /// (slot 0 = lowest byte of the first word).
    octaves: [u32; 3],
    /// The node's pitch class in cents (0..1200). It both PLACES the octave
    /// indicators and COLORS them, off the one quantity: an indicator's
    /// octave has a pitch, that octave's C plus this, and the indicator sits
    /// at that pitch's angle on the shared axis and in that pitch's color
    /// (see `harmonigraph_scene::octaves`).
    cents: f32,
    /// Melody/bass marks: `[melody_slots, bass_slots]`, one bit per octave
    /// slot (see `NodeInstance::melody_slots`). Kept as integers rather
    /// than folded into the dead `params.y`/`params.z` floats because the
    /// shader masks them bitwise, which needs a flat-interpolated `u32`.
    marks: [u32; 2],
    /// Each mark's own color (see `NodeInstance::melody_color`): the sector it
    /// links back to, not a fixed livery, so a ring reads as belonging to the
    /// indicator it points at.
    melody_color: [f32; 4],
    bass_color: [f32; 4],
    /// The sevens layer, packed: x = billboard size factor (1 on the home
    /// sheet), y = knockout gutter width in uv units (0 on the home sheet).
    /// See `NodeInstance::scale` / `::gutter`.
    sevens: [f32; 2],
    /// How much of the audio ring this node wears, 0..=1: the gate's answer for
    /// its wedges carried on the note Fade, floored by the node's own envelope
    /// (`NodeInstance::audio_ring`).
    ///
    /// A DECISION already taken and not the node's own peak level with the gate
    /// beside it in the uniforms, though there is a free slot there for one:
    /// the rule is "the loudest wedge reaches the gate", the levels and the
    /// wheel it is measured over both live on the CPU, and splitting the
    /// comparison across the bus would leave two places able to disagree about
    /// which nodes ring. What crosses is where that decision has GOT to, which
    /// is a level because a ring arrives and leaves on the Fade like every
    /// other layer of a node (see `harmonigraph_scene::RingFade`).
    ring: f32,
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        // Locations 5 and 9 are absent, not renumbered. Both are retired
        // slots — 5 the home-sheet flag, 9 the trail level, each read only by
        // an idle marker the nodes do not draw. The audio ring's own slot is
        // 11, and it carries how far the layer is on at this node rather than
        // a reading: WHAT the ring says is a window onto the shared spectrum
        // in the uniforms, and how much of one this node wears is the
        // per-node half of it (`GpuInstance::ring`). The macro
        // names each location and takes each OFFSET from the sequence, so a
        // dropped entry shrinks the stride to match the struct without moving
        // the rest off their numbers — which is what keeps this list and
        // lattice.wgsl's `Instance` readable side by side.
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Uint32x3,
            4 => Float32, 6 => Uint32x2,
            7 => Float32x4, 8 => Float32x4, 10 => Float32x2, 11 => Float32
        ],
    };
}

/// Pack the analyzer's per-bucket levels into the rows `spectrum_level()` in
/// lattice.wgsl unpacks: a byte per bucket, little-endian within each `u32`,
/// sixteen buckets to a row.
///
/// Already quantized on the way in — [`harmonigraph_scene::SpectralLevels`] is
/// bytes — so this is a repack and not a second rounding: the level a wedge
/// paints is exactly the level the fold measured, to the byte.
fn pack_spectrum(levels: &harmonigraph_scene::SpectralLevels) -> [[u32; 4]; SPECTRUM_WORDS] {
    let mut rows = [[0u32; 4]; SPECTRUM_WORDS];
    for (bucket, &level) in levels.iter().enumerate() {
        rows[bucket / 16][(bucket / 4) % 4] |= u32::from(level) << ((bucket % 4) * 8);
    }
    rows
}

/// Pack the per-octave activation levels into the bit layout
/// `octave_level()` in lattice.wgsl unpacks: 8 bits per slot,
/// little-endian (slot 0 = lowest byte of the first word).
fn pack_octaves(levels: &[f32; harmonigraph_scene::OCTAVE_SLOTS]) -> [u32; 3] {
    let mut octaves = [0u32; 3];
    for (slot, &level) in levels.iter().enumerate() {
        let byte = (level.clamp(0.0, 1.0) * 255.0).round() as u32;
        octaves[slot / 4] |= byte << ((slot % 4) * 8);
    }
    octaves
}

/// One edge-pipeline instance: a chord beam between two active adjacent
/// nodes, or a faint background grid line.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuEdge {
    /// xyz: endpoint A, w: strength (grid lines: opacity).
    a_strength: [f32; 4],
    /// xyz: endpoint B, w: kind (0 chord beam, 1 grid line, 2 dashed
    /// grid line).
    b_kind: [f32; 4],
    color: [f32; 4],
}

impl GpuEdge {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuEdge>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4
        ],
    };
}

/// Build the egui shape that renders `scene` into `rect`. `pane_id` must be
/// unique per lattice view shown in the same frame (each gets its own GPU
/// buffers; the pipeline is shared).
/// Where the lattice callback publishes what it measures about itself.
///
/// An atomic bag rather than return values because none of this comes back up
/// the call stack that asked for it: `prepare` runs inside egui-wgpu, and the
/// GPU timing arrives several frames after the frame it describes. All three
/// are f32 bits.
#[derive(Default)]
pub struct LatticeStats {
    /// GPU time of the lattice's passes, carrying the
    /// [`GPU_TIME_UNSUPPORTED`] / [`GPU_TIME_PENDING`] sentinels.
    pub gpu_ms: std::sync::atomic::AtomicU32,
    /// Wall time of the whole `prepare` callback. egui-wgpu runs this from
    /// inside `update_buffers`, so it is billed to the frame's upload stage
    /// and is invisible from outside.
    ///
    /// "Prepare" undersells it, and the three fields below exist because the
    /// name misled for a long time: this callback does not merely stage data.
    /// It also ENCODES the lattice's whole scene pass and the four-pass bloom
    /// chain onto egui's encoder — the largest piece of CPU work in the
    /// frame, sitting inside a row the overlay calls "buf up".
    pub prepare_ms: std::sync::atomic::AtomicU32,
    /// Of that, the time in `device.poll` draining the timestamp readback:
    /// what the GPU measurement costs to take. Kept separate so the
    /// instrumentation can be caught spending the budget it exists to
    /// measure.
    pub poll_ms: std::sync::atomic::AtomicU32,
    /// Of that, staging this frame's data: sizing the offscreen targets,
    /// recreating them when the size moved, the `queue.write_buffer` calls for
    /// instances, edges, labels and both sets of uniforms, and — on the rare
    /// frame that brings one — the copy of egui's font atlas the labels are
    /// read out of.
    pub write_ms: std::sync::atomic::AtomicU32,
    /// Of that, encoding the scene pass and the bloom chain — five
    /// `begin_render_pass` calls and the draws inside them. No GPU work
    /// happens here; this is the cost of BUILDING the command stream.
    ///
    /// Split from `write_ms` because the two answer different questions and
    /// move for different reasons. A cost that tracks the node count is
    /// staging; a cost that does not is the encoder, and the fixes have
    /// nothing in common.
    pub scene_ms: std::sync::atomic::AtomicU32,
}

/// `stats` receives this pane's own measurements. Pass `None` for panes whose
/// cost isn't the one being reported, so a second lattice on screen can't
/// overwrite the readings.
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    labels: LatticeLabels,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    stats: Option<std::sync::Arc<LatticeStats>>,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        LatticeCallback::from_scene(scene, labels, rect.size(), target_format, pane_id, stats),
    )
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    instances: Vec<GpuInstance>,
    /// Index into `instances` where the grid is drawn (see `from_scene`).
    grid_at: u32,
    /// Every label's glyphs, in the order the pass draws them, and where each
    /// label falls in the node run (see [`GlyphSeam`]).
    glyphs: Vec<GlyphInstance>,
    seams: Vec<GlyphSeam>,
    /// The halo's rings, and the two sheets on the frames either has moved.
    rings: [TextRing; 2],
    atlas: Option<FontAtlas>,
    marks: Option<FontAtlas>,
    /// Which way these names travel, for the glyph shader's filter.
    slide: SlideAxis,
    edges: Vec<GpuEdge>,
    uniforms: Uniforms,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    /// The callback rect's size in egui points; `prepare` multiplies by the
    /// screen's pixels-per-point and `render_scale` to size the offscreen
    /// target.
    size_points: [f32; 2],
    /// From the scene (a view setting), clamped to [`RENDER_SCALE_RANGE`].
    render_scale: f32,
    /// Where to publish this pane's own measurements.
    stats: Option<std::sync::Arc<LatticeStats>>,
}

/// One pass of the bloom chain: the pipeline to run, its bind group, and the
/// texture it renders into. See [`BloomChain::run`].
type BloomStep<'a> = (&'a wgpu::RenderPipeline, &'a wgpu::BindGroup, &'a wgpu::TextureView);

/// One label's glyphs, and where they are drawn: `at` is how many node
/// instances go in front of them.
///
/// A label sits immediately after the node it names, so the nodes drawn
/// after it — everything nearer — cover it exactly as they cover each other.
/// Labels sharing a seam are one entry, since they are one uninterrupted
/// draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GlyphSeam {
    at: u32,
    start: u32,
    count: u32,
    /// Whether the node this names is in the home run, which draws AFTER the
    /// grid — carried rather than inferred from `at`, because the two runs
    /// meet at `grid_at` and a culled node sits on the boundary without
    /// having moved it. The last far-sheet node to ship and a home node
    /// culled before any home node ships both land on `at == grid_at`, and
    /// they belong on opposite sides of the grid.
    after_grid: bool,
}

/// Put every label at its node's place in the draw order: the glyphs
/// regrouped into the order the pass wants them, and the seams that say
/// where each group goes.
///
/// `seam_of` is per node of the scene, as `from_scene` counted it. A label
/// naming a node that is not in the scene is dropped rather than guessed at,
/// and its glyphs go with it — the caller and the scene disagreeing about
/// how many nodes there are is a bug in the caller, and drawing the name
/// somewhere arbitrary would hide it.
fn place_labels(
    glyphs: Vec<GlyphInstance>,
    labels: &[Label],
    seam_of: &[(u32, bool)],
) -> (Vec<GlyphInstance>, Vec<GlyphSeam>) {
    // Where each label's glyphs sit in what the caller handed over, paired
    // with the seam they are going to. The cursor advances over labels the
    // scene has no node for as much as over the rest: the run lengths are
    // what say which glyphs are whose.
    let mut cursor = 0usize;
    let mut runs: Vec<((u32, bool), usize, usize)> = Vec::with_capacity(labels.len());
    for label in labels {
        let start = cursor;
        let count = label.glyphs as usize;
        cursor = (cursor + count).min(glyphs.len());
        if let Some(&seam) = seam_of.get(label.node as usize) {
            runs.push((seam, start, cursor - start));
        }
    }
    // Stable, so two labels at one seam keep the order they were drawn in —
    // which is the order the nodes are in, and the only thing that decides
    // between two names sharing a pixel. By the side of the grid before the
    // count, so the two labels that share `grid_at` from opposite runs sort
    // the way they draw rather than by which node came first.
    runs.sort_by_key(|&((at, after_grid), _, _)| (at, after_grid));

    let mut placed = Vec::with_capacity(glyphs.len());
    let mut seams: Vec<GlyphSeam> = Vec::new();
    for ((at, after_grid), start, count) in runs {
        if count == 0 {
            continue;
        }
        let first = placed.len() as u32;
        placed.extend_from_slice(&glyphs[start..start + count]);
        // Merged only with a seam on the SAME side of the grid: two labels
        // at one `at` are one uninterrupted draw, unless the grid goes
        // between them.
        match seams.last_mut() {
            Some(last) if last.at == at && last.after_grid == after_grid => {
                last.count += count as u32
            }
            _ => seams.push(GlyphSeam { at, start: first, count: count as u32, after_grid }),
        }
    }
    (placed, seams)
}

impl LatticeCallback {
    fn from_scene(
        scene: &Scene,
        labels: LatticeLabels,
        size_points: egui::Vec2,
        target_format: wgpu::TextureFormat,
        pane_id: u64,
        stats: Option<std::sync::Arc<LatticeStats>>,
    ) -> Self {
        let aspect = size_points.x / size_points.y.max(1.0);
        let render_scale = scene
            .render_scale
            .clamp(RENDER_SCALE_RANGE.0, RENDER_SCALE_RANGE.1);
        let camera = scene.camera;
        let view_proj = camera.view_proj(aspect);
        let (right, up) = camera.right_up();

        // Sort back-to-front along the view direction. The offscreen pass
        // does have a depth attachment, but its test is `Always` (see
        // create_scene_pipeline), so alpha blending still relies on draw
        // order — exactly as it did before the offscreen pass existed.
        //
        // Sheets back to front FIRST, then painter's order within a sheet.
        // That is still just back-to-front — world z IS the sevens axis, and
        // the first key is only its depth — but it stays EXACT when the
        // camera is orbited, where two nodes on one sheet have different
        // depths and a plain depth sort interleaves the sheets. Interleaving
        // is not a cosmetic problem: it puts the grid in the wrong place in
        // the order and leaves the home sheet's clearings with almost
        // nothing drawn before them to clear, so the knockout quietly did
        // nothing under perspective and orthographic while working under
        // cabinet (where every home node shares one depth and the two sorts
        // agree).
        //
        // Do not reorder the sheets on top of this. Forcing the home sheet
        // to the bottom (so off-sheet notes could never be hidden by it)
        // inverts the far half of the axis: the sheet BEHIND home then draws
        // last, and its clearing takes a bite out of the home sheet in front
        // of it. Grouping by distance from home does the same thing more
        // thoroughly. Depth is what the reader is being shown; it is what
        // the order has to follow.
        //
        // The `forward.z` factor is what keeps it honest if the view is
        // orbited right around past the sheets: which way along z is "away"
        // is the camera's business, not an assumption.
        let eye = camera.eye();
        let forward = (camera.target - eye).normalize_or_zero();
        let sheet_depth = |n: &harmonigraph_scene::NodeInstance| n.world_pos.z * forward.z;
        // Carrying the node's INDEX rather than the node itself, because a
        // label names one and has to be put back beside it after the sort.
        let mut order: Vec<(f32, f32, usize)> = scene
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (sheet_depth(n), (n.world_pos - eye).dot(forward), i))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.total_cmp(&a.1)));

        // The grid belongs to the home sheet — that is the only sheet that
        // draws one — so its place in the order is between the sheets behind
        // it and the home sheet itself. Under it, a node on a sheet BEHIND
        // the home one punches its clearing through the home grid, putting
        // a hole in the layer that is supposed to be hiding it.
        //
        // The home sheet then draws after the grid, so a home node's
        // clearing cuts the grid lines as well as the sheets behind — the
        // node sits in a clean gap in the lattice rather than on top of it.
        // (Drawing the home clearings in a pass of their own ahead of the
        // grid would spare the lines; the lines are wanted cut.)
        //
        // World z is measured from the home sheet, so its whole run sits at
        // sheet depth 0 — behind it is positive, in front negative. Sorting
        // by that above is what makes the home sheet one contiguous run,
        // under every projection rather than only the face-on one.
        let to_gpu = |n: &harmonigraph_scene::NodeInstance, gutter: f32| GpuInstance {
            world_pos: n.world_pos.to_array(),
                color: n.color.to_array(),
                params: [n.activation, n.melody_level, n.bass_level],
                octaves: pack_octaves(&n.octaves),
                cents: n.cents,
                marks: [n.melody_slots, n.bass_slots],
                melody_color: n.melody_color.to_array(),
                bass_color: n.bass_color.to_array(),
                sevens: [n.scale, gutter],
                ring: n.audio_ring,
        };

        let split = order
            .iter()
            .position(|&(plane, _, _)| plane <= 0.0)
            .unwrap_or(order.len());
        // A node that can paint nothing is not shipped at all. The shader
        // already discards it per fragment, but the billboard is deliberately
        // bigger than the node (QUAD_MARGIN and then some), so the discard is
        // paid a fragment at a time over a quad the disc never reaches — and
        // an unplayed lattice is ENTIRELY such nodes: an idle node draws no
        // marker and carries no trail mark, so a still lattice ships its grid
        // and nothing else.
        //
        // The gates are the ones `fs_main`'s idle branch reads, in the same
        // order, off the packed instance rather than the scene node — so this
        // asks the question the shader answers, not a restatement of it that
        // could drift. Reading the PACKED octave word is what makes that exact
        // rather than close: an octave level under half a byte quantizes to
        // zero on the way to the GPU, and a node dropped for that is a node
        // the shader would have discarded anyway.
        //
        // The audio RING is why this is not a property of the node alone: the
        // ring is a window onto the spectrum rather than a level a node
        // carries, so it takes BOTH the layer being on and this node wearing
        // some of the ring (`Scene::wear_audio_rings`) for the node to owe an
        // annulus. With the gate at its floor that is every node in the window,
        // silence included — the ungated picture, and what "the ring reads raw"
        // costs; dialled up, an idle node with nothing sounding at it goes back
        // to shipping nothing at all, ONCE ITS FADE HAS RUN OUT — the level
        // reaches exactly 0 rather than approaching it, so a ring on its way
        // out is shipped for exactly as long as it is drawn. The shader's idle
        // branch pays the rest per fragment, keeping an otherwise idle node to
        // the ring's own annulus and the hole that ring clears around it.
        let ringing = scene.spectral.ring_draws();
        let paints = |g: &GpuInstance| {
            (ringing && g.ring > 0.0)
                || g.params[0] > 0.0
                || g.params[1] > 0.0
                || g.params[2] > 0.0
                || (g.octaves[0] | g.octaves[1] | g.octaves[2]) != 0
        };
        // Where a node's own label goes, per node: after everything drawn up
        // to and including that node, counted over the KEPT instances. Not its
        // index in the sorted list — the cull above drops a node that can paint
        // nothing, and such a node can still carry a label (a hovered idle one
        // draws no disc and is named all the same), so the two part company at
        // the first culled node.
        let mut seam_of = vec![(0u32, false); scene.nodes.len()];
        let drawn = |out: &mut Vec<GpuInstance>,
                     seam_of: &mut [(u32, bool)],
                     ns: &[(f32, f32, usize)],
                     after_grid: bool| {
            for &(_, _, i) in ns {
                let node = &scene.nodes[i];
                let instance = to_gpu(node, node.gutter);
                if paints(&instance) {
                    out.push(instance);
                }
                seam_of[i] = (out.len() as u32, after_grid);
            }
        };
        let mut instances = Vec::with_capacity(order.len());
        drawn(&mut instances, &mut seam_of, &order[..split], false);
        // Where the grid is drawn inside that run: after the sheets BEHIND the
        // home one, counted over the kept instances rather than over `split`,
        // which indexes the list before the cull.
        let grid_at = instances.len() as u32;
        drawn(&mut instances, &mut seam_of, &order[split..], true);
        let (glyphs, seams) = place_labels(labels.glyphs, &labels.labels, &seam_of);

        // The grid draws under the nodes.
        let edges = scene
            .grid
            .iter()
            .map(|g| (g, if g.dashed { 2.0 } else { 1.0 }))
            .map(|(e, kind)| GpuEdge {
                a_strength: [e.a.x, e.a.y, e.a.z, e.strength],
                b_kind: [e.b.x, e.b.y, e.b.z, kind],
                color: e.color.to_array(),
            })
            .collect();

        LatticeCallback {
            instances,
            grid_at,
            glyphs,
            seams,
            rings: labels.rings,
            atlas: labels.atlas,
            marks: labels.marks,
            slide: labels.slide,
            edges,
            uniforms: Uniforms {
                view_proj: view_proj.to_cols_array(),
                cam_right: right.extend(0.0).to_array(),
                cam_up: up.extend(0.0).to_array(),
                misc: [0.0, scene.node_radius, 0.0, 0.0],
                misc2: [
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    render_scale,
                    bloom_strength(scene.bloom_strength),
                ],
                misc3: [
                    scene.core_radius,
                    scene.outer_inner,
                    scene.outer_outer,
                    scene.rings_outer,
                ],
                pitch_lut: std::array::from_fn(|k| scene.pitch_lut[k].to_array()),
                misc4: [scene.core_solidity, scene.mark_inner, 0.0, 0.0],
                misc5: [scene.grid_thickness, 0.0, scene.octave_gap, scene.mark_thickness],
                misc6: [0.0, 0.0, scene.sevens_soft, scene.pulse_marks.shader_index() as f32],
                background: scene.background.to_array(),
                lattice_ground: scene.lattice_ground.to_array(),
                misc7: [
                    scene.octave_layout.span as f32,
                    scene.octave_layout.center,
                    scene.spectral.inner,
                    scene.spectral.outer,
                ],
                misc8: [
                    scene.shimmer_slide(),
                    scene.shimmer_width,
                    scene.shimmer_intensity,
                    scene.shimmer_softness,
                ],
                // Straight indexing: the table is exactly as long as the
                // rows are wide (the const assert above is what keeps it so),
                // and a fallback here would quietly ship a wheel with a wrong
                // angle in it rather than failing the build.
                oct_bounds: std::array::from_fn(|row| {
                    std::array::from_fn(|col| scene.octave_layout.bounds[row * 4 + col])
                }),
                misc9: [scene.spectral.range, f32::from(u8::from(scene.spectral.folded)), 0.0, 0.0],
                spectral_lut: std::array::from_fn(|k| scene.spectral.lut[k].to_array()),
                // Zeroed rather than packed when the ring is off: `u.spectrum`
                // is read only through `spectral_ring`, which draws nothing off
                // an empty annulus, so the fresh feature-off frame skips the
                // 3828-bucket pack — twice, docked pane and Render preview —
                // and the struct uploads whole either way.
                spectrum: if scene.spectral.ring_draws() {
                    pack_spectrum(&scene.spectral.levels)
                } else {
                    [[0u32; 4]; SPECTRUM_WORDS]
                },
            },
            target_format,
            pane_id,
            size_points: [size_points.x, size_points.y],
            render_scale,
            stats,
        }
    }

    /// Whether this callback may drive the shared [`GpuTimer`] — true only for
    /// the one carrying a stats sink, i.e. the pane that publishes the reading.
    ///
    /// There is ONE timer per device, and its three-step readback cycle assumes
    /// each step lands in a different frame — specifically that the encoder
    /// holding `close`'s `copy_buffer_to_buffer` has been submitted before the
    /// next `poll` asks the staging buffer to map. egui-wgpu submits once per
    /// frame, AFTER running every callback's `prepare` on one shared encoder,
    /// so that only holds while a single callback drives the cycle.
    ///
    /// Two do exist: the Video tab's preview is a second live lattice, and the
    /// frame it first appears in ran `prepare` twice — the docked pane
    /// recording the copy, then the preview immediately calling `map_async` on
    /// the buffer that copy still had to write. Submitting that encoder is a
    /// wgpu validation error ("Buffer with 'lattice_gpu_timer_staging' label is
    /// still mapped"), which is fatal by default and took the plugin down with
    /// it — reproducibly, the moment the preview came into view.
    ///
    /// Gating on the stats sink is also what the reading MEANS: the overlay
    /// reports the cost of the docked lattice, and letting the preview consume
    /// the cycle would have published the preview's frame time under the
    /// docked pane's name.
    fn drives_timer(&self) -> bool {
        self.stats.is_some()
    }

    /// The lattice's own pipelines, in the order [`BloomChain::run`] steps
    /// through them.
    fn bloom_pipelines(resources: &LatticeResources) -> BloomPipelines<'_> {
        BloomPipelines {
            bright: &resources.bright_pipeline,
            downsample: &resources.downsample_pipeline,
            blur_h: &resources.blur_h_pipeline,
            blur_v: &resources.blur_v_pipeline,
        }
    }
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct LatticeResources {
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    /// One sampled texture + the shared sampler (bloom chain passes).
    filter_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The node labels: the same glyph shader the rest of the UI's text
    /// draws through (`crate::text`), built for THIS pass — its format and
    /// its depth attachment — so a name takes its place in the scene's own
    /// back-to-front order instead of being laid over the finished picture.
    glyph_rim_pipeline: wgpu::RenderPipeline,
    glyph_fill_pipeline: wgpu::RenderPipeline,
    glyph_layout: wgpu::BindGroupLayout,
    glyph_sampler: wgpu::Sampler,
    /// This renderer's copies of the two sheets a glyph can be cut from —
    /// egui's font atlas and the drawn marks'. Its own, not the text
    /// callback's: a mirror answers for one texture, and these are two.
    atlas: text::MirroredAtlas,
    marks: text::MirroredAtlas,
    blank: wgpu::Texture,
    target_format: wgpu::TextureFormat,
    panes: HashMap<u64, PaneBuffers>,
    /// GPU-side timing of the lattice passes. `None` when the device didn't
    /// grant timestamp queries — plenty of GPUs (and the offline renderer,
    /// which never asks for the feature) don't, and the readout says so
    /// rather than pretending.
    timer: Option<GpuTimer>,
    #[cfg(feature = "hot-reload")]
    watcher: ShaderWatcher,
}

/// Wall-clock time the GPU spends on one pane's lattice passes, read back
/// with timestamp queries.
///
/// Deliberately a lagging measurement. The queries resolve into a buffer that
/// must be MAPPED to be read, mapping can only be requested once the encoder
/// is submitted (egui-wgpu owns the submit, one `prepare` later), and the map
/// completes whenever the driver gets to it. Blocking on any of that would
/// stall the very pipeline being measured, and the reading would then describe
/// a frame that was slow *because* it was timed. So it runs as a three-step
/// cycle and publishes a result a few frames old. For "is the GPU the
/// bottleneck", stale and honest beats fresh and self-inflicted.
struct GpuTimer {
    set: wgpu::QuerySet,
    /// `resolve_query_set` destination. Not mappable, hence the copy.
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
    state: TimerState,
    /// Set by the map callback, which the driver may run on another thread.
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 1x1 target for the trailing pass that carries the closing sample.
    /// One pixel, so beginning it costs nothing worth measuring.
    tail: wgpu::TextureView,
}

#[derive(PartialEq, Clone, Copy)]
enum TimerState {
    /// Nothing in flight; the next frame may record.
    Idle,
    /// Queries sit in an encoder that has not been submitted yet.
    Recorded,
    /// Submitted, and the staging buffer has been asked to map.
    Mapping,
}

/// Two timestamps, 8 bytes each.
const TIMER_BYTES: u64 = 16;

/// Published in place of a measurement when the device can't do timestamp
/// queries at all.
///
/// A NaN bit pattern, as is [`GPU_TIME_PENDING`]. Zero would have been the
/// obvious sentinel and is the wrong choice: a real reading of 0.0 ms is
/// perfectly possible, and using it to mean "nothing yet" is what made a
/// landed-but-zero measurement indistinguishable from a stuck one.
pub const GPU_TIME_UNSUPPORTED: u32 = 0x7fc0_0001;

/// The initial value: a timer exists, but no measurement has come back yet.
pub const GPU_TIME_PENDING: u32 = 0x7fc0_0002;

impl GpuTimer {
    /// Build the query set and buffers, or `None` when the device can't.
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("lattice_gpu_timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_gpu_timer_resolve"),
                size: TIMER_BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_gpu_timer_staging"),
                size: TIMER_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period: queue.get_timestamp_period(),
            state: TimerState::Idle,
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tail: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("lattice_gpu_timer_tail"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
        })
    }

    /// Advance the readback cycle, returning a measurement in milliseconds on
    /// the frame one finally lands.
    fn poll(&mut self, device: &wgpu::Device) -> Option<f32> {
        use std::sync::atomic::Ordering;
        match self.state {
            TimerState::Idle => None,
            TimerState::Recorded => {
                // The encoder holding those queries has been submitted by now,
                // so the map can be asked for. That is true because egui-wgpu
                // submits once per frame and only ONE callback per frame gets
                // here — see `LatticeCallback::drives_timer`, which is what
                // keeps a second lattice view from mapping this buffer between
                // the copy being recorded and the submit that performs it.
                let ready = self.ready.clone();
                self.staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        ready.store(true, Ordering::Release);
                    }
                });
                // Poll, never Wait: a stall here would be the measurement
                // interfering with what it measures.
                let _ = device.poll(wgpu::PollType::Poll);
                self.state = TimerState::Mapping;
                None
            }
            TimerState::Mapping => {
                let _ = device.poll(wgpu::PollType::Poll);
                if !self.ready.swap(false, Ordering::Acquire) {
                    return None;
                }
                let ms = {
                    let view = self.staging.slice(..).get_mapped_range();
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    // Saturating: both timestamps come off the same queue and
                    // should be ordered, but an out-of-order pair must not
                    // wrap into an astronomical reading.
                    let delta = ticks[1].saturating_sub(ticks[0]) as f64;
                    (delta * self.period as f64 / 1.0e6) as f32
                };
                self.staging.unmap();
                self.state = TimerState::Idle;
                Some(ms)
            }
        }
    }

    /// Whether this frame should be timed — false while a readback is still
    /// in flight, so the query set is never overwritten mid-cycle.
    fn arming(&self) -> bool {
        self.state == TimerState::Idle
    }

    /// The opening sample, to hang on the first lattice pass.
    ///
    /// Both samples are BEGINNING-of-pass writes. The obvious shape —
    /// `write_timestamp` on the encoder, or beginning-and-end on one pass —
    /// does not work here: Metal advertises and grants both
    /// `TIMESTAMP_QUERY_INSIDE_ENCODERS` and end-of-pass writes, then
    /// silently records ZERO for them. Only the beginning-of-pass sample
    /// comes back with a real value, so the bracket is built from two of
    /// those, the closing one on a pass that exists only to carry it.
    fn opening(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: None,
        })
    }

    /// Close the bracket with a 1x1 no-op pass, and stage the result for a
    /// later frame to map.
    fn close(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_gpu_timer_tail_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.tail,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: Some(1),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        encoder.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, TIMER_BYTES);
        self.state = TimerState::Recorded;
    }
}

struct PaneBuffers {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
    /// Where the grid is drawn inside the node run: instances before this
    /// are the sheets behind the home one plus the home sheet's own
    /// clearings, and must land under the grid; the rest go over it. See
    /// `LatticeCallback::from_scene`.
    grid_at: u32,
    edge_buffer: wgpu::Buffer,
    edge_capacity: usize,
    edge_count: u32,
    /// This pane's labels: the glyphs, and where each label falls in the node
    /// run above (see [`GlyphSeam`]).
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: usize,
    glyph_count: u32,
    seams: Vec<GlyphSeam>,
    /// What the glyph shader is told about this pane: its size in points,
    /// the atlas's, and the rim's rings.
    glyph_uniform_buffer: wgpu::Buffer,
    /// Names both mirrored sheets, so it is rebuilt whenever a fresh one has
    /// been uploaded — and `glyph_sheet_keys` is which uploads it names.
    glyph_bind_group: Option<wgpu::BindGroup>,
    glyph_sheet_keys: (u64, u64),
    offscreen: Option<Offscreen>,
}

/// The per-pane offscreen render target and bloom chain, recreated when
/// the pane's pixel size (or render scale) changes.
///
/// The scene target uses the render-scaled size; the bloom textures use
/// fractions of the pane's NATIVE screen size, so the halo's on-screen
/// width doesn't change with the render-scale setting.
struct Offscreen {
    color_view: wgpu::TextureView,
    /// The same picture with the node LABELS left out, written beside
    /// `color_view` by the scene pass's second attachment.
    ///
    /// The bright pass reads THIS, so a name is not in the bloom at all: it
    /// neither glows nor — the half that is easier to miss — takes a bite out
    /// of the halo of the node it covers, which is what a name in the bloom
    /// input does by standing where that node's own bright pixels were.
    ///
    /// Both halves measured, by rendering a frame four ways (labels on/off
    /// crossed with bloom on/off) and subtracting, which isolates the bloom
    /// TERM: text in the bright pass added up to 28/255 of light in its own
    /// halo, against the whole frame's bloom peaking at 33, and took up to
    /// 9/255 back out of the halo it crossed.
    ///
    /// A whole second colour target is what that costs, at the render-scaled
    /// size — about 14 MB for a Retina-sized pane at scale 1, and it grows
    /// with the square of the render scale like the two targets beside it.
    /// What it does NOT cost is time: the scene pass and bloom chain over 384
    /// overlapping lit nodes and 2300 glyphs at 1536x1024 median 0.359 ms with
    /// this attachment and 0.365 ms without, which is the same within noise.
    /// There is one more colour write per node fragment and nothing else — no
    /// extra pass, no extra draw call, no extra geometry.
    ///
    /// There is also no cheaper slot. The bright pass samples a finished
    /// texture, so "after bloom but still interleaved with the nodes" does not
    /// exist, and anything short of a second attachment (a stencil, a
    /// threshold) buys back the memory by punching a hole in the node's own
    /// halo where the name sits — which is the artifact this removes.
    nodes_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// The halo, grown from the scene WITHOUT its labels.
    bloom: BloomChain,
    /// Composite: scene color + blurred bloom (quarter A) + uniforms.
    composite_bind_group: wgpu::BindGroup,
    size: [u32; 2],
    screen_size: [u32; 2],
}

/// The shared, pane-independent objects an [`Offscreen`] binds against.
struct OffscreenShared<'a> {
    format: wgpu::TextureFormat,
    composite_layout: &'a wgpu::BindGroupLayout,
    filter_layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
}

/// The bloom post-process's targets and bind groups: a soft-knee threshold
/// into half the picture's SCREEN size, a plain downsample to a quarter, then
/// a separable blur ping-ponging between two quarter-res textures.
///
/// One chain, every picture. The lattice feeds it the scene without its
/// labels; the piano roll feeds it the notes rendered again offscreen
/// (`crate::roll`); the spiral's dots feed it through `crate::glow`. That they
/// are the same four steps in the same order over the same fractions is the
/// whole of what makes one bloom strength mean one halo, and it is a claim a
/// second copy cannot keep: the step that matters
/// most is WHERE the threshold sits, and a chain that thresholds after the
/// downsample instead of before it measures a thin shape that has already been
/// averaged twice, so a ribbon gets a fraction of the halo the node it lit up
/// gets from the identical color.
///
/// The blurs run at a quarter, which is what makes the halo wide and cheap;
/// the threshold runs at a half, which is what makes it measure the picture
/// rather than a smear of it.
struct BloomChain {
    /// The thresholded picture at half the screen size.
    half_view: wgpu::TextureView,
    /// The blur's ping-pong pair, and A is where the chain ENDS — whatever
    /// composites the halo samples A, so the vertical blur must land there.
    quarter_a_view: wgpu::TextureView,
    quarter_b_view: wgpu::TextureView,
    /// Bind groups, named by the pass that USES them (source texture + the
    /// shared sampler): bright samples the caller's picture, downsample the
    /// half, blur_h quarter A, blur_v quarter B.
    bright_bind_group: wgpu::BindGroup,
    downsample_bind_group: wgpu::BindGroup,
    blur_h_bind_group: wgpu::BindGroup,
    blur_v_bind_group: wgpu::BindGroup,
}

/// The four pipelines [`BloomChain::run`] steps through, in that order.
///
/// Passed in rather than held: they are built per target format, and each
/// caller has its own (the lattice writes an offscreen texture, the roll and
/// the glow the surface egui handed them).
struct BloomPipelines<'a> {
    bright: &'a wgpu::RenderPipeline,
    downsample: &'a wgpu::RenderPipeline,
    blur_h: &'a wgpu::RenderPipeline,
    blur_v: &'a wgpu::RenderPipeline,
}

impl BloomChain {
    /// Build the chain over `source`, at fractions of `screen_size` device
    /// pixels.
    ///
    /// `screen_size` is the picture's size ON SCREEN and not the size of
    /// `source`: the lattice's scene texture is render-scaled and the roll's
    /// note texture is already halved, and in both cases what the halo's width
    /// must be a constant share of is the screen.
    fn new(
        device: &wgpu::Device,
        label: &str,
        format: wgpu::TextureFormat,
        filter_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        source: &wgpu::TextureView,
        screen_size: [u32; 2],
    ) -> Self {
        let tex = |label: String, w: u32, h: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(&label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let (hw, hh) = (screen_size[0].div_ceil(2).max(1), screen_size[1].div_ceil(2).max(1));
        let (qw, qh) = (screen_size[0].div_ceil(4).max(1), screen_size[1].div_ceil(4).max(1));
        let half_view = tex(format!("{label}_bloom_half"), hw, hh);
        let quarter_a_view = tex(format!("{label}_bloom_quarter_a"), qw, qh);
        let quarter_b_view = tex(format!("{label}_bloom_quarter_b"), qw, qh);
        let filter_bg = |label: String, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&label),
                layout: filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        BloomChain {
            bright_bind_group: filter_bg(format!("{label}_bright_bind_group"), source),
            downsample_bind_group: filter_bg(format!("{label}_downsample_bind_group"), &half_view),
            blur_h_bind_group: filter_bg(format!("{label}_blur_h_bind_group"), &quarter_a_view),
            blur_v_bind_group: filter_bg(format!("{label}_blur_v_bind_group"), &quarter_b_view),
            half_view,
            quarter_a_view,
            quarter_b_view,
        }
    }

    /// The four full-screen passes, in the one order they may run in:
    /// bright-pass into half res, downsample to quarter, then a separable blur
    /// ping-ponging quarter A -> B (horizontal) -> A (vertical). Whatever
    /// composites the halo samples quarter A, so the vertical blur MUST be the
    /// step that lands there.
    ///
    /// This is the only place that ordering is written down; the pipelines
    /// themselves are built by each caller.
    fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: BloomPipelines<'_>,
        label: &str,
    ) {
        let steps: [BloomStep; 4] = [
            (pipelines.bright, &self.bright_bind_group, &self.half_view),
            (pipelines.downsample, &self.downsample_bind_group, &self.quarter_a_view),
            (pipelines.blur_h, &self.blur_h_bind_group, &self.quarter_b_view),
            (pipelines.blur_v, &self.blur_v_bind_group, &self.quarter_a_view),
        ];
        for (pipeline, bind_group, target) in steps {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("{label}_bloom_pass")),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

impl Offscreen {
    fn new(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniform_buffer: &wgpu::Buffer,
        size: [u32; 2],
        screen_size: [u32; 2],
    ) -> Self {
        let OffscreenShared { format, composite_layout, filter_layout, sampler } = *shared;
        let tex = |label, w: u32, h: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach_and_sample =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;

        let color = tex("lattice_offscreen_color", size[0], size[1], format, attach_and_sample);
        let nodes = tex("lattice_offscreen_nodes", size[0], size[1], format, attach_and_sample);
        let depth = tex(
            "lattice_offscreen_depth",
            size[0],
            size[1],
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let color_view = color.create_view(&Default::default());
        let nodes_view = nodes.create_view(&Default::default());
        // The nodes-only copy, not the picture: the labels are drawn into the
        // picture and must not reach the bloom.
        let bloom = BloomChain::new(
            device,
            "lattice",
            format,
            filter_layout,
            sampler,
            &nodes_view,
            screen_size,
        );

        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_composite_bind_group"),
            layout: composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bloom.quarter_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        Offscreen {
            bloom,
            composite_bind_group,
            color_view,
            nodes_view,
            depth_view: depth.create_view(&Default::default()),
            size,
            screen_size,
        }
    }
}

/// Build one of the scene pipelines from WGSL source (startup uses the
/// baked-in source; hot-reload rebuilds from disk). Node and edge pipelines
/// share the module, bind group layout, blending, and topology; only entry
/// points and vertex layout differ.
///
/// `offscreen` is true for the production pipelines, which draw into the
/// offscreen pass and must declare what that pass carries: its depth
/// attachment, and its second colour attachment — the nodes-only copy the
/// bloom reads (see [`Offscreen::nodes_view`]). The parity test builds
/// single-attachment depthless variants that draw straight into the egui
/// pass, as its reference.
fn create_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    entry_points: (&str, &str),
    vertex_layout: wgpu::VertexBufferLayout<'_>,
    offscreen: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        ..Default::default()
    });

    // The offscreen pass's second attachment takes the same fragment under the
    // same blending: it is the same picture, drawn again without the labels.
    let color_target = wgpu::ColorTargetState {
        format: target_format,
        // Shader outputs premultiplied alpha.
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    let targets: &[Option<wgpu::ColorTargetState>] = if offscreen {
        &[Some(color_target.clone()), Some(color_target)]
    } else {
        &[Some(color_target)]
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        // Name the pipeline after its vertex entry point, so a GPU capture
        // can tell the node and edge passes apart.
        label: Some(entry_points.0),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(entry_points.0),
            compilation_options: Default::default(),
            buffers: &[vertex_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_points.1),
            compilation_options: Default::default(),
            targets,
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        // The depth buffer is written for future depth-reading effects but
        // never rejects a fragment (`Always`): translucent glows composite
        // by draw order, exactly as they did directly in the egui pass.
        depth_stencil: offscreen.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Build both scene pipelines from one source. `offscreen` picks the
/// two-attachment fragment entry points along with the pass state that goes
/// with them — the pair travels together, since a pipeline whose shader
/// writes one attachment cannot be used in a pass that carries two.
fn create_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    offscreen: bool,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (node, edge) = if offscreen {
        ("fs_main_scene", "fs_edge_scene")
    } else {
        ("fs_main", "fs_edge")
    };
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_main", node),
            GpuInstance::LAYOUT,
            offscreen,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_edge", edge),
            GpuEdge::LAYOUT,
            offscreen,
        ),
    )
}

/// egui's own blend state, verbatim (see egui-wgpu's renderer): premultiplied
/// color, and alpha accumulated so the pass composites the same way over a
/// transparent framebuffer.
///
/// The three callbacks that draw their own geometry take it — the roll's
/// notes, the glyphs of [`crate::text`], and the halo of [`crate::glow`]. On a
/// halo that is what makes it pure LIGHT: it carries zero alpha, so the color
/// term adds and the alpha term leaves the destination's own alone.
///
/// One definition rather than one per callback, because them agreeing is what
/// makes them composite identically — the roll's notes over the spectrogram,
/// the spiral's halo over its disc — where copies agree only until one is
/// edited.
///
/// The lattice's `fs_composite` is the one thing here that does NOT name it,
/// and deliberately: it spells the same operator as
/// `PREMULTIPLIED_ALPHA_BLENDING`, which is `a(1-b)+b` where this is
/// `a+b(1-a)` — the same arithmetic written from the other side, as
/// [`crate::text::create_text_pipeline`]'s own doc sets out. Pointing it here
/// would rename a difference that is real in the source and absent in every
/// pixel.
const EGUI_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// One post-process pipeline over the blit.wgsl module: a fullscreen quad
/// with the given fragment entry point. The composite (into the egui
/// pass) blends premultiplied; the bloom-chain passes overwrite their
/// whole target and pass `blend: None`.
fn create_post_pipeline(
    device: &wgpu::Device,
    entry_point: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    // Not named for the lattice: the roll builds its own post pipelines out of
    // the same source, so a validation error carrying the lattice's name would
    // send a reader to the wrong picture.
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl LatticeResources {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let (pipeline, edge_pipeline) =
            create_pipelines(device, SHADER_SRC, target_format, &bind_group_layout, true);

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_filter_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_composite_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let composite_pipeline = create_post_pipeline(
            device,
            "fs_composite",
            target_format,
            &composite_layout,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let filter = |entry| create_post_pipeline(device, entry, target_format, &filter_layout, None);
        let bright_pipeline = filter("fs_bright");
        let downsample_pipeline = filter("fs_blit");
        let blur_h_pipeline = filter("fs_blur_h");
        let blur_v_pipeline = filter("fs_blur_v");
        // Linear filtering: identity when render scale is 1 (texel-aligned
        // sampling), smooth resampling at any other scale.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lattice_composite_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // The label pipelines draw into the scene pass, so they are built
        // against its depth attachment as well as its format.
        let glyph_layout = text::glyph_bind_group_layout(device);
        let glyph_pipeline = |fragment| {
            text::create_text_pipeline(
                device,
                target_format,
                &glyph_layout,
                fragment,
                Some(DEPTH_FORMAT),
            )
        };
        let glyph_rim_pipeline = glyph_pipeline("fs_rim");
        let glyph_fill_pipeline = glyph_pipeline("fs_fill");

        LatticeResources {
            pipeline,
            edge_pipeline,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            bind_group_layout,
            composite_layout,
            filter_layout,
            sampler,
            glyph_rim_pipeline,
            glyph_fill_pipeline,
            glyph_layout,
            glyph_sampler: text::glyph_sampler(device),
            atlas: text::MirroredAtlas::default(),
            marks: text::MirroredAtlas::default(),
            blank: text::blank_atlas(device, queue),
            target_format,
            panes: HashMap::new(),
            timer: GpuTimer::new(device, queue),
            #[cfg(feature = "hot-reload")]
            watcher: ShaderWatcher::new(),
        }
    }

    /// Upload whichever of the two label sheets has moved.
    ///
    /// The text callback answers the same question with a great deal more
    /// (`text::TextResources::mirror_atlas`, which carries every pane already
    /// prepared this frame onto the new texture), and the difference is not an
    /// omission — it is where the two record their draws. That callback draws
    /// in `paint`, after every `prepare` in the frame, so a pane's bind group
    /// and uniforms have to still be right once some LATER pane has grown a
    /// sheet under it. A lattice pane draws in its OWN `prepare`, into its own
    /// offscreen: by the time a later pane uploads anything, this one's pass is
    /// encoded, holding the bind group it was recorded with.
    ///
    /// Which makes the carry-over not merely unnecessary here but wrong. The
    /// pass is encoded, not submitted — egui-wgpu runs the shared encoder after
    /// every prepare — and a `write_buffer` is ordered ahead of that encoder,
    /// so rewriting a prepared pane's atlas size would reach a pass that is
    /// still going to sample the texture it was recorded against. Old texture,
    /// new size, which is exactly the mismatch the text callback's version
    /// exists to prevent, arriving by the other road.
    fn mirror_sheets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: Option<&FontAtlas>,
        marks: Option<&FontAtlas>,
    ) {
        if let Some(atlas) = atlas.filter(|a| !self.atlas.holds(a)) {
            self.atlas.upload(device, queue, atlas);
        }
        if let Some(marks) = marks.filter(|a| !self.marks.holds(a)) {
            self.marks.upload(device, queue, marks);
        }
    }

    /// The two sheets' sizes, as the glyph uniforms carry them.
    ///
    /// A sheet that has never been uploaded reports the 1x1 blank standing in
    /// for it rather than its own zero, because the shader DIVIDES by this.
    /// See `text::TextResources::atlas_sizes`, which says what a zero costs.
    fn sheet_sizes(&self) -> [f32; 4] {
        let (a, m) = (self.atlas.size(), self.marks.size());
        [a[0], a[1], m[0], m[1]].map(|n| n.max(1) as f32)
    }

    /// Fetch (or create) a pane's GPU objects, and when `offscreen_size` is
    /// given, make sure its offscreen target exists at exactly that pixel
    /// size (pane resizes and render-scale changes recreate it).
    /// `screen_size` is the pane's native (unscaled) pixel size, which
    /// sizes the bloom chain.
    fn pane_buffers(
        &mut self,
        device: &wgpu::Device,
        pane_id: u64,
        offscreen_size: Option<[u32; 2]>,
        screen_size: [u32; 2],
    ) -> &mut PaneBuffers {
        let layout = &self.bind_group_layout;
        // Taken before the pane is borrowed: the view is a fresh handle onto
        // whatever the mirror holds right now, which is this frame's atlas —
        // `prepare` uploads before it gets here.
        let (glyph_layout, glyph_sampler) = (&self.glyph_layout, &self.glyph_sampler);
        let atlas_view = self.atlas.view();
        let mark_view = self.marks.view_or(&self.blank);
        let sheet_keys = (self.atlas.key(), self.marks.key());
        let shared = OffscreenShared {
            format: self.target_format,
            composite_layout: &self.composite_layout,
            filter_layout: &self.filter_layout,
            sampler: &self.sampler,
        };
        let pane = self.panes.entry(pane_id).or_insert_with(|| {
            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_bind_group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
            PaneBuffers {
                uniform_buffer,
                bind_group,
                instance_buffer: create_vertex_buffer::<GpuInstance>(
                    device,
                    "lattice_instances",
                    INITIAL_INSTANCE_CAPACITY,
                ),
                instance_capacity: INITIAL_INSTANCE_CAPACITY,
                instance_count: 0,
                edge_buffer: create_vertex_buffer::<GpuEdge>(
                    device,
                    "lattice_edges",
                    INITIAL_EDGE_CAPACITY,
                ),
                edge_capacity: INITIAL_EDGE_CAPACITY,
                edge_count: 0,
                grid_at: 0,
                glyph_buffer: create_vertex_buffer::<GlyphInstance>(
                    device,
                    "lattice_glyphs",
                    INITIAL_GLYPH_CAPACITY,
                ),
                glyph_capacity: INITIAL_GLYPH_CAPACITY,
                glyph_count: 0,
                seams: Vec::new(),
                glyph_uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("lattice_glyph_uniforms"),
                    size: std::mem::size_of::<text::TextUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                glyph_bind_group: None,
                glyph_sheet_keys: (u64::MAX, u64::MAX),
                offscreen: None,
            }
        });
        // A bind group names one texture per sheet, and a sheet that GREW is a
        // new one — so a pane that prepared against the old texture has to be
        // handed the new one before it draws again. Rebuilt on the keys rather
        // than on the sizes, which is the cheap way to be right about the
        // same-size re-upload too: nothing is stale there, and one bind group
        // per publication is nothing.
        if let Some(view) = &atlas_view {
            if pane.glyph_bind_group.is_none() || pane.glyph_sheet_keys != sheet_keys {
                pane.glyph_bind_group = Some(text::bind_group(
                    device,
                    glyph_layout,
                    glyph_sampler,
                    view,
                    &mark_view,
                    &pane.glyph_uniform_buffer,
                ));
                pane.glyph_sheet_keys = sheet_keys;
            }
        }
        if let Some(size) = offscreen_size {
            if pane
                .offscreen
                .as_ref()
                .is_none_or(|o| o.size != size || o.screen_size != screen_size)
            {
                pane.offscreen =
                    Some(Offscreen::new(device, &shared, &pane.uniform_buffer, size, screen_size));
            }
        }
        pane
    }
}

/// Starting element counts for a pane's per-instance and per-edge buffers;
/// both grow by `next_power_of_two` when a frame overflows them.
const INITIAL_INSTANCE_CAPACITY: usize = 256;
const INITIAL_EDGE_CAPACITY: usize = 64;
/// And for its labels. Only sounding, hovered and remembered nodes are named,
/// so a lattice's glyph count is a fraction of a text pane's.
const INITIAL_GLYPH_CAPACITY: usize = 512;

/// A `capacity`-element vertex buffer (VERTEX | COPY_DST) sized for `T`.
/// Used for both the instance and edge buffers, which differ only in label
/// and element type.
fn create_vertex_buffer<T>(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * std::mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl CallbackTrait for LatticeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Lazily (re)create shared resources. Recreate if the target format
        // changed (it can't today, but this keeps the invariant explicit).
        let recreate = callback_resources
            .get::<LatticeResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(LatticeResources::new(device, queue, self.target_format));
        }
        let resources: &mut LatticeResources = callback_resources
            .get_mut()
            .expect("inserted above when missing");

        // Advance the GPU timer's readback cycle first: a result that landed
        // is published now, and the cycle returns to Idle so this frame can be
        // the next one sampled.
        //
        // ONLY the callback carrying a stats sink touches the timer — see
        // `drives_timer`.
        let prepare_start = std::time::Instant::now();
        let poll_start = std::time::Instant::now();
        match (resources.timer.as_mut(), &self.stats) {
            (Some(timer), Some(out)) => {
                if let Some(ms) = timer.poll(device) {
                    out.gpu_ms.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
                }
            }
            // No timer at all: the device refused the feature. Say so, rather
            // than leaving the readout on the same "nothing yet" it shows
            // while a first measurement is still in flight — those are very
            // different answers and the overlay could not tell them apart.
            (None, Some(out)) => {
                out.gpu_ms.store(GPU_TIME_UNSUPPORTED, std::sync::atomic::Ordering::Relaxed);
            }
            (_, None) => {}
        }
        let poll_ms = poll_start.elapsed().as_secs_f32() * 1000.0;

        // Dev builds: pick up edits to the .wgsl on disk. A broken edit is
        // rejected with a message; the previous pipeline keeps rendering.
        #[cfg(feature = "hot-reload")]
        if let Some(source) = resources.watcher.poll() {
            match validate_wgsl(&source) {
                Ok(()) => {
                    let (pipeline, edge_pipeline) = create_pipelines(
                        device,
                        &source,
                        resources.target_format,
                        &resources.bind_group_layout,
                        true,
                    );
                    resources.pipeline = pipeline;
                    resources.edge_pipeline = edge_pipeline;
                    eprintln!("[harmonigraph-render] shader hot-reloaded");
                }
                Err(err) => {
                    eprintln!("[harmonigraph-render] shader reload REJECTED, keeping old pipeline:\n{err}");
                }
            }
        }

        // The labels' sheets, before anything is encoded: the glyphs are drawn
        // inside the scene pass below, so the textures they read have to be the
        // current ones by the time that pass is recorded.
        let write_start = std::time::Instant::now();
        resources.mirror_sheets(device, queue, self.atlas.as_ref(), self.marks.as_ref());
        // Nothing has ever been uploaded: the first frames of a session arrive
        // before any pane has drawn a glyph, and the labels wait for the frame
        // that brings one. Gated on the FONT atlas alone, which is not a
        // shortcut: a mark is always drawn beside a letter, so a frame with a
        // mark in it is a frame with type in it.
        let has_atlas = !resources.atlas.is_empty();
        let sheet_sizes = resources.sheet_sizes();

        // Offscreen pixel size: the callback rect at native resolution,
        // scaled by the render-scale view setting (clamped in from_scene).
        // The unscaled screen size drives the bloom chain.
        let max_dim = device.limits().max_texture_dimension_2d;
        let px_size = |scale: f32| {
            let px = screen_descriptor.pixels_per_point * scale;
            [
                ((self.size_points[0] * px).round() as u32).clamp(1, max_dim),
                ((self.size_points[1] * px).round() as u32).clamp(1, max_dim),
            ]
        };
        let size = px_size(self.render_scale);
        let screen_size = px_size(1.0);
        // Nothing to draw (matches paint()'s early-out): skip the offscreen
        // target and pass entirely. The EDGES count as much as the nodes —
        // `from_scene` drops nodes that can paint nothing, and an idle node
        // paints nothing, so a still lattice is exactly a frame of grid and
        // no instances — and keying this on the instances alone would take the
        // grid down with them. So do the LABELS, for the same reason from the
        // other end: a hovered idle node paints nothing and is named, so a
        // lattice can be a frame of one label and nothing else.
        let anything =
            !self.instances.is_empty() || !self.edges.is_empty() || !self.glyphs.is_empty();
        let offscreen_size = anything.then_some(size);

        let pane = resources.pane_buffers(device, self.pane_id, offscreen_size, screen_size);

        if self.instances.len() > pane.instance_capacity {
            pane.instance_capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer = create_vertex_buffer::<GpuInstance>(
                device,
                "lattice_instances",
                pane.instance_capacity,
            );
        }
        pane.instance_count = self.instances.len() as u32;
        pane.grid_at = self.grid_at.min(pane.instance_count);
        if !self.instances.is_empty() {
            queue.write_buffer(
                &pane.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }

        if self.edges.len() > pane.edge_capacity {
            pane.edge_capacity = self.edges.len().next_power_of_two();
            pane.edge_buffer =
                create_vertex_buffer::<GpuEdge>(device, "lattice_edges", pane.edge_capacity);
        }
        pane.edge_count = self.edges.len() as u32;
        if !self.edges.is_empty() {
            queue.write_buffer(&pane.edge_buffer, 0, bytemuck::cast_slice(&self.edges));
        }

        // The labels. With no atlas there is nothing to sample, so the pass
        // draws none of them rather than sampling a texture that isn't there.
        if has_atlas {
            // The glyphs are a VERTEX buffer, so growing it leaves the bind
            // group — uniforms, atlas, sampler — naming everything it named
            // before. Dropping it here would blank this pane's labels for the
            // frame the buffer grows on, since nothing rebuilds one until the
            // next `pane_buffers`.
            if self.glyphs.len() > pane.glyph_capacity {
                pane.glyph_capacity = self.glyphs.len().next_power_of_two();
                pane.glyph_buffer = create_vertex_buffer::<GlyphInstance>(
                    device,
                    "lattice_glyphs",
                    pane.glyph_capacity,
                );
            }
            pane.glyph_count = self.glyphs.len() as u32;
            pane.seams.clear();
            pane.seams.extend_from_slice(&self.seams);
            if !self.glyphs.is_empty() {
                queue.write_buffer(&pane.glyph_buffer, 0, bytemuck::cast_slice(&self.glyphs));
                // The glyphs' own points, not the screen's: the rects arrive
                // in this pane's space because the pass they are drawn in is
                // this pane's. `pixels_per_point` stays the DEVICE's — it is
                // what the atlas was rasterized at, and so what turns the
                // rim's radius in points into a texel offset, whatever the
                // render scale does to the target's pixels.
                queue.write_buffer(
                    &pane.glyph_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&text::TextUniforms {
                        screen_points: self.size_points,
                        atlas_size: [sheet_sizes[0], sheet_sizes[1]],
                        mark_atlas_size: [sheet_sizes[2], sheet_sizes[3]],
                        filter_axis: self.slide.unit(),
                        pixels_per_point: screen_descriptor.pixels_per_point.max(f32::EPSILON),
                        _pad: [0.0; 3],
                        ring0: text::TextUniforms::ring(self.rings[0]),
                        ring1: text::TextUniforms::ring(self.rings[1]),
                    }),
                );
            }
        } else {
            pane.glyph_count = 0;
            pane.seams.clear();
        }

        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
        let write_ms = write_start.elapsed().as_secs_f32() * 1000.0;

        let scene_start = std::time::Instant::now();
        // The scene pass: draw into the pane's offscreen target, on the
        // encoder egui-wgpu executes before its own render pass. paint()
        // then just composites the finished texture.
        let pane = resources
            .panes
            .get(&self.pane_id)
            .expect("created by pane_buffers above");
        let draws = pane.instance_count > 0 || pane.edge_count > 0 || pane.glyph_count > 0;
        if let Some(offscreen) = pane.offscreen.as_ref().filter(|_| draws) {
            // Bracket the scene pass and the bloom chain together: what the
            // overlay wants is the cost of drawing THE LATTICE, which is both.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing =
                self.drives_timer() && resources.timer.as_ref().is_some_and(GpuTimer::arming);
            let opening = if timing {
                resources.timer.as_ref().and_then(GpuTimer::opening)
            } else {
                None
            };
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_scene_pass"),
                // The picture, then the same picture without the labels (see
                // `Offscreen::nodes_view`). Both clear to transparent black:
                // premultiplied "nothing", so compositing over the pane
                // background reproduces drawing straight into the egui pass.
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &offscreen.color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &offscreen.nodes_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &offscreen.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: opening,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // The grid sits at the home sheet's own depth, so it goes
            // between the sheets behind it and the home sheet itself —
            // NOT under everything. Under everything, a node on a sheet
            // behind the home one punches its clearing through the home
            // grid, which puts a hole in the layer it is supposed to be
            // hidden by. `grid_at` is where that seam falls; the home
            // sheet's own clearings are the tail of the first run, ahead of
            // the grid, so they can hide the sheets behind without eating
            // the grid they sit on.
            let nodes = |pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>| {
                if range.is_empty() {
                    return;
                }
                pass.set_pipeline(&resources.pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, range);
            };
            let grid = |pass: &mut wgpu::RenderPass<'_>| {
                if pane.edge_count == 0 {
                    return;
                }
                pass.set_pipeline(&resources.edge_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
                pass.draw(0..4, 0..pane.edge_count);
            };
            // One label, at its own place in the order: every rim, then every
            // fill. Stamping had that order for free and the shader keeps it,
            // because two neighbouring letters otherwise darken each other's
            // ink where their rims overlap. Per LABEL rather than over the
            // whole frame, which is what interleaving costs and all it costs:
            // two names on different nodes overlapping is two labels at
            // different depths, and the nearer one is meant to sit on the
            // other.
            let label = |pass: &mut wgpu::RenderPass<'_>, seam: &GlyphSeam| {
                let Some(bind_group) = pane.glyph_bind_group.as_ref() else {
                    return;
                };
                let range = seam.start..seam.start + seam.count;
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                pass.set_pipeline(&resources.glyph_rim_pipeline);
                pass.draw(0..4, range.clone());
                pass.set_pipeline(&resources.glyph_fill_pipeline);
                pass.draw(0..4, range);
            };

            // The nodes, in order, with each label spliced in right after the
            // node it names — so what covers a name is exactly what covers
            // its node. The seams are sorted, so this walks forward once.
            //
            // Which side of the grid a label goes is the run its node was in,
            // not how its seam compares to `grid_at`: the runs MEET at that
            // number, so the last far node to ship and a home node culled
            // before any home node ships share it while belonging on
            // opposite sides. The grid covers a name if and only if it is
            // drawn after it, which is the same rule everything else in this
            // pass follows.
            let mut cursor = 0u32;
            let mut grid_drawn = false;
            for seam in &pane.seams {
                if !grid_drawn && seam.after_grid {
                    nodes(&mut pass, cursor..pane.grid_at);
                    grid(&mut pass);
                    (cursor, grid_drawn) = (pane.grid_at, true);
                }
                nodes(&mut pass, cursor..seam.at);
                cursor = seam.at;
                label(&mut pass, seam);
            }
            if !grid_drawn {
                nodes(&mut pass, cursor..pane.grid_at);
                grid(&mut pass);
                cursor = pane.grid_at;
            }
            nodes(&mut pass, cursor..pane.instance_count);
            drop(pass);

            // Skipped entirely at strength 0: the composite multiplies the
            // never-written quarter texture by 0, and fresh wgpu textures
            // read as zero anyway.
            if self.uniforms.misc2[3] > 0.0 {
                offscreen.bloom.run(egui_encoder, Self::bloom_pipelines(resources), "lattice");
            }

            if timing {
                if let Some(timer) = resources.timer.as_mut() {
                    timer.close(egui_encoder);
                }
            }
        } else if let Some(out) = &self.stats {
            // No pass was encoded, so no reading can land and the timer's
            // cycle does not turn over. Say "nothing measured" rather than
            // leaving the last real figure sitting there: `poll` returns None
            // from Idle forever, and the overlay would keep re-averaging a
            // number from whenever the lattice last drew.
            //
            // The distinction is why GPU_TIME_PENDING exists at all — a frozen
            // reading and a live one are the same bits otherwise, and a pane
            // that encodes no pass can sit here indefinitely rather than for a
            // frame. A silent lattice already ships no NODE, every idle one
            // being culled; what it takes to ship no edge either is a window
            // holding no adjacent pair, which the extent bars do not reach
            // (they stop at 1). So the state is currently out of a user's
            // reach and stays guarded on purpose: it is one lattice-sizing
            // change away, and the symptom would be a stale figure rather
            // than a crash — the kind nobody reports.
            out.gpu_ms.store(GPU_TIME_PENDING, std::sync::atomic::Ordering::Relaxed);
        }

        let scene_ms = scene_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(stats) = &self.stats {
            use std::sync::atomic::Ordering::Relaxed;
            let prepare_ms = prepare_start.elapsed().as_secs_f32() * 1000.0;
            stats.prepare_ms.store(prepare_ms.to_bits(), Relaxed);
            stats.poll_ms.store(poll_ms.to_bits(), Relaxed);
            stats.write_ms.store(write_ms.to_bits(), Relaxed);
            stats.scene_ms.store(scene_ms.to_bits(), Relaxed);
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<LatticeResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        // Nothing was rendered into the offscreen target. The edges and the
        // labels count as much as the nodes here — see `prepare`, where the
        // same test decides whether the target exists at all.
        if pane.instance_count == 0 && pane.edge_count == 0 && pane.glyph_count == 0 {
            return;
        }
        let Some(offscreen) = &pane.offscreen else {
            return;
        };

        // The scene was rendered in prepare(); stretch it over the
        // viewport (egui-wgpu sets the viewport to the callback rect).
        render_pass.set_pipeline(&resources.composite_pipeline);
        render_pass.set_bind_group(0, &offscreen.composite_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod gpu_harness;
#[cfg(test)]
mod lattice_tests;
